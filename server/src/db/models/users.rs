use chrono::NaiveDateTime;
use diesel::prelude::*;

/// A user account row.
#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = crate::db::schema::users)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct User {
    pub id: i64,
    pub email: String,
    pub password_hash: String,
    pub full_name: String,
    pub role_id: i64,
    pub deactivated: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Values needed to create a user.
#[derive(Debug, Insertable)]
#[diesel(table_name = crate::db::schema::users)]
pub struct NewUser {
    pub email: String,
    pub password_hash: String,
    pub full_name: String,
    pub role_id: i64,
}
