use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::Serialize;

use super::enums::AlarmState;

/// One transition in an alarm's lifecycle.
#[derive(Debug, Queryable, Selectable, Serialize, utoipa::ToSchema)]
#[diesel(table_name = crate::db::schema::alarm_events)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct AlarmEvent {
    pub id: i64,
    pub alarm_id: i64,
    /// Absent for the event that raised the alarm.
    pub from_state: Option<AlarmState>,
    pub to_state: AlarmState,
    /// Absent when the detector made the transition rather than a person.
    pub changed_by_user_id: Option<i64>,
    pub reason: String,
    pub created_at: NaiveDateTime,
}

/// Values persisted for one transition.
#[derive(Debug, Insertable)]
#[diesel(table_name = crate::db::schema::alarm_events)]
pub struct NewAlarmEvent {
    pub alarm_id: i64,
    pub from_state: Option<AlarmState>,
    pub to_state: AlarmState,
    pub changed_by_user_id: Option<i64>,
    pub reason: String,
}
