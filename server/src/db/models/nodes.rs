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
    pub external_identity: Option<String>,
    pub name: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub hardware: Value,
    pub capabilities: Value,
    pub suspended: bool,
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
