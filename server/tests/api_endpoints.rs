#![expect(
    clippy::expect_used,
    reason = "test harness helpers are not `#[test]` functions, so clippy.toml's in-tests allowances do not reach them"
)]

mod common;

use common::test_pool;

use axum::{Router, body::Body, http::Request};
use chrono::Utc;
use diesel::RunQueryDsl;
use futures_util::StreamExt;
use http_body_util::BodyExt;
use protocol::v1::node_ingest_server::NodeIngest;
use protocol::v1::{
    Capabilities, ChannelMeasurement, Hardware, HealthReport, Location, MeasurementReport, Metric,
    MetricReading, Modulation, NodeRegistrationRequest, SampleStats,
};
use serde_json::json;
use server::api::{health, nodes, pages, ws};
use server::db::models::nodes::NewNode;
use server::db::schema::{
    measurements as measurements_schema, node_credentials as node_credentials_schema,
    node_health as node_health_schema, nodes as nodes_schema,
};
use server::grpc::Ingest;
use server::state::{AppState, NodeEvent};
use tower::ServiceExt;

fn app(state: AppState) -> Router {
    Router::new()
        .merge(health::routes())
        .merge(nodes::routes())
        .merge(pages::routes())
        .merge(ws::routes())
        .with_state(state)
}

async fn serve(state: AppState) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind an ephemeral port");
    let addr = listener.local_addr().expect("no local address");

    drop(tokio::spawn(async move {
        drop(axum::serve(listener, app(state)).await);
    }));

    addr
}

async fn connect_ws(
    addr: std::net::SocketAddr,
    token: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let request = tokio_tungstenite::tungstenite::http::Request::builder()
        .uri(format!("ws://{addr}/api/ws"))
        .header("host", addr.to_string())
        .header("connection", "Upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header(
            "sec-websocket-key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        )
        .header("cookie", format!("session_token={token}"))
        .body(())
        .expect("failed to build the upgrade request");

    let (socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .expect("websocket handshake failed");

    socket
}

async fn seed_user(pool: &deadpool_diesel::postgres::Pool, email: &str) -> i64 {
    common::seed_user(pool, email, "Test User", 1, "test-hash").await
}

async fn seed_user_with_role(
    pool: &deadpool_diesel::postgres::Pool,
    email: &str,
    role_id: i64,
) -> i64 {
    common::seed_user(pool, email, "Test Technician", role_id, "test-hash").await
}

async fn seed_dispatch(pool: &deadpool_diesel::postgres::Pool, technician_id: i64) -> i64 {
    let conn = pool.get().await.expect("seed connection");
    conn.interact(move |conn| {
        use diesel::sql_types::BigInt;
        use diesel::{ExpressionMethods, QueryDsl};

        let node_id: i64 = nodes_schema::table
            .select(nodes_schema::id)
            .order(nodes_schema::id.asc())
            .first(conn)
            .expect("no seeded node to dispatch against");

        diesel::sql_query(
            "INSERT INTO alarms(node_id, state, explanation) \
             VALUES ($1, 'open', '{}'::jsonb)",
        )
        .bind::<BigInt, _>(node_id)
        .execute(conn)
        .expect("failed to raise the alarm");

        diesel::sql_query(
            "INSERT INTO work_orders(alarm_id, technician_user_id, station_name, status) \
             SELECT id, $1, 'Test station', 'assigned' FROM alarms ORDER BY id DESC LIMIT 1",
        )
        .bind::<BigInt, _>(technician_id)
        .execute(conn)
        .expect("failed to dispatch the work order");

        node_id
    })
    .await
    .expect("seed interact failed")
}

async fn assign_channel(
    pool: &deadpool_diesel::postgres::Pool,
    node_id: i64,
    frequency_hz: i64,
    name: &str,
    bandwidth_hz: Option<i32>,
) {
    let conn = pool.get().await.expect("seed connection");
    let name = name.to_owned();

    conn.interact(move |conn| {
        use server::db::models::channels::NewChannel;
        use server::db::models::enums::Modulation;
        use server::db::models::node_channels::NewNodeChannel;
        use server::db::schema::{channels, node_channels};

        let channel_id: i64 = diesel::insert_into(channels::table)
            .values(&NewChannel {
                name,
                frequency_hz,
                modulation: Modulation::Fm,
            })
            .returning(channels::id)
            .get_result(conn)
            .expect("failed to register the channel");

        diesel::insert_into(node_channels::table)
            .values(&NewNodeChannel {
                node_id,
                channel_id,
                bandwidth_hz,
            })
            .execute(conn)
            .expect("failed to assign the channel");
    })
    .await
    .expect("seed interact failed");
}

async fn seed_node(pool: &deadpool_diesel::postgres::Pool, identity: &str, name: &str) {
    let conn = pool.get().await.expect("seed connection");
    let new_node = NewNode {
        external_identity: Some(identity.to_owned()),
        name: name.to_owned(),
        latitude: Some(57.05),
        longitude: Some(9.92),
        hardware: json!({"device": "test dongle"}),
        capabilities: json!({"metrics": ["signal_strength"]}),
    };
    conn.interact(move |conn| {
        diesel::insert_into(nodes_schema::table)
            .values(&new_node)
            .execute(conn)
    })
    .await
    .expect("seed interact failed")
    .expect("seed node failed");
}

fn valid_registration(identity: &str) -> NodeRegistrationRequest {
    NodeRegistrationRequest {
        protocol_version: "1".to_owned(),
        identity: identity.to_owned(),
        name: format!("node-{identity}"),
        location: Some(Location {
            latitude: 57.05,
            longitude: 9.92,
        }),
        hardware: Some(Hardware {
            device: "test dongle".to_owned(),
            antenna: "test antenna".to_owned(),
            max_sample_rate_hz: 2_400_000,
        }),
        capabilities: Some(Capabilities {
            metrics: vec![i32::from(Metric::SignalStrength)],
            modulations: vec![i32::from(Modulation::Fm)],
        }),
    }
}

async fn plan_node(pool: &deadpool_diesel::postgres::Pool, name: &str) -> (i64, String) {
    let credential = format!("enrollment-{name}");
    let token = credential.clone();
    let planned = server::db::models::nodes::PlannedNode {
        name: name.to_owned(),
        latitude: None,
        longitude: None,
        hardware: json!({}),
        capabilities: json!({}),
        owner_id: None,
    };

    let conn = pool.get().await.expect("connection");
    let node_id: i64 = conn
        .interact(move |conn| {
            use diesel::RunQueryDsl;

            let node_id: i64 = diesel::insert_into(nodes_schema::table)
                .values(&planned)
                .returning(nodes_schema::id)
                .get_result(conn)?;
            diesel::insert_into(node_credentials_schema::table)
                .values(&server::db::models::node_credentials::NewNodeCredential { node_id, token })
                .execute(conn)?;

            Ok::<_, diesel::result::Error>(node_id)
        })
        .await
        .expect("plan interact")
        .expect("plan");

    (node_id, credential)
}

fn enrollment(identity: &str, credential: &str) -> tonic::Request<NodeRegistrationRequest> {
    bearer(valid_registration(identity), credential)
}

fn bearer<T>(message: T, credential: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {credential}").parse().expect("metadata"),
    );

    request
}

#[tokio::test]
async fn health_endpoint_returns_ok() {
    let pool = test_pool("api", "health").await;
    let response = app(AppState::new(pool))
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json body");
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn nodes_endpoint_requires_auth() {
    let pool = test_pool("api", "nodes_auth").await;
    let response = app(AppState::new(pool))
        .oneshot(
            Request::builder()
                .uri("/api/nodes")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn nodes_endpoint_lists_nodes() {
    let pool = test_pool("api", "nodes_list").await;
    let user_id = seed_user(&pool, "nodes-list@test").await;
    common::seed_session(&pool, user_id, "nodes-list-token").await;
    seed_node(&pool, "nodes-list-node", "Listed Node").await;

    let response = app(AppState::new(pool))
        .oneshot(
            Request::builder()
                .uri("/api/nodes")
                .header("authorization", "Bearer nodes-list-token")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json body");
    let nodes = json.as_array().expect("node array");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["name"], "Listed Node");
}

#[tokio::test]
async fn register_node_creates_node_and_credential() {
    let pool = test_pool("api", "register").await;
    let ingest = Ingest::new(AppState::new(pool.clone()));

    let (_, credential) = plan_node(&pool, "register-node-1").await;
    let response = ingest
        .register_node(enrollment("register-node-1", &credential))
        .await
        .expect("registration")
        .into_inner();

    assert_eq!(response.protocol_version, "1");
    assert!(!response.credential.is_empty());

    let delay = protocol::schedule_delay(response.schedule.as_ref(), response.server_time.as_ref())
        .expect("registration carries a report schedule");
    assert!(
        delay > std::time::Duration::ZERO && delay <= std::time::Duration::from_mins(1),
        "the first delivery was scheduled {delay:?} out"
    );

    let node_id = response.node_id;
    let conn = pool.get().await.expect("connection");
    let stored = conn
        .interact(move |conn| {
            use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};

            let node = nodes_schema::table
                .filter(nodes_schema::id.eq(node_id))
                .select(server::db::models::nodes::Node::as_select())
                .first(conn)?;
            let credential = node_credentials_schema::table
                .filter(node_credentials_schema::node_id.eq(node_id))
                .select(server::db::models::node_credentials::NodeCredential::as_select())
                .first(conn)?;
            Ok::<_, diesel::result::Error>((node, credential))
        })
        .await
        .expect("query interact")
        .expect("query");

    assert_eq!(
        stored.0.external_identity.as_deref(),
        Some("register-node-1")
    );
    assert_eq!(stored.1.token, response.credential);
}

#[tokio::test]
async fn register_node_rejects_duplicate_identity() {
    let pool = test_pool("api", "register_dup").await;
    let ingest = Ingest::new(AppState::new(pool.clone()));

    let (_, first) = plan_node(&pool, "register-node-dup").await;
    ingest
        .register_node(enrollment("register-node-dup", &first))
        .await
        .expect("first registration");

    let (_, second) = plan_node(&pool, "register-node-dup-other").await;
    let status = ingest
        .register_node(enrollment("register-node-dup", &second))
        .await
        .expect_err("second registration must fail");

    assert_eq!(status.code(), tonic::Code::AlreadyExists);
}

#[tokio::test]
async fn register_node_rejects_unknown_protocol_version() {
    let pool = test_pool("api", "register_version").await;
    let ingest = Ingest::new(AppState::new(pool.clone()));

    let (_, credential) = plan_node(&pool, "register-node-version").await;
    let mut message = valid_registration("register-node-version");
    message.protocol_version = "2".to_owned();
    let status = ingest
        .register_node(bearer(message, &credential))
        .await
        .expect_err("unknown version must fail");

    assert_eq!(status.code(), tonic::Code::InvalidArgument);
}

const fn stats(min: f64, max: f64, mean: f64, median: f64, stddev: f64, p95: f64) -> SampleStats {
    SampleStats {
        min,
        max,
        mean,
        median,
        stddev,
        p95,
        sample_count: 60,
    }
}

fn valid_report() -> MeasurementReport {
    let now = std::time::SystemTime::now();
    let window_start = now
        .checked_sub(std::time::Duration::from_mins(1))
        .unwrap_or(now);

    MeasurementReport {
        protocol_version: "1".to_owned(),
        window_start: Some(window_start.into()),
        window_end: Some(now.into()),
        channels: vec![ChannelMeasurement {
            frequency_hz: 89_700_000,
            modulation: i32::from(Modulation::Fm),
            label: "DR P4 Nordjylland".to_owned(),
            readings: vec![
                MetricReading {
                    metric: i32::from(Metric::SignalStrength),
                    stats: Some(stats(-48.2, -44.9, -46.3, -46.4, 0.6, -45.2)),
                },
                MetricReading {
                    metric: i32::from(Metric::SignalToNoise),
                    stats: Some(stats(27.1, 29.8, 28.4, 28.4, 0.5, 29.2)),
                },
            ],
        }],
    }
}

fn valid_health() -> HealthReport {
    HealthReport {
        protocol_version: "1".to_owned(),
        measured_at: Some(std::time::SystemTime::now().into()),
        uptime_seconds: 3_600.0,
        load_1m: 0.4,
        load_5m: 0.3,
        load_15m: 0.2,
        cpu_temperature_celsius: 44.5,
        clock_offset_seconds: 0.05,
    }
}

async fn registered_node(ingest: &Ingest, identity: &str) -> (i64, String) {
    let (_, credential) = plan_node(&ingest.state.pool, identity).await;
    let response = ingest
        .register_node(enrollment(identity, &credential))
        .await
        .expect("registration")
        .into_inner();

    (response.node_id, response.credential)
}

async fn count_measurements(pool: &deadpool_diesel::postgres::Pool) -> i64 {
    let conn = pool.get().await.expect("connection");
    conn.interact(|conn| {
        use diesel::QueryDsl;

        measurements_schema::table.count().get_result(conn)
    })
    .await
    .expect("count interact failed")
    .expect("count failed")
}

async fn count_health(pool: &deadpool_diesel::postgres::Pool) -> i64 {
    let conn = pool.get().await.expect("connection");
    conn.interact(|conn| {
        use diesel::QueryDsl;

        node_health_schema::table.count().get_result(conn)
    })
    .await
    .expect("count interact failed")
    .expect("count failed")
}

#[tokio::test]
async fn submit_measurements_rejects_missing_credential() {
    let pool = test_pool("api", "measure_noauth").await;
    let ingest = Ingest::new(AppState::new(pool));

    let status = ingest
        .submit_measurements(tonic::Request::new(valid_report()))
        .await
        .expect_err("an unauthenticated report must be rejected");

    assert_eq!(status.code(), tonic::Code::Unauthenticated);
}

#[tokio::test]
async fn submit_measurements_rejects_unknown_credential() {
    let pool = test_pool("api", "measure_badauth").await;
    let ingest = Ingest::new(AppState::new(pool));

    let status = ingest
        .submit_measurements(bearer(valid_report(), "not-a-real-credential"))
        .await
        .expect_err("an unknown credential must be rejected");

    assert_eq!(status.code(), tonic::Code::Unauthenticated);
}

#[tokio::test]
async fn submit_measurements_persists_one_row_per_reading() {
    let pool = test_pool("api", "measure_persist").await;
    let ingest = Ingest::new(AppState::new(pool.clone()));
    let (node_id, credential) = registered_node(&ingest, "measure-persist-node").await;

    let ack = ingest
        .submit_measurements(bearer(valid_report(), &credential))
        .await
        .expect("measurement report")
        .into_inner();

    assert_eq!(ack.protocol_version, "1");
    assert_eq!(ack.accepted_channels, 1);
    assert_eq!(count_measurements(&pool).await, 2);

    let conn = pool.get().await.expect("connection");
    let last_seen = conn
        .interact(move |conn| {
            use diesel::{ExpressionMethods, QueryDsl};

            nodes_schema::table
                .filter(nodes_schema::id.eq(node_id))
                .select(nodes_schema::last_seen_at)
                .first::<Option<chrono::NaiveDateTime>>(conn)
        })
        .await
        .expect("interact")
        .expect("query");

    assert!(last_seen.is_some(), "ingest must mark the node as seen");
}

#[tokio::test]
async fn submit_measurements_is_idempotent_under_backfill() {
    let pool = test_pool("api", "measure_backfill").await;
    let ingest = Ingest::new(AppState::new(pool.clone()));
    let (_, credential) = registered_node(&ingest, "measure-backfill-node").await;

    let report = valid_report();
    ingest
        .submit_measurements(bearer(report.clone(), &credential))
        .await
        .expect("first delivery");
    let ack = ingest
        .submit_measurements(bearer(report, &credential))
        .await
        .expect("resent delivery")
        .into_inner();

    assert_eq!(ack.accepted_channels, 1);
    assert_eq!(
        count_measurements(&pool).await,
        2,
        "a resent window must not duplicate rows"
    );
}

#[tokio::test]
async fn submit_measurements_rejects_suspended_node() {
    let pool = test_pool("api", "measure_suspended").await;
    let ingest = Ingest::new(AppState::new(pool.clone()));
    let (node_id, credential) = registered_node(&ingest, "measure-suspended-node").await;

    let conn = pool.get().await.expect("connection");
    conn.interact(move |conn| {
        use diesel::{ExpressionMethods, QueryDsl};

        diesel::update(nodes_schema::table.filter(nodes_schema::id.eq(node_id)))
            .set(nodes_schema::suspended.eq(true))
            .execute(conn)
    })
    .await
    .expect("interact")
    .expect("suspend");

    let status = ingest
        .submit_measurements(bearer(valid_report(), &credential))
        .await
        .expect_err("a suspended node must be rejected");

    assert_eq!(status.code(), tonic::Code::PermissionDenied);
    assert_eq!(count_measurements(&pool).await, 0);
}

#[tokio::test]
async fn report_health_persists_and_deduplicates() {
    let pool = test_pool("api", "health_persist").await;
    let ingest = Ingest::new(AppState::new(pool.clone()));
    let (_, credential) = registered_node(&ingest, "health-persist-node").await;

    let report = valid_health();
    let ack = ingest
        .report_health(bearer(report.clone(), &credential))
        .await
        .expect("health report")
        .into_inner();

    assert_eq!(ack.protocol_version, "1");
    assert_eq!(count_health(&pool).await, 1);

    ingest
        .report_health(bearer(report, &credential))
        .await
        .expect("resent health report");

    assert_eq!(
        count_health(&pool).await,
        1,
        "a resent health sample must not duplicate rows"
    );
}

#[tokio::test]
async fn report_health_rejects_missing_credential() {
    let pool = test_pool("api", "health_noauth").await;
    let ingest = Ingest::new(AppState::new(pool));

    let status = ingest
        .report_health(tonic::Request::new(valid_health()))
        .await
        .expect_err("an unauthenticated health report must be rejected");

    assert_eq!(status.code(), tonic::Code::Unauthenticated);
}

#[tokio::test]
async fn websocket_rejects_a_connection_without_a_session() {
    let pool = test_pool("api", "ws_noauth").await;
    let app = app(AppState::new(pool));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/ws")
                .body(Body::empty())
                .expect("failed to build request"),
        )
        .await
        .expect("request failed");

    assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn websocket_pushes_node_events_to_an_authenticated_client() {
    let pool = test_pool("api", "ws_push").await;
    let user_id = seed_user(&pool, "operator@example.com").await;
    common::seed_session(&pool, user_id, "ws-push-token").await;

    let state = AppState::new(pool);
    let addr = serve(state.clone()).await;
    let mut socket = connect_ws(addr, "ws-push-token").await;

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    state.publish_node(NodeEvent::Reported {
        node_id: 42,
        at: Utc::now().naive_utc(),
    });

    let message = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
        .await
        .expect("timed out waiting for the fragment")
        .expect("the socket closed before the fragment arrived")
        .expect("websocket error");

    let html = message.into_text().expect("fragment was not text");
    assert!(
        html.contains("id=\"node-42-state\""),
        "expected the out-of-band badge for node 42, got: {html}"
    );
    assert!(
        html.contains("hx-swap-oob"),
        "the fragment must be an out-of-band swap, got: {html}"
    );

    socket.close(None).await.expect("failed to close");
}

#[tokio::test]
async fn websocket_hides_nodes_a_technician_was_not_dispatched_to() {
    let pool = test_pool("api", "ws_filter").await;
    let technician_id = seed_user_with_role(&pool, "technician@example.com", 3).await;
    common::seed_session(&pool, technician_id, "ws-filter-token").await;
    seed_node(&pool, "dispatched", "Dispatched node").await;
    let dispatched_node_id = seed_dispatch(&pool, technician_id).await;

    let state = AppState::new(pool);
    let addr = serve(state.clone()).await;
    let mut socket = connect_ws(addr, "ws-filter-token").await;

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    state.publish_node(NodeEvent::Reported {
        node_id: dispatched_node_id + 1_000,
        at: Utc::now().naive_utc(),
    });
    state.publish_node(NodeEvent::Reported {
        node_id: dispatched_node_id,
        at: Utc::now().naive_utc(),
    });

    let message = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
        .await
        .expect("timed out waiting for the fragment")
        .expect("the socket closed before the fragment arrived")
        .expect("websocket error");

    let html = message.into_text().expect("fragment was not text");
    assert!(
        html.contains(&format!("id=\"node-{dispatched_node_id}-state\"")),
        "the first fragment must be the dispatched node badge, got: {html}"
    );

    socket.close(None).await.expect("failed to close");
}

#[tokio::test]
async fn channel_plan_is_empty_for_an_unassigned_node() {
    let pool = test_pool("api", "plan_empty").await;
    let ingest = Ingest::new(AppState::new(pool.clone()));

    let (_, credential) = plan_node(&pool, "plan-empty").await;
    let registration = ingest
        .register_node(enrollment("plan-empty", &credential))
        .await
        .expect("registration")
        .into_inner();

    let plan = ingest
        .get_channel_plan(bearer(
            protocol::v1::ChannelPlanRequest {
                protocol_version: "1".to_owned(),
                known_plan_version: 0,
            },
            &registration.credential,
        ))
        .await
        .expect("plan")
        .into_inner();

    assert_eq!(plan.protocol_version, "1");
    assert_eq!(plan.plan_version, 0);
    assert!(plan.channels.is_empty());
}

#[tokio::test]
async fn channel_plan_serves_assignments_and_bumps_its_version() {
    let pool = test_pool("api", "plan_assigned").await;
    let ingest = Ingest::new(AppState::new(pool.clone()));

    let (_, credential) = plan_node(&pool, "plan-assigned").await;
    let registration = ingest
        .register_node(enrollment("plan-assigned", &credential))
        .await
        .expect("registration")
        .into_inner();

    assign_channel(
        &pool,
        registration.node_id,
        89_700_000,
        "DR P4 Nordjylland",
        Some(200_000),
    )
    .await;

    let plan = ingest
        .get_channel_plan(bearer(
            protocol::v1::ChannelPlanRequest {
                protocol_version: "1".to_owned(),
                known_plan_version: 0,
            },
            &registration.credential,
        ))
        .await
        .expect("plan")
        .into_inner();

    assert_eq!(
        plan.plan_version, 1,
        "assigning a channel must bump the plan version"
    );
    assert_eq!(plan.channels.len(), 1);

    let channel = plan.channels.first().expect("one assignment");
    assert_eq!(channel.frequency_hz, 89_700_000);
    assert_eq!(channel.modulation, i32::from(Modulation::Fm));
    assert_eq!(channel.label, "DR P4 Nordjylland");
    assert_eq!(channel.bandwidth_hz, 200_000);
}

#[tokio::test]
async fn channel_plan_rejects_an_unauthenticated_caller() {
    let pool = test_pool("api", "plan_noauth").await;
    let ingest = Ingest::new(AppState::new(pool));

    let status = ingest
        .get_channel_plan(tonic::Request::new(protocol::v1::ChannelPlanRequest {
            protocol_version: "1".to_owned(),
            known_plan_version: 0,
        }))
        .await
        .expect_err("an unauthenticated plan request must be rejected");

    assert_eq!(status.code(), tonic::Code::Unauthenticated);
}

fn live_sample(session_id: u64) -> protocol::v1::LiveSample {
    protocol::v1::LiveSample {
        protocol_version: "1".to_owned(),
        session_id,
        measured_at: Some(std::time::SystemTime::now().into()),
        frequency_hz: 96_500_000,
        modulation: i32::from(protocol::v1::Modulation::Fm),
        label: "DR P4".to_owned(),
        readings: vec![protocol::v1::LiveReading {
            metric: i32::from(protocol::v1::Metric::SignalStrength),
            value: -42.0,
        }],
    }
}

#[tokio::test]
async fn a_live_session_reaches_a_watching_node_and_its_samples_are_never_stored() {
    let pool = test_pool("api", "live_session").await;
    let ingest = Ingest::new(AppState::new(pool.clone()));
    let (node_id, credential) = registered_node(&ingest, "live-node").await;

    let mut commands = ingest
        .watch_live(bearer(
            protocol::v1::LiveWatchRequest {
                protocol_version: "1".to_owned(),
            },
            &credential,
        ))
        .await
        .expect("a command stream")
        .into_inner();

    assert_eq!(ingest.state.live.open(node_id), server::live::Reach::Told);

    let command = commands
        .next()
        .await
        .expect("the stream is open")
        .expect("a command");
    assert_ne!(command.session_id, 0, "the node was told to stop");

    let before = count_measurements(&pool).await;
    let ack = ingest
        .submit_live(bearer(live_sample(command.session_id), &credential))
        .await
        .expect("the sample is accepted")
        .into_inner();

    assert_eq!(
        ack.session_id, command.session_id,
        "the node was told to stop a session it is running"
    );
    assert_eq!(
        count_measurements(&pool).await,
        before,
        "a live sample was persisted"
    );
}

#[tokio::test]
async fn a_sample_for_a_session_nobody_asked_for_is_answered_with_a_stop() {
    let pool = test_pool("api", "live_stale").await;
    let ingest = Ingest::new(AppState::new(pool));
    let (_, credential) = registered_node(&ingest, "stale-node").await;

    let ack = ingest
        .submit_live(bearer(live_sample(7), &credential))
        .await
        .expect("the sample is taken and refused politely")
        .into_inner();

    assert_eq!(ack.session_id, 0, "a forgotten session was believed");
}

#[tokio::test]
async fn an_expired_live_session_stops_being_accepted() {
    let pool = test_pool("api", "live_expiry").await;
    let ingest = Ingest::new(AppState::new(pool));
    let (node_id, credential) = registered_node(&ingest, "expiring-node").await;

    let mut commands = ingest
        .watch_live(bearer(
            protocol::v1::LiveWatchRequest {
                protocol_version: "1".to_owned(),
            },
            &credential,
        ))
        .await
        .expect("a command stream")
        .into_inner();
    assert_eq!(ingest.state.live.open(node_id), server::live::Reach::Told);
    let session_id = commands
        .next()
        .await
        .expect("the stream is open")
        .expect("a command")
        .session_id;

    ingest.state.live.close(node_id);

    let ack = ingest
        .submit_live(bearer(live_sample(session_id), &credential))
        .await
        .expect("the sample is taken")
        .into_inner();

    assert_eq!(ack.session_id, 0, "a closed session still took samples");
}

#[tokio::test]
async fn a_live_sample_outside_its_metric_range_is_rejected() {
    let pool = test_pool("api", "live_range").await;
    let ingest = Ingest::new(AppState::new(pool));
    let (_, credential) = registered_node(&ingest, "range-node").await;

    let mut sample = live_sample(1);
    sample.readings[0].value = 12.0;

    let status = ingest
        .submit_live(bearer(sample, &credential))
        .await
        .expect_err("an impossible reading must be rejected");

    assert_eq!(status.code(), tonic::Code::InvalidArgument);
}

#[tokio::test]
async fn the_ack_carries_the_cadence_the_node_row_asks_for() {
    let pool = test_pool("api", "live_cadence").await;
    let ingest = Ingest::new(AppState::new(pool.clone()));
    let (node_id, credential) = registered_node(&ingest, "cadence-node").await;

    let conn = pool.get().await.expect("connection");
    conn.interact(move |conn| {
        use diesel::{ExpressionMethods, QueryDsl, RunQueryDsl};

        diesel::update(nodes_schema::table.filter(nodes_schema::id.eq(node_id)))
            .set(nodes_schema::report_interval_seconds.eq(10))
            .execute(conn)
    })
    .await
    .expect("update interact")
    .expect("update");

    let ack = ingest
        .submit_measurements(bearer(valid_report(), &credential))
        .await
        .expect("the report is accepted")
        .into_inner();

    let delay = protocol::schedule_delay(ack.schedule.as_ref(), ack.server_time.as_ref())
        .expect("a schedule");

    assert!(
        delay <= std::time::Duration::from_secs(10),
        "a node on a ten second cadence was told to wait {delay:?}"
    );
}
