use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::Serialize;

use super::enums::{AlarmState, Metric};

/// One raised deviation, at whatever point of its lifecycle it has reached.
#[derive(Debug, Queryable, Selectable, Serialize, utoipa::ToSchema)]
#[diesel(table_name = crate::db::schema::alarms)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Alarm {
    pub id: i64,
    pub node_id: i64,
    /// Absent for whole-node alarms such as silence.
    pub channel_id: Option<i64>,
    /// Absent for whole-node alarms such as silence.
    pub metric: Option<Metric>,
    pub state: AlarmState,
    /// Detector payload: bucket, window, observed value, baseline, threshold.
    #[schema(value_type = Object)]
    pub explanation: serde_json::Value,
    pub raised_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub closed_at: Option<NaiveDateTime>,
}

/// Values persisted when an alarm is raised.
#[derive(Debug, Insertable)]
#[diesel(table_name = crate::db::schema::alarms)]
pub struct NewAlarm {
    pub node_id: i64,
    pub channel_id: Option<i64>,
    pub metric: Option<Metric>,
    pub state: AlarmState,
    pub explanation: serde_json::Value,
}
