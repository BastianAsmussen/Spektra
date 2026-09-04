use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::Serialize;

use super::enums::WorkOrderStatus;

/// One dispatch: an alarm, a named technician and a station to visit.
#[derive(Debug, Queryable, Selectable, Serialize, utoipa::ToSchema)]
#[diesel(table_name = crate::db::schema::work_orders)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct WorkOrder {
    pub id: i64,
    pub alarm_id: i64,
    pub technician_user_id: i64,
    pub station_name: String,
    pub status: WorkOrderStatus,
    /// Whether the technician found the fault still present. `None` until completed.
    pub fault_present: Option<bool>,
    pub cause: Option<String>,
    pub action_taken: Option<String>,
    pub completed_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
}

/// Values persisted when an alarm is dispatched.
#[derive(Debug, Insertable)]
#[diesel(table_name = crate::db::schema::work_orders)]
pub struct NewWorkOrder {
    pub alarm_id: i64,
    pub technician_user_id: i64,
    pub station_name: String,
}
