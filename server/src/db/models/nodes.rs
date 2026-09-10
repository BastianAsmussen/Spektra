use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::Serialize;
use serde_json::Value;
use utoipa::ToSchema;

/// A registered receiver node.
#[derive(Debug, Queryable, Selectable, Serialize, ToSchema)]
#[diesel(table_name = crate::db::schema::nodes)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Node {
    pub id: i64,
    /// Absent until the node enrols.
    pub external_identity: Option<String>,
    pub name: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub hardware: Value,
    pub capabilities: Value,
    pub suspended: bool,
    /// Seconds between deliveries the server asks this node for.
    pub report_interval_seconds: i32,
    pub owner_id: Option<i64>,
    pub created_at: NaiveDateTime,
    pub last_seen_at: Option<NaiveDateTime>,
}

/// Values needed to register a node.
#[derive(Debug, Insertable)]
#[diesel(table_name = crate::db::schema::nodes)]
pub struct NewNode {
    pub external_identity: Option<String>,
    pub name: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub hardware: Value,
    pub capabilities: Value,
}

/// A node an administrator has planned but that has not enrolled yet.
#[derive(Debug, Insertable)]
#[diesel(table_name = crate::db::schema::nodes)]
pub struct PlannedNode {
    pub name: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub hardware: Value,
    pub capabilities: Value,
    pub owner_id: Option<i64>,
}

/// The fields an administrator may change on a node.
#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::db::schema::nodes)]
pub struct NodeChanges {
    pub name: Option<String>,
    pub latitude: Option<Option<f64>>,
    pub longitude: Option<Option<f64>>,
    pub owner_id: Option<Option<i64>>,
    pub report_interval_seconds: Option<i32>,
}
