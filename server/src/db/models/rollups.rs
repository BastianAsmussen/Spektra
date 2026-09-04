use chrono::NaiveDateTime;
use diesel::prelude::*;

use super::enums::{Metric, RollupResolution};

/// One metric summarized over one bucket.
#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = crate::db::schema::rollups)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Rollup {
    pub id: i64,
    pub node_id: i64,
    pub channel_id: i64,
    pub metric: Metric,
    pub resolution: RollupResolution,
    pub bucket_start: NaiveDateTime,
    pub bucket_end: NaiveDateTime,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub median: f64,
    pub stddev: f64,
    pub sample_count: i64,
    pub created_at: NaiveDateTime,
}

/// Values persisted for one summarized bucket.
#[derive(Debug, Insertable)]
#[diesel(table_name = crate::db::schema::rollups)]
pub struct NewRollup {
    pub node_id: i64,
    pub channel_id: i64,
    pub metric: Metric,
    pub resolution: RollupResolution,
    pub bucket_start: NaiveDateTime,
    pub bucket_end: NaiveDateTime,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub median: f64,
    pub stddev: f64,
    pub sample_count: i64,
}
