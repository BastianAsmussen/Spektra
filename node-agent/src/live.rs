use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use protocol::v1::{LiveReading, LiveSample};
use protocol::{MAX_LIVE_SESSION, live_session};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::client::{Client, ClientError};
use crate::report::{ChannelIdentity, MetricSample};
use protocol::PROTOCOL_VERSION;

const QUEUE: usize = 4;
const REDIAL: Duration = Duration::from_secs(5);

struct Dwell {
    channel: ChannelIdentity,
    samples: Vec<MetricSample>,
    measured_at: SystemTime,
}

/// The handle the sampling loop offers dwells to.
#[derive(Clone)]
pub struct Live {
    dwells: mpsc::Sender<Dwell>,
    running: Arc<AtomicBool>,
}

impl Live {
    /// Whether a session is running and dwells are worth offering.
    #[must_use]
    pub fn wants(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Offer one dwell, dropping it if the sender is behind.
    pub fn offer(&self, channel: &ChannelIdentity, samples: &[MetricSample]) {
        if !self.wants() {
            return;
        }

        let dwell = Dwell {
            channel: channel.clone(),
            samples: samples.to_vec(),
            measured_at: SystemTime::now(),
        };

        if self.dwells.try_send(dwell).is_err() {
            tracing::debug!("a live dwell was dropped; the sender is behind");
        }
    }
}

/// Build the handle and the task that serves it.
pub fn spawn(client: Client) -> (Live, impl Future<Output = ()> + Send) {
    let (dwells, receiver) = mpsc::channel(QUEUE);
    let running = Arc::new(AtomicBool::new(false));
    let live = Live {
        dwells,
        running: Arc::clone(&running),
    };

    (live, run(client, receiver, running))
}

struct Session {
    id: u64,
    ends: Instant,
    interval: Duration,
    sent: HashMap<u64, Instant>,
}

impl Session {
    fn due(&mut self, frequency_hz: u64, now: Instant) -> bool {
        if self
            .sent
            .get(&frequency_hz)
            .is_some_and(|last| now.duration_since(*last) < self.interval)
        {
            return false;
        }

        self.sent.insert(frequency_hz, now);

        true
    }
}

async fn run(mut client: Client, mut dwells: mpsc::Receiver<Dwell>, running: Arc<AtomicBool>) {
    let mut session: Option<Session> = None;

    loop {
        let Ok(mut commands) = client.watch_live().await.inspect_err(|err| {
            tracing::debug!(error = %err, "the live command stream is not open");
        }) else {
            idle(&running, &mut session, &mut dwells).await;
            continue;
        };

        loop {
            let ends = session
                .as_ref()
                .map_or_else(|| deadline(MAX_LIVE_SESSION), |it| it.ends);

            tokio::select! {
                command = commands.message() => match command {
                    Ok(Some(command)) => {
                        session = open(&command);
                        running.store(session.is_some(), Ordering::Relaxed);
                    }
                    Ok(None) | Err(_) => break,
                },

                dwell = dwells.recv() => match dwell {
                    Some(dwell) => {
                        if !deliver(&mut client, session.as_mut(), dwell).await {
                            session = None;
                            running.store(false, Ordering::Relaxed);
                        }
                    }
                    None => return,
                },

                () = tokio::time::sleep_until(ends), if session.is_some() => {
                    tracing::debug!("a live session ran out");
                    session = None;
                    running.store(false, Ordering::Relaxed);
                }
            }
        }

        idle(&running, &mut session, &mut dwells).await;
    }
}

async fn idle(
    running: &AtomicBool,
    session: &mut Option<Session>,
    dwells: &mut mpsc::Receiver<Dwell>,
) {
    running.store(false, Ordering::Relaxed);
    *session = None;
    while dwells.try_recv().is_ok() {}

    tokio::time::sleep(REDIAL).await;
}

fn deadline(after: Duration) -> Instant {
    Instant::now()
        .checked_add(after)
        .unwrap_or_else(Instant::now)
}

fn open(command: &protocol::v1::LiveCommand) -> Option<Session> {
    let (remaining, interval) = live_session(command)?;

    tracing::info!(
        session = command.session_id,
        seconds = remaining.as_secs(),
        interval_ms = interval.as_millis(),
        "a live session started"
    );

    Some(Session {
        id: command.session_id,
        ends: deadline(remaining),
        interval,
        sent: HashMap::new(),
    })
}

async fn deliver(client: &mut Client, session: Option<&mut Session>, dwell: Dwell) -> bool {
    let Some(session) = session else {
        return false;
    };
    if !session.due(dwell.channel.frequency_hz, Instant::now()) {
        return true;
    }

    let sample = LiveSample {
        protocol_version: PROTOCOL_VERSION.to_owned(),
        session_id: session.id,
        measured_at: Some(dwell.measured_at.into()),
        frequency_hz: dwell.channel.frequency_hz,
        modulation: i32::from(dwell.channel.modulation),
        label: dwell.channel.label,
        readings: dwell
            .samples
            .iter()
            .map(|sample| LiveReading {
                metric: i32::from(sample.metric),
                value: sample.value,
            })
            .collect(),
    };

    match client.submit_live(sample).await {
        Ok(ack) => ack.session_id == session.id,
        Err(ClientError::Rpc(status)) => {
            tracing::debug!(error = %status, "a live sample was refused");

            true
        }
        Err(err) => {
            tracing::warn!(error = %err, "a live sample could not be sent");

            false
        }
    }
}
