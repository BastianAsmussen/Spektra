use std::sync::Arc;

use axum::extract::FromRef;
use chrono::NaiveDateTime;
use tokio::sync::broadcast;

use crate::db::models::enums::{AlarmState, Metric};
use crate::live;
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
    pub ingest_pool: deadpool_diesel::postgres::Pool,
    pub node_events: broadcast::Sender<NodeEvent>,
    pub alarm_events: broadcast::Sender<AlarmEvent>,
    pub live_samples: broadcast::Sender<live::Sample>,
    pub live: Arc<live::Sessions>,
    pub metrics: Arc<Metrics>,
}

impl AppState {
    /// Build state from a database pool, creating fresh broadcast channels.
    #[must_use]
    pub fn new(pool: deadpool_diesel::postgres::Pool) -> Self {
        let (node_events, _) = broadcast::channel(CHANNEL_CAPACITY);
        let (alarm_events, _) = broadcast::channel(CHANNEL_CAPACITY);
        let (live_samples, _) = broadcast::channel(CHANNEL_CAPACITY);

        Self {
            ingest_pool: pool.clone(),
            pool,
            node_events,
            alarm_events,
            live_samples,
            live: Arc::new(live::Sessions::new()),
            metrics: Metrics::new(),
        }
    }

    /// Give ingest and the maintenance jobs a pool of their own.
    #[must_use]
    pub fn with_ingest_pool(mut self, pool: deadpool_diesel::postgres::Pool) -> Self {
        self.ingest_pool = pool;
        self
    }

    /// Publish a node event, ignoring the case where nobody is listening.
    pub fn publish_node(&self, event: NodeEvent) {
        drop(self.node_events.send(event));
    }

    /// Publish an alarm event, ignoring the case where nobody is listening.
    pub fn publish_alarm(&self, event: AlarmEvent) {
        drop(self.alarm_events.send(event));
    }

    /// Publish one live dwell, ignoring the case where nobody is listening.
    pub fn publish_live(&self, sample: live::Sample) {
        drop(self.live_samples.send(sample));
    }
}

impl FromRef<AppState> for deadpool_diesel::postgres::Pool {
    fn from_ref(state: &AppState) -> Self {
        state.pool.clone()
    }
}
