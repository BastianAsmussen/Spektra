use chrono::NaiveDateTime;
use diesel::prelude::*;

/// A per-node bearer credential row.
#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = crate::db::schema::node_credentials)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NodeCredential {
    pub id: i64,
    pub node_id: i64,
    pub token: String,
    pub created_at: NaiveDateTime,
    pub expires_at: Option<NaiveDateTime>,
    pub revoked_at: Option<NaiveDateTime>,
}

/// Values needed to issue a credential.
#[derive(Debug, Insertable)]
#[diesel(table_name = crate::db::schema::node_credentials)]
pub struct NewNodeCredential {
    pub node_id: i64,
    pub token: String,
}
