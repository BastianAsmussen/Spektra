pub mod persist;
pub mod validate;

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use diesel::prelude::*;
use futures_util::Stream;
use protocol::v1::node_ingest_server::{NodeIngest, NodeIngestServer};
use protocol::v1::{
    ChannelAssignment, ChannelPlan, ChannelPlanRequest, HealthAck, HealthReport, IngestAck,
    LiveAck, LiveCommand, LiveSample, LiveWatchRequest, MeasurementReport, NodeRegistrationRequest,
    NodeRegistrationResponse, ReportSchedule,
};
use rand::RngExt;
use serde_json::json;
use tonic::metadata::MetadataMap;
use tonic::{Request, Response, Status};

use crate::db::schema::node_credentials as credentials_schema;
use crate::db::schema::nodes as nodes_schema;
use crate::live;
use crate::state::{AppState, NodeEvent};

const DEFAULT_CYCLE_MS: i64 = 60_000;

/// Knuth's multiplier, coprime with [`DEFAULT_CYCLE_MS`].
const SLOT_SPREAD: i64 = 2_654_435_761;

///
fn report_schedule(node_id: i64, now: DateTime<Utc>, cycle_ms: i64) -> Option<ReportSchedule> {
    let cycle_ms = if cycle_ms > 0 {
        cycle_ms
    } else {
        DEFAULT_CYCLE_MS
    };

    let millis = now.timestamp_millis();
    let slot = node_id.wrapping_mul(SLOT_SPREAD).rem_euclid(cycle_ms);
    let cycle_start = millis.saturating_sub(millis.rem_euclid(cycle_ms));
    let mut next = cycle_start.saturating_add(slot);
    if next <= millis {
        next = next.saturating_add(cycle_ms);
    }

    Some(ReportSchedule {
        next_report_at: Some(SystemTime::from(DateTime::from_timestamp_millis(next)?).into()),
    })
}

struct NodeAuth {
    node_id: i64,
    cycle_ms: i64,
}

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
    ///
    async fn authenticate(&self, metadata: &MetadataMap) -> Result<NodeAuth, Status> {
        let token = bearer_token(metadata)?;
        let now = Utc::now().naive_utc();

        let conn = self.state.ingest_pool.get().await.map_err(internal)?;
        let (node_id, suspended, interval_seconds) = conn
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
                    .select((
                        nodes_schema::id,
                        nodes_schema::suspended,
                        nodes_schema::report_interval_seconds,
                    ))
                    .first::<(i64, bool, i32)>(conn)
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

        Ok(NodeAuth {
            node_id,
            cycle_ms: i64::from(interval_seconds).saturating_mul(1_000),
        })
    }

    ///
    async fn authenticate_registration(
        &self,
        metadata: &MetadataMap,
    ) -> Result<(NodeAuth, String), Status> {
        let token = bearer_token(metadata)?;
        let auth = self.authenticate(metadata).await?;

        Ok((auth, token))
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

    type WatchLiveStream = Pin<Box<dyn Stream<Item = Result<LiveCommand, Status>> + Send>>;

    /// Hold a stream open so the server can start a session on this node.
    ///
    async fn watch_live(
        &self,
        request: Request<LiveWatchRequest>,
    ) -> Result<Response<Self::WatchLiveStream>, Status> {
        let node_id = self.authenticate(request.metadata()).await?.node_id;
        validate::live_watch(request.get_ref())?;

        let sessions = Arc::clone(&self.state.live);
        let commands = sessions.watch(node_id);

        tracing::debug!(node_id, "a node is watching for live sessions");

        let stream = futures_util::stream::unfold(
            (commands, sessions, node_id),
            |(mut commands, sessions, node_id)| async move {
                let command = commands.recv().await?;

                Some((Ok(command), (commands, sessions, node_id)))
            },
        );

        Ok(Response::new(Box::pin(stream)))
    }

    /// Take one dwell from an inspected node and push it at open dashboards.
    ///
    async fn submit_live(&self, request: Request<LiveSample>) -> Result<Response<LiveAck>, Status> {
        let node_id = self.authenticate(request.metadata()).await?.node_id;
        validate::live(request.get_ref())?;

        let sample = request.into_inner();
        let session_id = sample.session_id;
        let stamp = sample
            .measured_at
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("measured_at is required"))?;
        let measured_at = persist::naive_utc(stamp.seconds, stamp.nanos, "measured_at")?.and_utc();

        if !self.state.live.accepts(node_id, session_id) {
            return Ok(Response::new(live_ack(0)));
        }

        let readings = sample
            .readings
            .iter()
            .map(|reading| Ok((persist::metric(reading.metric)?, reading.value)))
            .collect::<Result<Vec<_>, Status>>()?;

        self.state.publish_live(live::Sample {
            node_id,
            label: sample.label,
            frequency_hz: sample.frequency_hz,
            measured_at,
            readings,
        });

        Ok(Response::new(live_ack(session_id)))
    }
}

fn live_ack(session_id: u64) -> LiveAck {
    LiveAck {
        protocol_version: protocol::PROTOCOL_VERSION.to_owned(),
        server_time: Some(SystemTime::now().into()),
        session_id,
    }
}

impl Ingest {
    ///
    async fn register_node_inner(
        &self,
        request: Request<NodeRegistrationRequest>,
    ) -> Result<Response<NodeRegistrationResponse>, Status> {
        let (auth, credential) = self.authenticate_registration(request.metadata()).await?;
        let node_id = auth.node_id;

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

        let conn = self.state.ingest_pool.get().await.map_err(internal)?;

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
            protocol_version: protocol::PROTOCOL_VERSION.to_owned(),
            node_id,
            credential,
            server_time: Some(SystemTime::now().into()),
            schedule: report_schedule(node_id, Utc::now(), auth.cycle_ms),
        }))
    }

    async fn submit_measurements_inner(
        &self,
        request: Request<MeasurementReport>,
    ) -> Result<Response<IngestAck>, Status> {
        let NodeAuth { node_id, cycle_ms } = self.authenticate(request.metadata()).await?;

        validate::measurements(request.get_ref())?;
        let report = persist::PreparedReport::try_from(request.into_inner())?;

        let accepted_channels = u64::try_from(report.channel_count())
            .map_err(|_| Status::invalid_argument("the report carries too many channels"))?;

        let conn = self.state.ingest_pool.get().await.map_err(internal)?;
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
            protocol_version: protocol::PROTOCOL_VERSION.to_owned(),
            server_time: Some(SystemTime::now().into()),
            accepted_channels,
            schedule: report_schedule(node_id, Utc::now(), cycle_ms),
        }))
    }

    async fn report_health_inner(
        &self,
        request: Request<HealthReport>,
    ) -> Result<Response<HealthAck>, Status> {
        let node_id = self.authenticate(request.metadata()).await?.node_id;

        validate::health(request.get_ref())?;
        let row = persist::health_row(node_id, request.get_ref())?;

        let conn = self.state.ingest_pool.get().await.map_err(internal)?;
        let written = conn
            .interact(move |conn| persist::write_health(conn, &row))
            .await
            .map_err(internal)?
            .map_err(internal)?;

        tracing::debug!(node_id, written, "health ingested");

        Ok(Response::new(HealthAck {
            protocol_version: protocol::PROTOCOL_VERSION.to_owned(),
            server_time: Some(SystemTime::now().into()),
        }))
    }

    async fn channel_plan_inner(
        &self,
        request: Request<ChannelPlanRequest>,
    ) -> Result<Response<ChannelPlan>, Status> {
        let node_id = self.authenticate(request.metadata()).await?.node_id;

        validate::channel_plan(request.get_ref())?;
        let known = request.into_inner().known_plan_version;

        let conn = self.state.ingest_pool.get().await.map_err(internal)?;
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
            protocol_version: protocol::PROTOCOL_VERSION.to_owned(),
            plan_version: plan_version.unsigned_abs(),
            channels,
            issued_at: Some(SystemTime::now().into()),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot_ms(node_id: i64, now: DateTime<Utc>, cycle_ms: i64) -> i64 {
        delay_ms(node_id, now, cycle_ms)
            .saturating_add(now.timestamp_millis())
            .rem_euclid(cycle_ms)
    }

    fn delay_ms(node_id: i64, now: DateTime<Utc>, cycle_ms: i64) -> i64 {
        let schedule = report_schedule(node_id, now, cycle_ms).expect("a schedule");
        let at = schedule.next_report_at.expect("an instant");
        let millis = at
            .seconds
            .saturating_mul(1_000)
            .saturating_add(i64::from(at.nanos) / 1_000_000);

        millis.saturating_sub(now.timestamp_millis())
    }

    fn at(millis: i64) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(1_772_000_000_000_i64.saturating_add(millis))
            .expect("a real time")
    }

    #[test]
    fn a_node_always_lands_on_its_own_slot() {
        let expected = slot_ms(7, at(0), DEFAULT_CYCLE_MS);

        for offset in (0..DEFAULT_CYCLE_MS).step_by(997) {
            assert_eq!(slot_ms(7, at(offset), DEFAULT_CYCLE_MS), expected);
        }
    }

    #[test]
    fn a_fleet_of_sequential_ids_spreads_over_the_whole_cycle() {
        let fleet = 7_200_i64;
        let now = at(0);

        let mut slots: Vec<i64> = (1..=fleet)
            .map(|id| slot_ms(id, now, DEFAULT_CYCLE_MS))
            .collect();
        slots.sort_unstable();
        slots.dedup();

        assert_eq!(slots.len(), 7_200, "ids collided on a slot");

        let ideal = DEFAULT_CYCLE_MS / fleet;
        let widest = slots
            .windows(2)
            .filter_map(|pair| match pair {
                [before, after] => Some(after.saturating_sub(*before)),
                _ => None,
            })
            .max()
            .expect("a gap");

        assert!(
            widest <= ideal.saturating_mul(4),
            "widest gap {widest} ms against an ideal spacing of {ideal} ms"
        );
    }

    #[test]
    fn the_next_report_is_always_ahead_and_within_one_cycle() {
        for node_id in 0..500_i64 {
            let now = at(node_id.saturating_mul(37));
            let delay = delay_ms(node_id, now, DEFAULT_CYCLE_MS);

            assert!(delay > 0, "node {node_id} was told to report in the past");
            assert!(delay <= DEFAULT_CYCLE_MS, "node {node_id} waits {delay} ms");
        }
    }

    #[test]
    fn a_shorter_cycle_reports_sooner_and_still_spreads() {
        let cycle = 5_000_i64;
        let now = at(0);

        for node_id in 0..500_i64 {
            let delay = delay_ms(node_id, now, cycle);

            assert!(delay > 0, "node {node_id} was told to report in the past");
            assert!(delay <= cycle, "node {node_id} waits {delay} ms of {cycle}");
        }

        let mut slots: Vec<i64> = (1..=500_i64).map(|id| slot_ms(id, now, cycle)).collect();
        slots.sort_unstable();
        slots.dedup();

        assert_eq!(slots.len(), 500, "ids collided inside the shorter cycle");
    }

    #[test]
    fn a_cycle_of_zero_falls_back_to_the_default() {
        let now = at(1_234);

        assert_eq!(
            delay_ms(11, now, 0),
            delay_ms(11, now, DEFAULT_CYCLE_MS),
            "a zero cycle did not fall back"
        );
    }
}
