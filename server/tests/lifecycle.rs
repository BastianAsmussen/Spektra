#![expect(
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    reason = "test harness helpers are not `#[test]` functions, so clippy.toml's in-tests allowances do not reach them"
)]

mod common;

use common::{Pool, test_pool};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::{Router, response::Response};
use chrono::{Duration, Utc};
use diesel::prelude::*;
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use server::api::{alarms, work_orders};
use server::db::models::enums::{AlarmState, Metric};
use server::db::models::nodes::NewNode;
use server::db::models::sessions::NewSession;
use server::db::models::users::NewUser;
use server::db::schema::{
    alarms as alarms_schema, nodes as nodes_schema, sessions as sessions_schema,
    users as users_schema,
};
use server::state::AppState;
use tower::ServiceExt as _;

const ADMINISTRATOR: i64 = 1;
const OPERATOR: i64 = 2;
const TECHNICIAN: i64 = 3;
const READER: i64 = 4;

fn app(state: AppState) -> Router {
    Router::new()
        .merge(alarms::routes())
        .merge(work_orders::routes())
        .with_state(state)
}

async fn seed_actor(pool: &Pool, email: &str, role_id: i64) -> (i64, String) {
    let conn = pool.get().await.expect("seed connection");
    let new_user = NewUser {
        email: email.to_owned(),
        password_hash: "test-hash".to_owned(),
        full_name: "Test User".to_owned(),
        role_id,
    };
    let user_id: i64 = conn
        .interact(move |conn| {
            diesel::insert_into(users_schema::table)
                .values(&new_user)
                .returning(users_schema::id)
                .get_result(conn)
        })
        .await
        .expect("seed interact failed")
        .expect("seed user failed");

    let token = format!("token-{email}");
    let session = NewSession {
        token: token.clone(),
        user_id,
        expires_at: (Utc::now() + Duration::hours(1)).naive_utc(),
    };
    conn.interact(move |conn| {
        diesel::insert_into(sessions_schema::table)
            .values(&session)
            .execute(conn)
    })
    .await
    .expect("seed interact failed")
    .expect("seed session failed");

    (user_id, token)
}

async fn seed_alarm(pool: &Pool) -> (i64, i64) {
    let conn = pool.get().await.expect("seed connection");

    conn.interact(|conn| {
        let node_id: i64 = diesel::insert_into(nodes_schema::table)
            .values(&NewNode {
                external_identity: Some("lifecycle-node".to_owned()),
                name: "Lifecycle node".to_owned(),
                latitude: Some(57.05),
                longitude: Some(9.92),
                hardware: json!({"device": "test"}),
                capabilities: json!({"metrics": []}),
            })
            .returning(nodes_schema::id)
            .get_result(conn)
            .expect("node insert");

        let alarm_id: i64 = diesel::insert_into(alarms_schema::table)
            .values((
                alarms_schema::node_id.eq(node_id),
                alarms_schema::metric.eq(Some(Metric::SignalToNoise)),
                alarms_schema::state.eq(AlarmState::Open),
                alarms_schema::explanation.eq(json!({"kind": "seeded"})),
            ))
            .returning(alarms_schema::id)
            .get_result(conn)
            .expect("alarm insert");

        (node_id, alarm_id)
    })
    .await
    .expect("seed interact failed")
}

async fn get(state: &AppState, token: &str, uri: &str) -> Response {
    app(state.clone())
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("cookie", format!("session_token={token}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response")
}

async fn post(state: &AppState, token: &str, uri: &str, body: Value) -> Response {
    app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("cookie", format!("session_token={token}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("response")
}

async fn body_json(response: Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();

    serde_json::from_slice(&bytes).expect("json")
}

async fn alarm_state(pool: &Pool, alarm_id: i64) -> AlarmState {
    let conn = pool.get().await.expect("connection");

    conn.interact(move |conn| {
        alarms_schema::table
            .filter(alarms_schema::id.eq(alarm_id))
            .select(alarms_schema::state)
            .first(conn)
    })
    .await
    .expect("interact")
    .expect("state")
}

#[tokio::test]
async fn an_alarm_walks_its_lifecycle_and_records_every_step() {
    let pool = test_pool("lifecycle", "walk").await;
    let state = AppState::new(pool.clone());
    let (_, token) = seed_actor(&pool, "operator@example.org", OPERATOR).await;
    let (_, alarm_id) = seed_alarm(&pool).await;

    for (to, reason) in [
        ("acknowledged", "taking a look"),
        ("under_verification", "sending someone"),
        ("closed", "feeder replaced"),
    ] {
        let response = post(
            &state,
            &token,
            &format!("/api/alarms/{alarm_id}/transition"),
            json!({"to": to, "reason": reason}),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK, "moving to {to}");
    }

    assert_eq!(alarm_state(&pool, alarm_id).await, AlarmState::Closed);

    let detail = body_json(get(&state, &token, &format!("/api/alarms/{alarm_id}")).await).await;
    let events = detail["events"].as_array().expect("events");

    assert_eq!(events.len(), 3, "one event per move: {events:?}");
    assert_eq!(events[0]["from_state"], "open");
    assert_eq!(events[0]["to_state"], "acknowledged");
    assert_eq!(events[0]["reason"], "taking a look");
    assert_eq!(events[2]["to_state"], "closed");
    assert!(detail["closed_at"].is_string(), "a closed alarm is stamped");
}

#[tokio::test]
async fn a_step_cannot_be_skipped() {
    let pool = test_pool("lifecycle", "skip").await;
    let state = AppState::new(pool.clone());
    let (_, token) = seed_actor(&pool, "operator@example.org", OPERATOR).await;
    let (_, alarm_id) = seed_alarm(&pool).await;

    let response = post(
        &state,
        &token,
        &format!("/api/alarms/{alarm_id}/transition"),
        json!({"to": "under_verification", "reason": "skipping ahead"}),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(alarm_state(&pool, alarm_id).await, AlarmState::Open);
}

#[tokio::test]
async fn a_closed_alarm_stays_closed() {
    let pool = test_pool("lifecycle", "final").await;
    let state = AppState::new(pool.clone());
    let (_, token) = seed_actor(&pool, "operator@example.org", OPERATOR).await;
    let (_, alarm_id) = seed_alarm(&pool).await;

    post(
        &state,
        &token,
        &format!("/api/alarms/{alarm_id}/transition"),
        json!({"to": "closed", "reason": "a passing weather front"}),
    )
    .await;

    let response = post(
        &state,
        &token,
        &format!("/api/alarms/{alarm_id}/transition"),
        json!({"to": "acknowledged", "reason": "reopening"}),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(alarm_state(&pool, alarm_id).await, AlarmState::Closed);
}

#[tokio::test]
async fn a_transition_needs_a_reason() {
    let pool = test_pool("lifecycle", "reason").await;
    let state = AppState::new(pool.clone());
    let (_, token) = seed_actor(&pool, "operator@example.org", OPERATOR).await;
    let (_, alarm_id) = seed_alarm(&pool).await;

    let response = post(
        &state,
        &token,
        &format!("/api/alarms/{alarm_id}/transition"),
        json!({"to": "acknowledged", "reason": "   "}),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn a_reader_may_look_but_not_touch() {
    let pool = test_pool("lifecycle", "reader").await;
    let state = AppState::new(pool.clone());
    let (_, token) = seed_actor(&pool, "reader@example.org", READER).await;
    let (_, alarm_id) = seed_alarm(&pool).await;

    assert_eq!(
        get(&state, &token, "/api/alarms").await.status(),
        StatusCode::OK
    );

    let response = post(
        &state,
        &token,
        &format!("/api/alarms/{alarm_id}/transition"),
        json!({"to": "acknowledged", "reason": "curious"}),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn an_unauthenticated_request_is_rejected() {
    let pool = test_pool("lifecycle", "unauthenticated").await;
    let state = AppState::new(pool.clone());
    seed_alarm(&pool).await;

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
}

#[tokio::test]
async fn the_listing_filters_by_state() {
    let pool = test_pool("lifecycle", "filter").await;
    let state = AppState::new(pool.clone());
    let (_, token) = seed_actor(&pool, "operator@example.org", OPERATOR).await;
    let (_, alarm_id) = seed_alarm(&pool).await;

    let open = body_json(get(&state, &token, "/api/alarms?state=open").await).await;
    assert_eq!(open.as_array().map(Vec::len), Some(1));

    let closed = body_json(get(&state, &token, "/api/alarms?state=closed").await).await;
    assert_eq!(closed.as_array().map(Vec::len), Some(0));

    post(
        &state,
        &token,
        &format!("/api/alarms/{alarm_id}/transition"),
        json!({"to": "closed", "reason": "done"}),
    )
    .await;

    let closed = body_json(get(&state, &token, "/api/alarms?state=closed").await).await;
    assert_eq!(closed.as_array().map(Vec::len), Some(1));
}

#[tokio::test]
async fn a_dispatch_moves_the_alarm_under_verification() {
    let pool = test_pool("lifecycle", "dispatch").await;
    let state = AppState::new(pool.clone());
    let (_, operator) = seed_actor(&pool, "operator@example.org", OPERATOR).await;
    let (technician_id, _) = seed_actor(&pool, "tech@example.org", TECHNICIAN).await;
    let (_, alarm_id) = seed_alarm(&pool).await;

    let response = post(
        &state,
        &operator,
        &format!("/api/alarms/{alarm_id}/dispatch"),
        json!({"technician_user_id": technician_id, "station_name": "Hadsund"}),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);

    let order = body_json(response).await;
    assert_eq!(order["alarm_id"], alarm_id);
    assert_eq!(order["station_name"], "Hadsund");
    assert_eq!(order["status"], "assigned");
    assert!(order["fault_present"].is_null(), "nobody has been yet");

    assert_eq!(
        alarm_state(&pool, alarm_id).await,
        AlarmState::UnderVerification
    );
}

#[tokio::test]
async fn a_technician_may_not_dispatch() {
    let pool = test_pool("lifecycle", "no_self_dispatch").await;
    let state = AppState::new(pool.clone());
    let (technician_id, technician) = seed_actor(&pool, "tech@example.org", TECHNICIAN).await;
    let (_, alarm_id) = seed_alarm(&pool).await;

    let response = post(
        &state,
        &technician,
        &format!("/api/alarms/{alarm_id}/dispatch"),
        json!({"technician_user_id": technician_id, "station_name": "Hadsund"}),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_dispatch_needs_a_station() {
    let pool = test_pool("lifecycle", "no_station").await;
    let state = AppState::new(pool.clone());
    let (_, operator) = seed_actor(&pool, "operator@example.org", OPERATOR).await;
    let (technician_id, _) = seed_actor(&pool, "tech@example.org", TECHNICIAN).await;
    let (_, alarm_id) = seed_alarm(&pool).await;

    let response = post(
        &state,
        &operator,
        &format!("/api/alarms/{alarm_id}/dispatch"),
        json!({"technician_user_id": technician_id, "station_name": "  "}),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn a_field_report_closes_the_alarm_it_came_from() {
    let pool = test_pool("lifecycle", "round_trip").await;
    let state = AppState::new(pool.clone());
    let (_, operator) = seed_actor(&pool, "operator@example.org", OPERATOR).await;
    let (technician_id, technician) = seed_actor(&pool, "tech@example.org", TECHNICIAN).await;
    let (_, alarm_id) = seed_alarm(&pool).await;

    let order = body_json(
        post(
            &state,
            &operator,
            &format!("/api/alarms/{alarm_id}/dispatch"),
            json!({"technician_user_id": technician_id, "station_name": "Hadsund"}),
        )
        .await,
    )
    .await;
    let order_id = order["id"].as_i64().expect("an order id");

    let response = post(
        &state,
        &technician,
        &format!("/api/work-orders/{order_id}/complete"),
        json!({
            "fault_present": true,
            "cause": "water in the feeder",
            "action_taken": "replaced the connector"
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);

    let result = body_json(response).await;
    assert_eq!(result["alarm_state"], "closed");
    assert_eq!(result["work_order"]["status"], "completed");
    assert_eq!(result["work_order"]["fault_present"], true);
    assert!(result["work_order"]["completed_at"].is_string());

    assert_eq!(alarm_state(&pool, alarm_id).await, AlarmState::Closed);

    let detail = body_json(get(&state, &operator, &format!("/api/alarms/{alarm_id}")).await).await;
    let events = detail["events"].as_array().expect("events");
    let last = events.last().expect("a last event");

    assert_eq!(last["to_state"], "closed");
    assert_eq!(
        last["reason"],
        "fault confirmed on site: water in the feeder; replaced the connector"
    );
}

#[tokio::test]
async fn a_clear_visit_is_recorded_as_one() {
    let pool = test_pool("lifecycle", "no_fault").await;
    let state = AppState::new(pool.clone());
    let (_, operator) = seed_actor(&pool, "operator@example.org", OPERATOR).await;
    let (technician_id, technician) = seed_actor(&pool, "tech@example.org", TECHNICIAN).await;
    let (_, alarm_id) = seed_alarm(&pool).await;

    let order = body_json(
        post(
            &state,
            &operator,
            &format!("/api/alarms/{alarm_id}/dispatch"),
            json!({"technician_user_id": technician_id, "station_name": "Hadsund"}),
        )
        .await,
    )
    .await;
    let order_id = order["id"].as_i64().expect("an order id");

    let result = body_json(
        post(
            &state,
            &technician,
            &format!("/api/work-orders/{order_id}/complete"),
            json!({"fault_present": false, "cause": null, "action_taken": null}),
        )
        .await,
    )
    .await;

    assert_eq!(result["work_order"]["fault_present"], false);
    assert_eq!(result["alarm_state"], "closed");
}

#[tokio::test]
async fn an_order_cannot_be_completed_twice() {
    let pool = test_pool("lifecycle", "twice").await;
    let state = AppState::new(pool.clone());
    let (_, operator) = seed_actor(&pool, "operator@example.org", OPERATOR).await;
    let (technician_id, technician) = seed_actor(&pool, "tech@example.org", TECHNICIAN).await;
    let (_, alarm_id) = seed_alarm(&pool).await;

    let order = body_json(
        post(
            &state,
            &operator,
            &format!("/api/alarms/{alarm_id}/dispatch"),
            json!({"technician_user_id": technician_id, "station_name": "Hadsund"}),
        )
        .await,
    )
    .await;
    let order_id = order["id"].as_i64().expect("an order id");
    let report = json!({"fault_present": true, "cause": null, "action_taken": null});

    let first = post(
        &state,
        &technician,
        &format!("/api/work-orders/{order_id}/complete"),
        report.clone(),
    )
    .await;
    let second = post(
        &state,
        &technician,
        &format!("/api/work-orders/{order_id}/complete"),
        report,
    )
    .await;

    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn a_technician_only_sees_their_own_orders() {
    let pool = test_pool("lifecycle", "own_orders").await;
    let state = AppState::new(pool.clone());
    let (_, operator) = seed_actor(&pool, "operator@example.org", OPERATOR).await;
    let (mine_id, mine) = seed_actor(&pool, "mine@example.org", TECHNICIAN).await;
    let (theirs_id, theirs) = seed_actor(&pool, "theirs@example.org", TECHNICIAN).await;
    let (_, alarm_id) = seed_alarm(&pool).await;

    for technician in [mine_id, theirs_id] {
        post(
            &state,
            &operator,
            &format!("/api/alarms/{alarm_id}/dispatch"),
            json!({"technician_user_id": technician, "station_name": "Hadsund"}),
        )
        .await;
    }

    let ours = body_json(get(&state, &mine, "/api/work-orders").await).await;
    let others = body_json(get(&state, &theirs, "/api/work-orders").await).await;
    let all = body_json(get(&state, &operator, "/api/work-orders").await).await;

    assert_eq!(ours.as_array().map(Vec::len), Some(1));
    assert_eq!(others.as_array().map(Vec::len), Some(1));
    assert_eq!(
        all.as_array().map(Vec::len),
        Some(2),
        "the operator sees both"
    );
    assert_eq!(ours[0]["technician_user_id"], mine_id);
}

#[tokio::test]
async fn a_technician_may_not_report_on_someone_elses_order() {
    let pool = test_pool("lifecycle", "foreign_order").await;
    let state = AppState::new(pool.clone());
    let (_, operator) = seed_actor(&pool, "operator@example.org", OPERATOR).await;
    let (mine_id, _) = seed_actor(&pool, "mine@example.org", TECHNICIAN).await;
    let (_, theirs) = seed_actor(&pool, "theirs@example.org", TECHNICIAN).await;
    let (_, alarm_id) = seed_alarm(&pool).await;

    let order = body_json(
        post(
            &state,
            &operator,
            &format!("/api/alarms/{alarm_id}/dispatch"),
            json!({"technician_user_id": mine_id, "station_name": "Hadsund"}),
        )
        .await,
    )
    .await;
    let order_id = order["id"].as_i64().expect("an order id");

    let response = post(
        &state,
        &theirs,
        &format!("/api/work-orders/{order_id}/complete"),
        json!({"fault_present": true, "cause": null, "action_taken": null}),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_technician_only_sees_alarms_they_were_sent_to() {
    let pool = test_pool("lifecycle", "alarm_visibility").await;
    let state = AppState::new(pool.clone());
    let (_, operator) = seed_actor(&pool, "operator@example.org", OPERATOR).await;
    let (technician_id, technician) = seed_actor(&pool, "tech@example.org", TECHNICIAN).await;
    let (_, alarm_id) = seed_alarm(&pool).await;

    let before = body_json(get(&state, &technician, "/api/alarms").await).await;
    assert_eq!(before.as_array().map(Vec::len), Some(0));
    assert_eq!(
        get(&state, &technician, &format!("/api/alarms/{alarm_id}"))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );

    post(
        &state,
        &operator,
        &format!("/api/alarms/{alarm_id}/dispatch"),
        json!({"technician_user_id": technician_id, "station_name": "Hadsund"}),
    )
    .await;

    let after = body_json(get(&state, &technician, "/api/alarms").await).await;
    assert_eq!(after.as_array().map(Vec::len), Some(1));
}

#[tokio::test]
async fn an_administrator_sees_the_whole_fleet() {
    let pool = test_pool("lifecycle", "administrator").await;
    let state = AppState::new(pool.clone());
    let (_, admin) = seed_actor(&pool, "admin@example.org", ADMINISTRATOR).await;
    let (_, alarm_id) = seed_alarm(&pool).await;

    let listing = body_json(get(&state, &admin, "/api/alarms").await).await;
    assert_eq!(listing.as_array().map(Vec::len), Some(1));

    assert_eq!(
        get(&state, &admin, &format!("/api/alarms/{alarm_id}"))
            .await
            .status(),
        StatusCode::OK
    );
}
