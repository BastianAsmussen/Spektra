use diesel::prelude::*;

use super::enums::Modulation;

/// A monitored channel, identified by frequency and modulation.
#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = crate::db::schema::channels)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Channel {
    pub id: i64,
    pub name: String,
    pub frequency_hz: i64,
    pub modulation: Modulation,
    pub created_at: chrono::NaiveDateTime,
}

/// Values needed to register a channel the first time a node reports it.
#[derive(Debug, Insertable)]
#[diesel(table_name = crate::db::schema::channels)]
pub struct NewChannel {
    pub name: String,
    pub frequency_hz: i64,
    pub modulation: Modulation,
}
