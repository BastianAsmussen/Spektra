use color_eyre::Result;
use color_eyre::eyre::{WrapErr, eyre};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use server::api::{admin, alarms, auth, health, nodes, ops, series, work_orders};
use server::grpc;
use server::jobs;
use server::notify::Ntfy;
use server::state::AppState;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/");

const WEB_POOL_SIZE: usize = 4;
const INGEST_POOL_SIZE: usize = 8;

#[derive(OpenApi)]
#[openapi(
    paths(
        health::get_health,
        nodes::list_nodes,
        alarms::list_alarms,
        alarms::get_alarm,
        alarms::transition_alarm,
        work_orders::list_work_orders,
        work_orders::dispatch,
        work_orders::complete,
        ops::get_status,
        ops::get_throughput,
        series::get_series,
        admin::list_users,
        admin::create_user,
        admin::update_user,
        admin::plan_node,
        admin::update_node,
        admin::rotate_credential,
        admin::set_suspension,
    ),
    components(schemas(
        server::db::models::nodes::Node,
        server::db::models::alarms::Alarm,
        server::db::models::alarm_events::AlarmEvent,
        server::db::models::enums::AlarmState,
        server::db::models::enums::Metric,
        server::api::alarms::AlarmDetail,
        server::api::alarms::TransitionRequest,
        server::db::models::work_orders::WorkOrder,
        server::db::models::enums::WorkOrderStatus,
        server::api::work_orders::DispatchRequest,
        server::api::work_orders::FieldReport,
        server::api::work_orders::CompletionResult,
        server::api::ops::OpsStatus,
        server::api::ops::ThroughputSeries,
        server::ops::Snapshot,
        server::api::series::Series,
        server::api::series::Source,
        server::api::errors::ErrorBody,
        server::api::health::HealthStatus,
        server::api::admin::UserSummary,
        server::api::admin::NewUserRequest,
        server::api::admin::UserUpdate,
        server::api::admin::PlannedNodeRequest,
        server::api::admin::NodeUpdate,
        server::api::admin::MintedCredential,
        server::api::admin::SuspensionRequest,
    )),
    modifiers(&SecurityAddon),
    tags(
        (name = "ops", description = "Operational endpoints"),
        (name = "nodes", description = "Node endpoints"),
        (name = "alarms", description = "Alarm lifecycle endpoints"),
        (name = "work orders", description = "Dispatch and field verification"),
        (name = "series", description = "Measurement history"),
        (name = "administration", description = "User accounts and node suspension")
    )
)]
struct ApiDoc;

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "session_token",
                utoipa::openapi::security::SecurityScheme::ApiKey(
                    utoipa::openapi::security::ApiKey::Cookie(
                        utoipa::openapi::security::ApiKeyValue::new("session_token"),
                    ),
                ),
            );
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    dotenvy::dotenv().ok();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("{}=debug", env!("CARGO_CRATE_NAME")).into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let db_url =
        std::env::var("DATABASE_URL").wrap_err("DATABASE_URL environment variable is not set!")?;

    let pool = build_pool(&db_url, WEB_POOL_SIZE).wrap_err("Failed to build the web pool!")?;
    let ingest_pool =
        build_pool(&db_url, INGEST_POOL_SIZE).wrap_err("Failed to build the ingest pool!")?;

    {
        let conn = pool
            .get()
            .await
            .wrap_err("Failed to obtain a database connection from the pool!")?;

        conn.interact(|conn| conn.run_pending_migrations(MIGRATIONS).map(|_| ()))
            .await
            .map_err(|e| eyre!("Migration task panicked: {e:?}"))?
            .map_err(|e| eyre!("Failed to run database migrations: {e}"))?;

        tracing::info!("Database migrations applied successfully.");
    }

    match auth::bootstrap_admin(&pool).await {
        Ok(true) => tracing::info!("created the administrator named by SPEKTRA_ADMIN_EMAIL"),
        Ok(false) => {}
        Err(err) => tracing::error!(error = %err, "could not create the administrator"),
    }

    let state = AppState::new(pool).with_ingest_pool(ingest_pool);
    tokio::spawn(jobs::run(state.clone()));

    let notifier = Ntfy::from_env();
    if notifier.is_none() {
        tracing::info!("ntfy is not configured; alarms go to the live channel only");
    }
    tokio::spawn(jobs::detect(state.clone(), notifier));
    tokio::spawn(jobs::sample_throughput(state.clone()));

    let app = server::web::router(state.clone())
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            ops::measure,
        ));

    let grpc_addr_raw =
        std::env::var("GRPC_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:50051".to_owned());
    let grpc_addr = grpc_addr_raw
        .parse()
        .wrap_err_with(|| format!("Failed to parse gRPC bind address '{grpc_addr_raw}'"))?;
    let grpc_task = tokio::spawn(grpc::serve(state, grpc_addr));

    let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_owned());
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .wrap_err_with(|| format!("Failed to bind to {bind_addr}!"))?;

    tracing::info!("Listening on {bind_addr}...");
    tracing::info!("Swagger UI available at http://{bind_addr}/swagger-ui/");
    tracing::info!("gRPC ingest listening on {grpc_addr}...");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .wrap_err("Server exited with an error!")?;

    grpc_task
        .await
        .map_err(|e| eyre!("gRPC server task failed: {e}"))??;

    tracing::info!("Server shut down gracefully.");

    Ok(())
}

fn build_pool(db_url: &str, size: usize) -> Result<deadpool_diesel::postgres::Pool> {
    let manager = deadpool_diesel::postgres::Manager::new(
        db_url.to_owned(),
        deadpool_diesel::Runtime::Tokio1,
    );

    deadpool_diesel::postgres::Pool::builder(manager)
        .max_size(size)
        .build()
        .map_err(Into::into)
}

async fn shutdown_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        tracing::error!(error = %err, "Failed to install CTRL+C signal handler!");
        std::future::pending::<()>().await;
    }

    tracing::info!("Shutdown signal received, draining connections...");
}
