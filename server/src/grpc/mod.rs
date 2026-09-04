pub mod persist;
pub mod validate;

use std::net::SocketAddr;
use std::time::SystemTime;

use chrono::Utc;
use diesel::prelude::*;
use protocol::v1::node_ingest_server::{NodeIngest, NodeIngestServer};
use protocol::v1::{
    ChannelAssignment, ChannelPlan, ChannelPlanRequest, HealthAck, HealthReport, IngestAck,
    MeasurementReport, NodeRegistrationRequest, NodeRegistrationResponse,
};
use rand::RngExt;
use serde_json::json;
use tonic::metadata::MetadataMap;
use tonic::{Request, Response, Status};

use crate::db::schema::node_credentials as credentials_schema;
use crate::db::schema::nodes as nodes_schema;
use crate::state::{AppState, NodeEvent};

/// The v1 ingest service.
///
#[derive(Clone)]
pub struct Ingest {
    pub state: AppState,
}

impl Ingest {
    /// Build the service from shared state.
    #[must_use]
    pub const fn new(state: AppState) -> Self {
        Self { state }
    }

    /// Wrap the service for a tonic server.
    #[must_use]
    pub fn server(self) -> NodeIngestServer<Self> {
        NodeIngestServer::new(self)
    }

    ///
    async fn authenticate(&self, metadata: &MetadataMap) -> Result<i64, Status> {
        let token = bearer_token(metadata)?;
        let now = Utc::now().naive_utc();

        let conn = self.state.pool.get().await.map_err(internal)?;
        let (node_id, suspended) = conn
            .interact(move |conn| {
                credentials_schema::table
                    .inner_join(nodes_schema::table)
                    .filter(credentials_schema::token.eq(token))
                    .filter(credentials_schema::revoked_at.is_null())
                    .filter(
                        credentials_schema::expires_at
                            .is_null()
                            .or(credentials_schema::expires_at.gt(now)),
                    )
                    .select((nodes_schema::id, nodes_schema::suspended))
                    .first::<(i64, bool)>(conn)
            })
            .await
            .map_err(internal)?
            .map_err(|err| match err {
                diesel::result::Error::NotFound => {
                    Status::unauthenticated("unknown, revoked or expired node credential")
                }
                other => internal(other),
            })?;

        if suspended {
            return Err(Status::permission_denied("this node is suspended"));
        }

        Ok(node_id)
    }

    ///
    async fn authenticate_registration(
        &self,
        metadata: &MetadataMap,
    ) -> Result<(i64, String), Status> {
        let token = bearer_token(metadata)?;
        let node_id = self.authenticate(metadata).await?;

        Ok((node_id, token))
    }
}

fn bearer_token(metadata: &MetadataMap) -> Result<String, Status> {
    let header = metadata
        .get("authorization")
        .ok_or_else(|| Status::unauthenticated("missing authorization metadata"))?
        .to_str()
        .map_err(|_| Status::unauthenticated("authorization metadata is not valid ASCII"))?;

    let token = header
        .strip_prefix("Bearer ")
        .ok_or_else(|| {
            Status::unauthenticated("authorization must be formatted as 'Bearer <credential>'")
        })?
        .trim();

    if token.is_empty() {
        return Err(Status::unauthenticated("the bearer credential is empty"));
    }

    Ok(token.to_owned())
}

/// Run the gRPC ingest server until the process receives a shutdown signal.
///
/// # Errors
///
/// Returns an error if binding the socket or serving fails.
pub async fn serve(state: AppState, addr: SocketAddr) -> Result<(), tonic::transport::Error> {
    tonic::transport::Server::builder()
        .add_service(Ingest::new(state).server())
        .serve_with_shutdown(addr, shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        tracing::error!(error = %err, "failed to install CTRL+C handler, gRPC server will not drain");
        std::future::pending::<()>().await;
    }
}

fn internal<E: std::error::Error>(err: E) -> Status {
    tracing::error!(error = %err, "ingest internal error");
    Status::internal("an unexpected error occurred")
}

/// Generate a 32-byte random bearer credential, hex encoded.
///
#[must_use]
pub fn generate_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::rng().fill(&mut bytes[..]);

    hex::encode(bytes)
}

///
fn count<T>(
    metrics: &crate::ops::Metrics,
    outcome: &Result<Response<T>, Status>,
    accepted: fn(&crate::ops::Metrics),
) {
    match outcome {
        Ok(_) => accepted(metrics),
        Err(status) if status.code() == tonic::Code::Internal => metrics.ingest_failed(),
        Err(_) => metrics.ingest_rejected(),
    }
}

#[tonic::async_trait]
impl NodeIngest for Ingest {
    async fn register_node(
        &self,
        request: Request<NodeRegistrationRequest>,
    ) -> Result<Response<NodeRegistrationResponse>, Status> {
        let outcome = self.register_node_inner(request).await;
        count(
            &self.state.metrics,
            &outcome,
            crate::ops::Metrics::registered,
        );

        outcome
    }

    async fn submit_measurements(
        &self,
        request: Request<MeasurementReport>,
    ) -> Result<Response<IngestAck>, Status> {
        let outcome = self.submit_measurements_inner(request).await;
        count(
            &self.state.metrics,
            &outcome,
            crate::ops::Metrics::measurement_accepted,
        );

        outcome
    }

    async fn report_health(
        &self,
        request: Request<HealthReport>,
    ) -> Result<Response<HealthAck>, Status> {
        let outcome = self.report_health_inner(request).await;
        count(
            &self.state.metrics,
            &outcome,
            crate::ops::Metrics::health_accepted,
        );

        outcome
    }

    async fn get_channel_plan(
        &self,
        request: Request<ChannelPlanRequest>,
    ) -> Result<Response<ChannelPlan>, Status> {
        self.channel_plan_inner(request).await
    }
}

impl Ingest {
    ///
    async fn register_node_inner(
        &self,
        request: Request<NodeRegistrationRequest>,
    ) -> Result<Response<NodeRegistrationResponse>, Status> {
        let (node_id, credential) = self.authenticate_registration(request.metadata()).await?;

        let req = request.into_inner();
        validate::registration(&req)?;

        let hardware = req.hardware.unwrap_or_default();
        let capabilities = req.capabilities.unwrap_or_default();
        let identity = req.identity.clone();
        let name = req.name.clone();
        let latitude = req.location.as_ref().map(|loc| loc.latitude);
        let longitude = req.location.as_ref().map(|loc| loc.longitude);
        let hardware = json!({
            "device": hardware.device,
            "antenna": hardware.antenna,
            "max_sample_rate_hz": hardware.max_sample_rate_hz,
        });
        let capabilities = json!({
            "metrics": capabilities.metrics,
            "modulations": capabilities.modulations,
        });

        let conn = self.state.pool.get().await.map_err(internal)?;

        conn.interact(move |conn| {
            conn.transaction(|conn| {
                let existing: Option<String> = nodes_schema::table
                    .filter(nodes_schema::id.eq(node_id))
                    .select(nodes_schema::external_identity)
                    .for_update()
                    .first(conn)?;

                if existing.is_some_and(|bound| bound != identity) {
                    return Err(diesel::result::Error::RollbackTransaction);
                }

                diesel::update(nodes_schema::table.filter(nodes_schema::id.eq(node_id)))
                    .set((
                        nodes_schema::external_identity.eq(identity),
                        nodes_schema::hardware.eq(hardware),
                        nodes_schema::capabilities.eq(capabilities),
                    ))
                    .execute(conn)?;

                diesel::update(
                    nodes_schema::table
                        .filter(nodes_schema::id.eq(node_id))
                        .filter(nodes_schema::name.eq("")),
                )
                .set(nodes_schema::name.eq(name))
                .execute(conn)?;

                if latitude.is_some() && longitude.is_some() {
                    diesel::update(
                        nodes_schema::table
                            .filter(nodes_schema::id.eq(node_id))
                            .filter(nodes_schema::latitude.is_null()),
                    )
                    .set((
                        nodes_schema::latitude.eq(latitude),
                        nodes_schema::longitude.eq(longitude),
                    ))
                    .execute(conn)?;
                }

                Ok(())
            })
        })
        .await
        .map_err(internal)?
        .map_err(|err: diesel::result::Error| match err {
            diesel::result::Error::RollbackTransaction => Status::permission_denied(
                "this credential is already enrolled to a different node identity",
            ),
            diesel::result::Error::DatabaseError(
                diesel::result::DatabaseErrorKind::UniqueViolation,
                _,
            ) => Status::already_exists("a node with this identity is already registered"),
            other => internal(other),
        })?;

        Ok(Response::new(NodeRegistrationResponse {
            protocol_version: validate::PROTOCOL_VERSION.to_owned(),
            node_id,
            credential,
            server_time: Some(SystemTime::now().into()),
        }))
    }

    async fn submit_measurements_inner(
        &self,
        request: Request<MeasurementReport>,
    ) -> Result<Response<IngestAck>, Status> {
        let node_id = self.authenticate(request.metadata()).await?;

        validate::measurements(request.get_ref())?;
        let report = persist::PreparedReport::try_from(request.into_inner())?;

        let accepted_channels = u64::try_from(report.channel_count())
            .map_err(|_| Status::invalid_argument("the report carries too many channels"))?;

        let conn = self.state.pool.get().await.map_err(internal)?;
        let written = conn
            .interact(move |conn| report.write(conn, node_id))
            .await
            .map_err(internal)?
            .map_err(internal)?;

        tracing::debug!(node_id, accepted_channels, written, "measurements ingested");

        self.state.publish_node(NodeEvent::Reported {
            node_id,
            at: Utc::now().naive_utc(),
        });

        Ok(Response::new(IngestAck {
            protocol_version: validate::PROTOCOL_VERSION.to_owned(),
            server_time: Some(SystemTime::now().into()),
            accepted_channels,
        }))
    }

    async fn report_health_inner(
        &self,
        request: Request<HealthReport>,
    ) -> Result<Response<HealthAck>, Status> {
        let node_id = self.authenticate(request.metadata()).await?;

        validate::health(request.get_ref())?;
        let row = persist::health_row(node_id, request.get_ref())?;

        let conn = self.state.pool.get().await.map_err(internal)?;
        let written = conn
            .interact(move |conn| persist::write_health(conn, &row))
            .await
            .map_err(internal)?
            .map_err(internal)?;

        tracing::debug!(node_id, written, "health ingested");

        Ok(Response::new(HealthAck {
            protocol_version: validate::PROTOCOL_VERSION.to_owned(),
            server_time: Some(SystemTime::now().into()),
        }))
    }

    async fn channel_plan_inner(
        &self,
        request: Request<ChannelPlanRequest>,
    ) -> Result<Response<ChannelPlan>, Status> {
        let node_id = self.authenticate(request.metadata()).await?;

        validate::channel_plan(request.get_ref())?;
        let known = request.into_inner().known_plan_version;

        let conn = self.state.pool.get().await.map_err(internal)?;
        let (plan_version, assignments) = conn
            .interact(move |conn| persist::channel_plan(conn, node_id))
            .await
            .map_err(internal)?
            .map_err(internal)?;

        let channels = assignments
            .into_iter()
            .map(|assignment| ChannelAssignment {
                frequency_hz: assignment.frequency_hz.unsigned_abs(),
                modulation: i32::from(assignment.modulation),
                label: assignment.label,
                bandwidth_hz: assignment.bandwidth_hz.unwrap_or(0).unsigned_abs(),
            })
            .collect();

        tracing::debug!(node_id, known, plan_version, "channel plan served");

        Ok(Response::new(ChannelPlan {
            protocol_version: validate::PROTOCOL_VERSION.to_owned(),
            plan_version: plan_version.unsigned_abs(),
            channels,
            issued_at: Some(SystemTime::now().into()),
        }))
    }
}
