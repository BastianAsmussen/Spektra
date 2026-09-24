use chrono::NaiveDateTime;
use diesel::prelude::*;

/// One node self-report of its operational state.
#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = crate::db::schema::node_health)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NodeHealth {
    pub id: i64,
    pub node_id: i64,
    pub measured_at: NaiveDateTime,
    pub uptime_seconds: Option<f64>,
    pub load_1m: Option<f64>,
    pub load_5m: Option<f64>,
    pub load_15m: Option<f64>,
    pub cpu_temperature_celsius: Option<f64>,
    pub clock_offset_seconds: Option<f64>,
    pub created_at: NaiveDateTime,
}

/// Values persisted for one accepted health report.
#[derive(Debug, Insertable)]
#[diesel(table_name = crate::db::schema::node_health)]
pub struct NewNodeHealth {
    pub node_id: i64,
    pub measured_at: NaiveDateTime,
    pub uptime_seconds: Option<f64>,
    pub load_1m: Option<f64>,
    pub load_5m: Option<f64>,
    pub load_15m: Option<f64>,
    pub cpu_temperature_celsius: Option<f64>,
    pub clock_offset_seconds: Option<f64>,
}
