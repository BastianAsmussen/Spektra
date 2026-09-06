use std::time::Instant;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use diesel::prelude::*;
use serde::Serialize;
use utoipa::ToSchema;

use super::auth::AuthUser;
use super::errors::{ApiError, ErrorBody};
use crate::db::schema::nodes as nodes_schema;
use crate::ops::Snapshot;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/ops/status", get(get_status))
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
