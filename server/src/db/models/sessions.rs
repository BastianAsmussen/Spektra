use chrono::NaiveDateTime;
use diesel::prelude::*;

/// A session row as read from the database.
#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = crate::db::schema::sessions)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Session {
    pub id: i64,
    pub token: String,
    pub user_id: i64,
    pub created_at: NaiveDateTime,
    pub expires_at: NaiveDateTime,
    pub last_used_at: Option<NaiveDateTime>,
}

/// Values needed to create a session.
#[derive(Debug, Insertable)]
#[diesel(table_name = crate::db::schema::sessions)]
pub struct NewSession {
    pub token: String,
    pub user_id: i64,
    pub expires_at: NaiveDateTime,
}
