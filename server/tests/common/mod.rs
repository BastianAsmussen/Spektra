#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "a test harness is not a `#[test]` function, so clippy.toml's in-tests allowances do not reach it"
)]
#![allow(dead_code)]

use std::time::Duration;

use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/");

pub type Pool = deadpool_diesel::postgres::Pool;

const CREATE_ATTEMPTS: usize = 10;

const RETRY_DELAY: Duration = Duration::from_millis(250);

/// The cluster the per-test databases are created in.
pub fn test_database_url() -> String {
    std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        "postgres://spektra_test:test_password@localhost:5432/spektra_test".to_owned()
    })
}

/// A migrated, isolated database named after `prefix` and `test_name`.
pub async fn test_pool(prefix: &str, test_name: &str) -> Pool {
    let base_url = test_database_url();
    let db_name = format!("spektra_{prefix}_{test_name}");

    create_database(&base_url, &db_name).await;

    let test_url = base_url.rsplit_once('/').map_or_else(
        || format!("{base_url}/{db_name}"),
        |(prefix, _)| format!("{prefix}/{db_name}"),
    );

    let manager =
        deadpool_diesel::postgres::Manager::new(test_url, deadpool_diesel::Runtime::Tokio1);
    let pool = deadpool_diesel::postgres::Pool::builder(manager)
        .build()
        .expect("failed to build test pool");

    let conn = pool.get().await.expect("migration connection");
    conn.interact(|conn| {
        conn.run_pending_migrations(MIGRATIONS)
            .expect("migrations failed");
    })
    .await
    .expect("migration interact failed");

    pool
}

async fn create_database(base_url: &str, db_name: &str) {
    let manager =
        deadpool_diesel::postgres::Manager::new(base_url, deadpool_diesel::Runtime::Tokio1);
    let setup_pool = deadpool_diesel::postgres::Pool::builder(manager)
        .build()
        .expect("failed to build setup pool");

    let mut last: Option<String> = None;
    for attempt in 1..=CREATE_ATTEMPTS {
        let conn = setup_pool.get().await.expect("setup connection");
        let name = db_name.to_owned();

        let outcome = conn
            .interact(move |conn| {
                use diesel::connection::SimpleConnection;

                drop(conn.batch_execute(&format!(
                    "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
                     WHERE datname = '{name}';"
                )));
                drop(conn.batch_execute(&format!("DROP DATABASE IF EXISTS \"{name}\";")));

                conn.batch_execute(&format!("CREATE DATABASE \"{name}\";"))
            })
            .await
            .expect("setup interact failed");

        match outcome {
            Ok(()) => return,
            Err(err) => {
                last = Some(err.to_string());

                if attempt < CREATE_ATTEMPTS {
                    tokio::time::sleep(RETRY_DELAY).await;
                }
            }
        }
    }

    panic!(
        "could not create {db_name} in {CREATE_ATTEMPTS} attempts: {}",
        last.unwrap_or_else(|| "no error recorded".to_owned())
    );
}

/// Insert a user and return its id.
pub async fn seed_user(
    pool: &Pool,
    email: &str,
    full_name: &str,
    role_id: i64,
    password_hash: &str,
) -> i64 {
    use diesel::prelude::*;
    use server::db::models::users::NewUser;
    use server::db::schema::users as users_schema;

    let conn = pool.get().await.expect("seed connection");
    let new_user = NewUser {
        email: email.to_owned(),
        password_hash: password_hash.to_owned(),
        full_name: full_name.to_owned(),
        role_id,
    };

    conn.interact(move |conn| {
        diesel::insert_into(users_schema::table)
            .values(&new_user)
            .returning(users_schema::id)
            .get_result(conn)
    })
    .await
    .expect("seed interact failed")
    .expect("seed user failed")
}

/// Open a session for `user_id`, valid for an hour.
pub async fn seed_session(pool: &Pool, user_id: i64, token: &str) {
    use chrono::{Duration, Utc};
    use diesel::prelude::*;
    use server::db::models::sessions::NewSession;
    use server::db::schema::sessions as sessions_schema;

    let conn = pool.get().await.expect("seed connection");
    let new_session = NewSession {
        token: token.to_owned(),
        user_id,
        expires_at: Utc::now()
            .naive_utc()
            .checked_add_signed(Duration::hours(1))
            .expect("an hour from now is representable"),
    };

    conn.interact(move |conn| {
        diesel::insert_into(sessions_schema::table)
            .values(&new_session)
            .execute(conn)
    })
    .await
    .expect("seed interact failed")
    .expect("seed session failed");
}
