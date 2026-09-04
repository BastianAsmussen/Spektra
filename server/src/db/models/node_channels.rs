use chrono::NaiveDateTime;
use diesel::prelude::*;

/// A channel a node is assigned to monitor.
#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = crate::db::schema::node_channels)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NodeChannel {
    pub id: i64,
    pub node_id: i64,
    pub channel_id: i64,
    pub bandwidth_hz: Option<i32>,
    pub created_at: NaiveDateTime,
}

/// Values needed to assign a channel to a node.
#[derive(Debug, Insertable)]
#[diesel(table_name = crate::db::schema::node_channels)]
pub struct NewNodeChannel {
    pub node_id: i64,
    pub channel_id: i64,
    pub bandwidth_hz: Option<i32>,
}
