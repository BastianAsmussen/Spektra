use std::time::Instant;

use askama::Template;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{Html, Response};
use axum::routing::get;
use axum::{Json, Router};
use diesel::prelude::*;
use serde::Serialize;
use utoipa::ToSchema;

use super::admin::require_admin;
use super::auth::{AuthPage, AuthUser};
use super::errors::{ApiError, ErrorBody};
use super::pages;
use super::series::{Points, Presentation};
use crate::db::schema::nodes as nodes_schema;
use crate::ops::{Snapshot, ratio};
use crate::state::AppState;
use crate::templates::{DriftTemplate, OpsTilesFragment, stamp_or_empty};

/// All routes under `/api/ops`, plus the page that draws them.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/ops/status", get(get_status))
        .route("/api/ops/throughput", get(get_throughput))
        .route("/drift", get(page))
        .route("/fragments/ops", get(tiles))
}

/// The server's own state, as it sees it.
#[derive(Debug, Serialize, ToSchema)]
pub struct OpsStatus {
    /// Whether the server considers itself to be working.
    pub healthy: bool,
    /// One line per thing that is wrong. Empty when healthy.
    pub problems: Vec<String>,
    /// Nodes registered, suspended ones included.
    pub fleet_size: u64,
    #[serde(flatten)]
    pub metrics: Snapshot,
}

///
#[derive(Debug, Serialize, ToSchema)]
pub struct ThroughputSeries {
    /// What the source is called, in Danish, for the caption.
    pub source_name: &'static str,
    /// How to label and scale the values.
    pub presentation: Presentation,
    pub points: Points,
}

/// Report the server's own operational state.
///
/// # Errors
///
#[utoipa::path(
    get,
    path = "/api/ops/status",
    responses(
        (status = 200, description = "The server's own operational state", body = OpsStatus),
        (status = 401, description = "Not authenticated", body = ErrorBody),
    ),
    security(("session_token" = [])),
    tag = "ops"
)]
pub async fn get_status(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<OpsStatus>, ApiError> {
    let conn = state.pool.get().await?;
    let fleet: i64 = conn
        .interact(|conn| nodes_schema::table.count().get_result(conn))
        .await??;
    let fleet_size = u64::try_from(fleet).unwrap_or(0);

    let metrics = state.metrics.snapshot(&state.pool);
    let problems = metrics.problems(chrono::Utc::now().naive_utc(), fleet_size);

    Ok(Json(OpsStatus {
        healthy: problems.is_empty(),
        problems,
        fleet_size,
        metrics,
    }))
}

/// Measurements per second over the sampled history, oldest first.
///
/// # Errors
///
#[utoipa::path(
    get,
    path = "/api/ops/throughput",
    responses(
        (status = 200, description = "Ingest throughput over the sampled window", body = ThroughputSeries),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 403, description = "Not an administrator", body = ErrorBody),
    ),
    security(("session_token" = [])),
    tag = "ops"
)]
pub async fn get_throughput(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<ThroughputSeries>, ApiError> {
    drop(require_admin(&state, auth.session.user_id).await?);

    let (at, value) = state.metrics.throughput();

    Ok(Json(ThroughputSeries {
        source_name: "serverens egne tællere",
        presentation: Presentation {
            name: "Dataindtag",
            unit: "målinger/s",
            scale: 1.0,
        },
        points: Points { at, value },
    }))
}

async fn page(auth: AuthPage, State(state): State<AppState>) -> Result<Html<String>, ApiError> {
    let user_id = auth.0.session.user_id;
    let access = require_admin(&state, user_id).await?;
    let chrome = pages::chrome_for(&state, user_id).await?;

    let html = DriftTemplate {
        nodes_total: chrome.nodes_total,
        silent: chrome.silent,
        open_alarms: chrome.open_alarms,
        open_orders: chrome.open_orders,
        user_name: chrome.user_name,
        user_role: access.role,
        live: false,
    }
    .render()
    .map_err(ApiError::internal)?;

    Ok(Html(html))
}

async fn tiles(auth: AuthPage, State(state): State<AppState>) -> Result<Html<String>, ApiError> {
    let user_id = auth.0.session.user_id;
    drop(require_admin(&state, user_id).await?);

    let conn = state.pool.get().await?;
    let fleet: i64 = conn
        .interact(|conn| nodes_schema::table.count().get_result(conn))
        .await??;
    let fleet_size = u64::try_from(fleet).unwrap_or(0);

    let metrics = state.metrics.snapshot(&state.pool);
    let rates = state.metrics.rates();
    let problems = metrics.problems(chrono::Utc::now().naive_utc(), fleet_size);

    let html = OpsTilesFragment {
        healthy: problems.is_empty(),
        problems,
        window_seconds: rates.window_seconds,
        measurements: danish(rates.measurements, 1),
        health: danish(rates.health, 1),
        rejected: danish(rates.rejected, 1),
        failed: danish(rates.failed, 1),
        http_requests: danish(rates.http_requests, 1),
        http_server_errors: danish(rates.http_server_errors, 2),
        http_mean_ms: danish(ratio(rates.http_mean_micros, 1_000), 1),
        http_slowest_ms: danish(ratio(metrics.http_slowest_micros, 1_000), 1),
        pool_size: metrics.pool_size,
        pool_available: metrics.pool_available,
        pool_waiting: metrics.pool_waiting,
        measurements_total: metrics.measurements_accepted,
        registrations: metrics.registrations,
        alarms_raised: metrics.alarms_raised,
        last_ingest: stamp_or_empty(metrics.last_ingest_at),
        last_detection: stamp_or_empty(metrics.last_detection_at),
    }
    .render()
    .map_err(ApiError::internal)?;

    Ok(Html(html))
}

fn danish(value: f64, decimals: usize) -> String {
    format!("{value:.decimals$}").replace('.', ",")
}

/// Time every request and record its status.
///
pub async fn measure(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let started = Instant::now();
    let response = next.run(request).await;
    let micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);

    state
        .metrics
        .http_served(micros, response.status().as_u16());

    response
}
