use std::time::{Duration, SystemTime, UNIX_EPOCH};

use protocol::PROTOCOL_VERSION;
use protocol::schedule_delay;
use protocol::v1::node_ingest_client::NodeIngestClient;
use protocol::v1::{
    Capabilities, ChannelPlan, ChannelPlanRequest, Hardware, HealthReport, IngestAck, LiveAck,
    LiveCommand, LiveSample, LiveWatchRequest, Location, MeasurementReport, Modulation,
    NodeRegistrationRequest,
};
use tonic::codec::CompressionEncoding;
use tonic::metadata::{MetadataMap, MetadataValue};
use tonic::transport::{Channel, ClientTlsConfig};
use tonic::{Request, Status};

use crate::config::Config;
use crate::dsp::DERIVED_METRICS;
use crate::health::Health;
use crate::identity::{self, Identity, IdentityError};

/// Why a call could not be made.
#[derive(Debug)]
pub enum ClientError {
    /// The endpoint is not a URI a channel can be built from.
    Endpoint { server: String, reason: String },
    /// The server refused a call.
    Rpc(Status),
    /// The identity could not be read or written.
    Identity(IdentityError),
    /// The credential holds bytes that cannot go in a metadata value.
    Credential,
    /// The server already knows this identity, but this node no longer holds the credential.
    LostCredential { identity: String },
    /// No enrollment token was configured, so there is nothing to register with.
    MissingEnrollmentToken,
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::Endpoint {
                ref server,
                ref reason,
            } => {
                write!(f, "'{server}' is not a usable gRPC endpoint: {reason}")
            }
            Self::Rpc(ref status) => write!(f, "the server refused the call: {status}"),
            Self::Identity(ref err) => write!(f, "{err}"),
            Self::Credential => f.write_str("the stored credential is not valid metadata"),
            Self::LostCredential { ref identity } => write!(
                f,
                "the server already knows identity '{identity}' but this node holds no credential for it; re-issue one server-side rather than re-registering, which would abandon this node's history"
            ),
            Self::MissingEnrollmentToken => f.write_str(
                "no enrollment token is configured; an administrator mints one for this node in the web client and it is passed as SPEKTRA_ENROLLMENT_TOKEN",
            ),
        }
    }
}

impl std::error::Error for ClientError {}

impl From<Status> for ClientError {
    fn from(status: Status) -> Self {
        Self::Rpc(status)
    }
}

impl From<IdentityError> for ClientError {
    fn from(err: IdentityError) -> Self {
        Self::Identity(err)
    }
}

/// A connected, authenticated node client.
pub struct Client {
    inner: NodeIngestClient<Channel>,
    credential: Option<String>,
    clock_offset_seconds: Option<f64>,
    delivery_delay: Option<Duration>,
}

impl Client {
    /// Build a client for `server` without waiting for the connection.
    ///
    /// # Errors
    ///
    /// [`ClientError::Endpoint`] if the address is not a usable URI.
    pub fn new(server: &str) -> Result<Self, ClientError> {
        let channel = Channel::from_shared(server.to_owned())
            .map_err(|err| err.to_string())
            .and_then(|endpoint| {
                endpoint
                    .tls_config(ClientTlsConfig::new().with_enabled_roots())
                    .map_err(|err| err.to_string())
            })
            .map_err(|reason| ClientError::Endpoint {
                server: server.to_owned(),
                reason,
            })?
            .connect_lazy();

        Ok(Self {
            inner: NodeIngestClient::new(channel)
                .send_compressed(CompressionEncoding::Zstd)
                .accept_compressed(CompressionEncoding::Zstd),
            credential: None,
            clock_offset_seconds: None,
            delivery_delay: None,
        })
    }

    /// The node's clock offset against the server, in seconds.
    #[must_use]
    pub const fn clock_offset_seconds(&self) -> Option<f64> {
        self.clock_offset_seconds
    }

    /// What the server last asked this node to wait before delivering.
    #[must_use]
    pub const fn delivery_delay(&self) -> Option<Duration> {
        self.delivery_delay
    }

    /// A second client over the same connection, holding the same credential.
    #[must_use]
    pub fn duplicate(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            credential: self.credential.clone(),
            clock_offset_seconds: self.clock_offset_seconds,
            delivery_delay: None,
        }
    }

    /// Open the stream the server pushes live sessions down.
    ///
    /// # Errors
    ///
    /// [`ClientError::Rpc`] if the server refuses or is unreachable.
    pub async fn watch_live(&mut self) -> Result<tonic::Streaming<LiveCommand>, ClientError> {
        let request = self.authenticated(LiveWatchRequest {
            protocol_version: PROTOCOL_VERSION.to_owned(),
        })?;

        Ok(self.inner.watch_live(request).await?.into_inner())
    }

    /// Send one dwell to an open live session.
    ///
    /// # Errors
    ///
    /// [`ClientError::Rpc`] if the server refuses it.
    pub async fn submit_live(&mut self, sample: LiveSample) -> Result<LiveAck, ClientError> {
        let request = self.authenticated(sample)?;

        Ok(self.inner.submit_live(request).await?.into_inner())
    }

    /// Load the stored identity, or register this node and store one.
    ///
    /// # Errors
    ///
    /// [`ClientError::Identity`] on state-directory failure, [`ClientError::LostCredential`] if the server already knows the identity, [`ClientError::Rpc`] otherwise.
    pub async fn register_or_load(&mut self, config: &Config) -> Result<Identity, ClientError> {
        let path = config.identity_path();

        if let Some(stored) = identity::load(&path)? {
            tracing::info!(
                node_id = stored.node_id,
                identity = %stored.identity,
                "resuming as a registered node"
            );
            self.credential = Some(stored.credential.clone());

            return Ok(stored);
        }

        let value = identity::generate();
        let request = NodeRegistrationRequest {
            protocol_version: PROTOCOL_VERSION.to_owned(),
            identity: value.clone(),
            name: config.name.clone(),
            location: Some(Location {
                latitude: config.latitude,
                longitude: config.longitude,
            }),
            hardware: Some(Hardware {
                device: config.device.device.driver().to_owned(),
                antenna: config.antenna.clone(),
                max_sample_rate_hz: u64::from(config.device.sample_rate_hz),
            }),
            capabilities: Some(Capabilities {
                metrics: DERIVED_METRICS.iter().copied().map(i32::from).collect(),
                modulations: vec![i32::from(Modulation::Fm)],
            }),
        };

        let token = config
            .enrollment_token
            .clone()
            .ok_or(ClientError::MissingEnrollmentToken)?;
        self.credential = Some(token);

        let response = self
            .inner
            .register_node(self.authenticated(request)?)
            .await
            .map_err(|status| match status.code() {
                tonic::Code::AlreadyExists => ClientError::LostCredential {
                    identity: value.clone(),
                },
                _ => ClientError::Rpc(status),
            })?
            .into_inner();

        self.observe_server_time(response.server_time.as_ref());
        self.delivery_delay =
            schedule_delay(response.schedule.as_ref(), response.server_time.as_ref());

        let registered = Identity {
            identity: value,
            node_id: response.node_id,
            credential: response.credential,
        };
        identity::store(&path, &registered)?;
        self.credential = Some(registered.credential.clone());

        tracing::info!(
            node_id = registered.node_id,
            identity = %registered.identity,
            "registered with the server"
        );

        Ok(registered)
    }

    /// Deliver one aggregated report.
    ///
    /// # Errors
    ///
    /// [`ClientError::Rpc`] if the server refuses it.
    pub async fn submit(&mut self, report: MeasurementReport) -> Result<IngestAck, ClientError> {
        let request = self.authenticated(report)?;
        let ack = self.inner.submit_measurements(request).await?.into_inner();

        self.observe_server_time(ack.server_time.as_ref());
        self.delivery_delay = schedule_delay(ack.schedule.as_ref(), ack.server_time.as_ref());

        Ok(ack)
    }

    /// Report the node's own operational state.
    ///
    /// # Errors
    ///
    /// [`ClientError::Rpc`] if the server refuses it.
    pub async fn report_health(&mut self, health: Health) -> Result<(), ClientError> {
        let report = HealthReport {
            protocol_version: PROTOCOL_VERSION.to_owned(),
            measured_at: Some(SystemTime::now().into()),
            uptime_seconds: health.uptime_seconds,
            load_1m: health.load_1m,
            load_5m: health.load_5m,
            load_15m: health.load_15m,
            cpu_temperature_celsius: health.cpu_temperature_celsius,
            clock_offset_seconds: self.clock_offset_seconds,
        };

        let request = self.authenticated(report)?;
        let ack = self.inner.report_health(request).await?.into_inner();

        self.observe_server_time(ack.server_time.as_ref());

        Ok(())
    }

    /// Ask the server which channels this node is assigned.
    ///
    /// # Errors
    ///
    /// [`ClientError::Rpc`] if the server refuses the call.
    pub async fn channel_plan(&mut self, known: u64) -> Result<ChannelPlan, ClientError> {
        let request = self.authenticated(ChannelPlanRequest {
            protocol_version: PROTOCOL_VERSION.to_owned(),
            known_plan_version: known,
        })?;

        Ok(self.inner.get_channel_plan(request).await?.into_inner())
    }

    fn authenticated<T>(&self, message: T) -> Result<Request<T>, ClientError> {
        let mut request = Request::new(message);

        if let Some(credential) = self.credential.as_deref() {
            let value: MetadataValue<_> = format!("Bearer {credential}")
                .parse()
                .map_err(|_| ClientError::Credential)?;

            let mut metadata = MetadataMap::new();
            metadata.insert("authorization", value);
            *request.metadata_mut() = metadata;
        }

        Ok(request)
    }

    fn observe_server_time(&mut self, server_time: Option<&prost_types::Timestamp>) {
        let Some(stamp) = server_time else {
            return;
        };

        let Ok(node_since_epoch) = SystemTime::now().duration_since(UNIX_EPOCH) else {
            return;
        };

        let Ok(nanos) = u32::try_from(stamp.nanos) else {
            return;
        };

        let Ok(seconds) = u64::try_from(stamp.seconds) else {
            return;
        };

        let server_since_epoch = Duration::new(seconds, nanos);
        self.clock_offset_seconds =
            Some(node_since_epoch.as_secs_f64() - server_since_epoch.as_secs_f64());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_derived_metric_has_an_accepted_range() {
        for metric in DERIVED_METRICS {
            assert!(
                protocol::metric_range(*metric).is_some(),
                "{metric:?} is derived but has no accepted range"
            );
        }
    }

    #[test]
    fn a_bad_endpoint_is_rejected_before_any_call() {
        assert!(matches!(
            Client::new("not a uri at all"),
            Err(ClientError::Endpoint { .. })
        ));
    }
}
