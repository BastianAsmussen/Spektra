use axum::Router;
use axum::http::{HeaderValue, header};
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::api::{admin, alarms, auth, health, nodes, ops, pages, series, work_orders, ws};
use crate::state::AppState;

const DEFAULT_STATIC_DIR: &str = "server/assets/dist";

/// [`router_with_static`] with the directory from `SPEKTRA_STATIC_DIR`.
pub fn router(state: AppState) -> Router<()> {
    let dir = std::env::var("SPEKTRA_STATIC_DIR").unwrap_or_else(|_| DEFAULT_STATIC_DIR.to_owned());
    router_with_static(state, &dir)
}

/// Every route, the static assets and the response-transfer layers.
pub fn router_with_static(state: AppState, dir: &str) -> Router<()> {
    Router::new()
        .merge(health::routes())
        .merge(nodes::routes())
        .merge(alarms::routes())
        .merge(alarms::fragments::routes())
        .merge(work_orders::routes())
        .merge(work_orders::fragments::routes())
        .merge(ops::routes())
        .merge(series::routes())
        .merge(pages::routes())
        .merge(admin::routes())
        .merge(admin::fragments::routes())
        .merge(auth::routes())
        .merge(ws::routes())
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .nest_service(
            "/static",
            Router::new().fallback_service(static_files(dir)).layer(
                SetResponseHeaderLayer::overriding(
                    header::CACHE_CONTROL,
                    HeaderValue::from_static("public, max-age=31536000, immutable"),
                ),
            ),
        )
        .layer(CompressionLayer::new())
        .with_state(state)
}

fn static_files(dir: &str) -> ServeDir {
    if std::path::Path::new(dir).is_dir() {
        tracing::info!(directory = dir, "serving web client assets");
    } else {
        tracing::warn!(
            directory = dir,
            "SPEKTRA_STATIC_DIR does not exist; the dashboard will load unstyled"
        );
    }

    ServeDir::new(dir).precompressed_br().precompressed_gzip()
}
