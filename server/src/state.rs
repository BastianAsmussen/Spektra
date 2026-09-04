use std::sync::Arc;

use axum::extract::FromRef;
use chrono::NaiveDateTime;
use tokio::sync::broadcast;

use crate::db::models::enums::{AlarmState, Metric};
use crate::ops::Metrics;

const CHANNEL_CAPACITY: usize = 1024;

/// Something changed about a node that an open dashboard should see.
#[derive(Debug, Clone, Copy)]
pub enum NodeEvent {
    Reported { node_id: i64, at: NaiveDateTime },
    Silent { node_id: i64, since: NaiveDateTime },
    SuspensionChanged { node_id: i64, suspended: bool },
}

impl NodeEvent {
    #[must_use]
    pub const fn node_id(&self) -> i64 {
        match *self {
            Self::Reported { node_id, .. }
            | Self::Silent { node_id, .. }
            | Self::SuspensionChanged { node_id, .. } => node_id,
        }
    }
}

/// Something changed about an alarm that an open dashboard should see.
#[derive(Debug, Clone, Copy)]
pub enum AlarmEvent {
    Raised {
        alarm_id: i64,
        node_id: i64,
        metric: Option<Metric>,
    },
    StateChanged {
        alarm_id: i64,
        node_id: i64,
        from: AlarmState,
        to: AlarmState,
    },
}

impl AlarmEvent {
    #[must_use]
    pub const fn node_id(&self) -> i64 {
        match *self {
            Self::Raised { node_id, .. } | Self::StateChanged { node_id, .. } => node_id,
        }
    }

    #[must_use]
    pub const fn alarm_id(&self) -> i64 {
        match *self {
            Self::Raised { alarm_id, .. } | Self::StateChanged { alarm_id, .. } => alarm_id,
        }
    }
}

/// Shared application state threaded through every handler.
#[derive(Clone)]
pub struct AppState {
    pub pool: deadpool_diesel::postgres::Pool,
    pub node_events: broadcast::Sender<NodeEvent>,
    pub alarm_events: broadcast::Sender<AlarmEvent>,
    pub metrics: Arc<Metrics>,
}

impl AppState {
    /// Build state from a database pool, creating fresh broadcast channels.
    #[must_use]
    pub fn new(pool: deadpool_diesel::postgres::Pool) -> Self {
        let (node_events, _) = broadcast::channel(CHANNEL_CAPACITY);
        let (alarm_events, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            pool,
            node_events,
            alarm_events,
            metrics: Metrics::new(),
        }
    }

    /// Publish a node event, ignoring the case where nobody is listening.
    pub fn publish_node(&self, event: NodeEvent) {
        drop(self.node_events.send(event));
    }

    /// Publish an alarm event, ignoring the case where nobody is listening.
    pub fn publish_alarm(&self, event: AlarmEvent) {
        drop(self.alarm_events.send(event));
    }
}

impl FromRef<AppState> for deadpool_diesel::postgres::Pool {
    fn from_ref(state: &AppState) -> Self {
        state.pool.clone()
    }
}
