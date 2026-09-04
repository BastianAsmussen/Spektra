use chrono::NaiveDateTime;
use diesel::prelude::*;

use super::enums::Metric;

/// One metric summary over one aggregation window.
#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = crate::db::schema::measurements)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Measurement {
    pub id: i64,
    pub node_id: i64,
    pub channel_id: i64,
    pub metric: Metric,
    pub window_start: NaiveDateTime,
    pub window_end: NaiveDateTime,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub median: f64,
    pub stddev: f64,
    pub p95: f64,
    pub sample_count: i64,
    pub created_at: NaiveDateTime,
}

/// Values persisted for one accepted metric reading.
#[derive(Debug, Insertable)]
#[diesel(table_name = crate::db::schema::measurements)]
pub struct NewMeasurement {
    pub node_id: i64,
    pub channel_id: i64,
    pub metric: Metric,
    pub window_start: NaiveDateTime,
    pub window_end: NaiveDateTime,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub median: f64,
    pub stddev: f64,
    pub p95: f64,
    pub sample_count: i64,
}
