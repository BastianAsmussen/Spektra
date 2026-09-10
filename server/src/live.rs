use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, TimeDelta, Utc};
use protocol::v1::LiveCommand;
use tokio::sync::mpsc;

use crate::db::models::enums::Metric;

/// How long a session runs before the open panel has to renew it.
pub const SESSION_TTL: TimeDelta = TimeDelta::seconds(45);

/// How often the server asks an inspected node to sample, in milliseconds.
pub const SAMPLE_INTERVAL_MS: u32 = 1_000;

const COMMAND_QUEUE: usize = 3;

/// One dwell from an inspected node, on its way to open dashboards.
#[derive(Debug, Clone)]
pub struct Sample {
    pub node_id: i64,
    pub label: String,
    pub frequency_hz: u64,
    pub measured_at: DateTime<Utc>,
    pub readings: Vec<(Metric, f64)>,
}

#[derive(Default)]
struct NodeLive {
    commands: Option<mpsc::Sender<LiveCommand>>,
    session: Option<Session>,
}

#[derive(Clone, Copy)]
struct Session {
    id: u64,
    expires_at: DateTime<Utc>,
}

/// Whether a node could be reached when a session was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    Told,
    Absent,
}

/// Every node's live channel.
pub struct Sessions {
    nodes: Mutex<HashMap<i64, NodeLive>>,
    next_id: AtomicU64,
}

impl Default for Sessions {
    fn default() -> Self {
        Self::new()
    }
}

impl Sessions {
    #[must_use]
    pub fn new() -> Self {
        Self {
            nodes: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Register a node's command stream and hand back the receiving end.
    pub fn watch(&self, node_id: i64) -> mpsc::Receiver<LiveCommand> {
        let (tx, rx) = mpsc::channel(COMMAND_QUEUE);

        let Ok(mut nodes) = self.nodes.lock() else {
            return rx;
        };

        let entry = nodes.entry(node_id).or_default();
        entry.commands = Some(tx);

        if let Some(session) = live(entry.session, Utc::now()) {
            send(entry, command(session));
        }

        rx
    }

    /// Start a session on this node, or push an existing one's expiry out.
    pub fn open(&self, node_id: i64) -> Reach {
        let now = Utc::now();

        let Ok(mut nodes) = self.nodes.lock() else {
            return Reach::Absent;
        };

        let entry = nodes.entry(node_id).or_default();
        let id = live(entry.session, now)
            .map_or_else(|| self.next_id.fetch_add(1, Ordering::Relaxed), |it| it.id);
        let session = Session {
            id,
            expires_at: now.checked_add_signed(SESSION_TTL).unwrap_or(now),
        };

        entry.session = Some(session);

        if entry.commands.is_none() {
            return Reach::Absent;
        }

        send(entry, command(session));

        Reach::Told
    }

    /// End a session on this node.
    pub fn close(&self, node_id: i64) {
        let Ok(mut nodes) = self.nodes.lock() else {
            return;
        };
        let Some(entry) = nodes.get_mut(&node_id) else {
            return;
        };

        entry.session = None;
        send(entry, stop());
    }

    /// Whether this node is running the session a sample claims.
    #[must_use]
    pub fn accepts(&self, node_id: i64, session_id: u64) -> bool {
        let Ok(nodes) = self.nodes.lock() else {
            return false;
        };

        nodes
            .get(&node_id)
            .and_then(|entry| live(entry.session, Utc::now()))
            .is_some_and(|session| session.id == session_id)
    }

    /// Whether anybody is inspecting this node right now.
    #[must_use]
    pub fn is_open(&self, node_id: i64) -> bool {
        let Ok(nodes) = self.nodes.lock() else {
            return false;
        };

        nodes
            .get(&node_id)
            .and_then(|entry| live(entry.session, Utc::now()))
            .is_some()
    }

    /// Stop every session whose expiry has passed.
    pub fn sweep(&self) {
        let now = Utc::now();

        let Ok(mut nodes) = self.nodes.lock() else {
            return;
        };
        for entry in nodes.values_mut() {
            if entry.commands.as_ref().is_some_and(mpsc::Sender::is_closed) {
                entry.commands = None;
            }

            if entry.session.is_some() && live(entry.session, now).is_none() {
                entry.session = None;
                send(entry, stop());
            }
        }

        nodes.retain(|_, entry| entry.commands.is_some() || entry.session.is_some());
    }
}

const fn live(session: Option<Session>, now: DateTime<Utc>) -> Option<Session> {
    match session {
        Some(session) if session.expires_at.timestamp_millis() > now.timestamp_millis() => {
            Some(session)
        }
        _ => None,
    }
}

fn send(entry: &mut NodeLive, command: LiveCommand) {
    let Some(sender) = entry.commands.as_ref() else {
        return;
    };

    if sender.try_send(command).is_err() {
        entry.commands = None;
    }
}

fn command(session: Session) -> LiveCommand {
    LiveCommand {
        protocol_version: protocol::PROTOCOL_VERSION.to_owned(),
        server_time: Some(std::time::SystemTime::from(Utc::now()).into()),
        session_id: session.id,
        min_interval_ms: SAMPLE_INTERVAL_MS,
        expires_at: Some(std::time::SystemTime::from(session.expires_at).into()),
    }
}

fn stop() -> LiveCommand {
    LiveCommand {
        protocol_version: protocol::PROTOCOL_VERSION.to_owned(),
        server_time: Some(std::time::SystemTime::from(Utc::now()).into()),
        session_id: 0,
        min_interval_ms: 0,
        expires_at: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_node_nobody_dialled_in_for_cannot_be_told() {
        let sessions = Sessions::new();

        assert_eq!(sessions.open(1), Reach::Absent);
        assert!(sessions.is_open(1));
    }

    #[test]
    fn opening_reaches_a_node_that_is_watching() {
        let sessions = Sessions::new();
        let mut commands = sessions.watch(1);

        assert_eq!(sessions.open(1), Reach::Told);

        let command = commands.try_recv().expect("a command");
        assert_ne!(command.session_id, 0);
        assert_eq!(command.min_interval_ms, SAMPLE_INTERVAL_MS);
    }

    #[test]
    fn a_node_that_dials_in_mid_session_is_told_about_it() {
        let sessions = Sessions::new();
        assert_eq!(sessions.open(1), Reach::Absent);

        let mut commands = sessions.watch(1);

        assert_ne!(commands.try_recv().expect("a command").session_id, 0);
    }

    #[test]
    fn renewing_keeps_the_same_session_id() {
        let sessions = Sessions::new();
        let mut commands = sessions.watch(1);

        assert_eq!(sessions.open(1), Reach::Told);
        let first = commands.try_recv().expect("a command").session_id;
        assert_eq!(sessions.open(1), Reach::Told);
        let second = commands.try_recv().expect("a command").session_id;

        assert_eq!(first, second);
    }

    #[test]
    fn only_the_running_session_is_accepted() {
        let sessions = Sessions::new();
        let mut commands = sessions.watch(1);
        assert_eq!(sessions.open(1), Reach::Told);
        let id = commands.try_recv().expect("a command").session_id;

        assert!(sessions.accepts(1, id));
        assert!(!sessions.accepts(1, id.wrapping_add(1)));
        assert!(!sessions.accepts(1, 0));
        assert!(!sessions.accepts(2, id), "another node's id was accepted");
    }

    #[test]
    fn closing_stops_the_node_and_the_session() {
        let sessions = Sessions::new();
        let mut commands = sessions.watch(1);
        assert_eq!(sessions.open(1), Reach::Told);
        let id = commands.try_recv().expect("a command").session_id;

        sessions.close(1);

        assert_eq!(commands.try_recv().expect("a stop").session_id, 0);
        assert!(!sessions.accepts(1, id));
        assert!(!sessions.is_open(1));
    }

    #[test]
    fn a_sweep_drops_an_expired_session() {
        let sessions = Sessions::new();
        let mut commands = sessions.watch(1);
        assert_eq!(sessions.open(1), Reach::Told);
        drop(commands.try_recv().expect("a command"));

        let mut nodes = sessions.nodes.lock().expect("the map");
        let entry = nodes.get_mut(&1).expect("the node");
        let mut session = entry.session.expect("a session");
        session.expires_at = Utc::now() - TimeDelta::seconds(1);
        entry.session = Some(session);
        drop(nodes);

        assert!(!sessions.is_open(1), "an expired session still reads open");
        sessions.sweep();

        assert_eq!(commands.try_recv().expect("a stop").session_id, 0);
    }

    #[test]
    fn a_redial_replaces_the_stream_it_found() {
        let sessions = Sessions::new();
        let first = sessions.watch(1);
        let mut second = sessions.watch(1);

        assert_eq!(sessions.open(1), Reach::Told);
        assert!(second.try_recv().is_ok(), "the redial was not the one told");
        drop(first);
    }

    #[test]
    fn a_sweep_forgets_a_node_whose_stream_went_away() {
        let sessions = Sessions::new();
        drop(sessions.watch(1));

        sessions.sweep();

        assert_eq!(
            sessions.open(1),
            Reach::Absent,
            "a dead stream still counts as reachable"
        );
    }
}
