#![expect(
    clippy::expect_used,
    reason = "test harness helpers are not `#[test]` functions, so clippy.toml's in-tests allowances do not reach them"
)]

mod common;

use common::{Pool, test_pool};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::{Router, response::Response};
use diesel::prelude::*;
use http_body_util::BodyExt as _;
use server::api::{alarms, auth, pages};
use server::db::models::users::NewUser;
use server::db::schema::{sessions as sessions_schema, users as users_schema};
use server::state::AppState;
use tower::ServiceExt as _;

fn app(state: AppState) -> Router {
    Router::new()
        .merge(auth::routes())
        .merge(pages::routes())
        .merge(alarms::routes())
        .with_state(state)
}

async fn seed_user(pool: &Pool, email: &str, password: &str) -> i64 {
    let conn = pool.get().await.expect("seed connection");
    let new_user = NewUser {
        email: email.to_owned(),
        password_hash: auth::hash_password(password).expect("a hash"),
        full_name: "Test User".to_owned(),
        role_id: 1,
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

async fn post_login(state: &AppState, email: &str, password: &str) -> Response {
    let body = format!(
        "email={}&password={}",
        urlencode(email),
        urlencode(password)
    );

    app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("response")
}

fn urlencode(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '@' => "%40".to_owned(),
            '+' => "%2B".to_owned(),
            ' ' => "%20".to_owned(),
            '&' => "%26".to_owned(),
            other => other.to_string(),
        })
        .collect()
}

fn set_cookie(response: &Response) -> Option<String> {
    response
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn token_from(response: &Response) -> Option<String> {
    set_cookie(response).and_then(|header| {
        header
            .split(';')
            .next()
            .and_then(|pair| pair.split_once('='))
            .map(|(_, token)| token.to_owned())
    })
}

async fn sessions_for(pool: &Pool, user_id: i64) -> i64 {
    let conn = pool.get().await.expect("connection");

    conn.interact(move |conn| {
        sessions_schema::table
            .filter(sessions_schema::user_id.eq(user_id))
            .count()
            .get_result(conn)
    })
    .await
    .expect("interact")
    .expect("count")
}

#[tokio::test]
async fn the_right_password_opens_a_session() {
    let pool = test_pool("auth", "login_ok").await;
    let state = AppState::new(pool.clone());
    let user_id = seed_user(&pool, "operator@example.org", "hunter2 hunter2").await;

    let response = post_login(&state, "operator@example.org", "hunter2 hunter2").await;

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response
            .headers()
            .get("location")
            .and_then(|l| l.to_str().ok()),
        Some("/")
    );
    assert_eq!(sessions_for(&pool, user_id).await, 1);
}

#[tokio::test]
async fn the_session_cookie_is_not_reachable_from_script() {
    let pool = test_pool("auth", "cookie_flags").await;
    let state = AppState::new(pool.clone());
    seed_user(&pool, "operator@example.org", "hunter2 hunter2").await;

    let response = post_login(&state, "operator@example.org", "hunter2 hunter2").await;
    let header = set_cookie(&response).expect("a session cookie");

    assert!(header.contains("HttpOnly"), "{header}");
    assert!(header.contains("SameSite=Lax"), "{header}");
    assert!(header.contains("Path=/"), "{header}");
}

#[tokio::test]
async fn a_wrong_password_opens_nothing() {
    let pool = test_pool("auth", "login_wrong").await;
    let state = AppState::new(pool.clone());
    let user_id = seed_user(&pool, "operator@example.org", "hunter2 hunter2").await;

    let response = post_login(&state, "operator@example.org", "hunter3 hunter3").await;

    assert_eq!(response.status(), StatusCode::OK, "the form is re-rendered");
    assert!(
        set_cookie(&response).is_none(),
        "a cookie was issued anyway"
    );
    assert_eq!(sessions_for(&pool, user_id).await, 0);
}

#[tokio::test]
async fn an_unknown_address_is_answered_like_a_wrong_password() {
    let pool = test_pool("auth", "login_unknown").await;
    let state = AppState::new(pool.clone());
    seed_user(&pool, "operator@example.org", "hunter2 hunter2").await;

    let unknown = post_login(&state, "nobody@example.org", "hunter2 hunter2").await;
    let wrong = post_login(&state, "operator@example.org", "wrong").await;

    assert_eq!(unknown.status(), wrong.status());

    let body = |response: Response| async {
        response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes()
    };
    assert_eq!(body(unknown).await, body(wrong).await);
}

#[tokio::test]
async fn an_address_is_matched_without_regard_to_case() {
    let pool = test_pool("auth", "login_case").await;
    let state = AppState::new(pool.clone());
    seed_user(&pool, "operator@example.org", "hunter2 hunter2").await;

    let response = post_login(&state, "Operator@Example.ORG", "hunter2 hunter2").await;

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn a_page_sends_a_signed_out_browser_to_the_form() {
    let pool = test_pool("auth", "page_redirect").await;
    let state = AppState::new(pool);

    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response
            .headers()
            .get("location")
            .and_then(|l| l.to_str().ok()),
        Some("/login")
    );
}

#[tokio::test]
async fn a_dead_cookie_does_not_trap_the_browser_between_the_two_pages() {
    let pool = test_pool("auth", "dead_cookie").await;
    let state = AppState::new(pool);
    let router = app(state);

    let dashboard = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .header("cookie", "session_token=a-token-no-session-row-has")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(dashboard.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        dashboard
            .headers()
            .get("location")
            .and_then(|l| l.to_str().ok()),
        Some("/login")
    );

    let form = router
        .oneshot(
            Request::builder()
                .uri("/login")
                .header("cookie", "session_token=a-token-no-session-row-has")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(
        form.status(),
        StatusCode::OK,
        "the form redirected instead of rendering, which is the loop"
    );

    assert!(
        set_cookie(&form).is_some_and(|value| value.contains("session_token=")),
        "the unusable cookie was left in place"
    );
}

#[tokio::test]
async fn the_api_still_answers_a_signed_out_client_with_json() {
    let pool = test_pool("auth", "api_401").await;
    let state = AppState::new(pool);

    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/api/alarms")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .map(|value| value.starts_with("application/json")),
        Some(true)
    );
}

#[tokio::test]
async fn a_session_reaches_the_dashboard() {
    let pool = test_pool("auth", "page_ok").await;
    let state = AppState::new(pool.clone());
    seed_user(&pool, "operator@example.org", "hunter2 hunter2").await;

    let token = token_from(&post_login(&state, "operator@example.org", "hunter2 hunter2").await)
        .expect("a token");

    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/")
                .header("cookie", format!("session_token={token}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn logging_out_destroys_the_session_row() {
    let pool = test_pool("auth", "logout").await;
    let state = AppState::new(pool.clone());
    let user_id = seed_user(&pool, "operator@example.org", "hunter2 hunter2").await;

    let token = token_from(&post_login(&state, "operator@example.org", "hunter2 hunter2").await)
        .expect("a token");
    assert_eq!(sessions_for(&pool, user_id).await, 1);

    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/logout")
                .header("cookie", format!("session_token={token}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        sessions_for(&pool, user_id).await,
        0,
        "the row outlived the cookie"
    );

    let after = app(state)
        .oneshot(
            Request::builder()
                .uri("/")
                .header("cookie", format!("session_token={token}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(after.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn the_administrator_is_created_once_and_not_again() {
    let pool = test_pool("auth", "bootstrap").await;

    // SAFETY: the test process sets these for its own bootstrap call. Other
    // tests in this file do not read them.
    unsafe {
        std::env::set_var("SPEKTRA_ADMIN_EMAIL", "admin@spektra.test");
        std::env::set_var("SPEKTRA_ADMIN_PASSWORD", "a long enough password");
    }

    assert!(auth::bootstrap_admin(&pool).await.expect("bootstrap"));
    assert!(
        !auth::bootstrap_admin(&pool).await.expect("bootstrap"),
        "a restart created a second administrator"
    );

    let state = AppState::new(pool);
    let response = post_login(&state, "admin@spektra.test", "a long enough password").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    unsafe {
        std::env::remove_var("SPEKTRA_ADMIN_EMAIL");
        std::env::remove_var("SPEKTRA_ADMIN_PASSWORD");
    }
}
