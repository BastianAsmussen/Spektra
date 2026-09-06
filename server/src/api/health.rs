use axum::{Json, Router, routing::get};
use serde::Serialize;
use utoipa::ToSchema;

use crate::state::AppState;

/// Liveness probe for the ops surface.
#[derive(Debug, Serialize, ToSchema)]
pub struct HealthStatus {
    /// Always `"ok"` while the process serves requests.
    pub status: &'static str,
    /// Build version.
    pub version: &'static str,
}

/// All routes under `/api/health`.
pub fn routes() -> Router<AppState> {
    Router::new().route("/api/health", get(get_health))
}

/// Report process liveness.
#[utoipa::path(
    get,
    path = "/api/health",
    responses(
        (status = 200, description = "Process is up", body = HealthStatus),
    ),
    tag = "ops"
)]
pub async fn get_health() -> Json<HealthStatus> {
    Json(HealthStatus {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}
