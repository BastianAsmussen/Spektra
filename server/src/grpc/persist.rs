use std::collections::HashMap;

use chrono::{DateTime, NaiveDateTime, TimeDelta, Utc};
use diesel::pg::PgConnection;
use diesel::prelude::*;
use protocol::v1 as wire;
use tonic::Status;

use crate::db::models::channels::NewChannel;
use crate::db::models::enums::{Metric, Modulation};
use crate::db::models::measurements::NewMeasurement;
use crate::db::models::node_health::NewNodeHealth;
use crate::db::schema::channels as channels_schema;
use crate::db::schema::measurements as measurements_schema;
use crate::db::schema::node_channels as node_channels_schema;
use crate::db::schema::node_health as node_health_schema;
use crate::db::schema::nodes as nodes_schema;

const MAX_LABEL_CHARS: usize = 100;

struct PreparedReading {
    metric: Metric,
    min: f64,
    max: f64,
    mean: f64,
    median: f64,
    stddev: f64,
    p95: f64,
    sample_count: i64,
}

struct PreparedChannel {
    name: String,
    frequency_hz: i64,
    modulation: Modulation,
    readings: Vec<PreparedReading>,
}

/// A measurement report with every field converted to a storage type.
pub struct PreparedReport {
    window_start: NaiveDateTime,
    window_end: NaiveDateTime,
    channels: Vec<PreparedChannel>,
}

impl TryFrom<wire::MeasurementReport> for PreparedReport {
    type Error = Status;

    fn try_from(report: wire::MeasurementReport) -> Result<Self, Self::Error> {
        let start = report
            .window_start
            .ok_or_else(|| Status::invalid_argument("window_start is required"))?;
        let end = report
            .window_end
            .ok_or_else(|| Status::invalid_argument("window_end is required"))?;

        let channels = report
            .channels
            .into_iter()
            .map(PreparedChannel::try_from)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            window_start: naive_utc(start.seconds, start.nanos, "window_start")?,
            window_end: naive_utc(end.seconds, end.nanos, "window_end")?,
            channels,
        })
    }
}

impl TryFrom<wire::ChannelMeasurement> for PreparedChannel {
    type Error = Status;

    fn try_from(channel: wire::ChannelMeasurement) -> Result<Self, Self::Error> {
        let frequency_hz = i64::try_from(channel.frequency_hz).map_err(|_| {
            Status::invalid_argument(format!(
                "frequency {} Hz does not fit in a signed 64-bit integer",
                channel.frequency_hz
            ))
        })?;

        if channel.label.chars().count() > MAX_LABEL_CHARS {
            return Err(Status::invalid_argument(format!(
                "label must not exceed {MAX_LABEL_CHARS} characters"
            )));
        }
        let name = if channel.label.is_empty() {
            format!("{frequency_hz} Hz")
        } else {
            channel.label
        };

        let readings = channel
            .readings
            .into_iter()
            .map(PreparedReading::try_from)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            name,
            frequency_hz,
            modulation: modulation(channel.modulation)?,
            readings,
        })
    }
}

impl TryFrom<wire::MetricReading> for PreparedReading {
    type Error = Status;

    fn try_from(reading: wire::MetricReading) -> Result<Self, Self::Error> {
        let stats = reading
            .stats
            .ok_or_else(|| Status::invalid_argument("stats are required"))?;

        Ok(Self {
            metric: metric(reading.metric)?,
            min: stats.min,
            max: stats.max,
            mean: stats.mean,
            median: stats.median,
            stddev: stats.stddev,
            p95: stats.p95,
            sample_count: i64::try_from(stats.sample_count).map_err(|_| {
                Status::invalid_argument("sample_count does not fit in a signed 64-bit integer")
            })?,
        })
    }
}

impl PreparedReport {
    /// Number of channel measurements the report carries.
    #[must_use]
    pub const fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// Persist the report and mark the node as seen, in one transaction.
    ///
    /// # Errors
    ///
    /// Returns the diesel error if any statement fails.
    pub fn write(&self, conn: &mut PgConnection, node_id: i64) -> QueryResult<usize> {
        conn.transaction(|conn| {
            let channel_ids = self.resolve_channels(conn)?;
            let rows = self.rows(node_id, &channel_ids);

            let written = diesel::insert_into(measurements_schema::table)
                .values(&rows)
                .on_conflict_do_nothing()
                .execute(conn)?;

            touch_node(conn, node_id)?;

            Ok(written)
        })
    }

    fn resolve_channels(&self, conn: &mut PgConnection) -> QueryResult<Vec<i64>> {
        let frequencies: Vec<i64> = self
            .channels
            .iter()
            .map(|channel| channel.frequency_hz)
            .collect();

        let mut known = load_channel_ids(conn, &frequencies)?;

        let unknown: Vec<NewChannel> = self
            .channels
            .iter()
            .filter(|channel| !known.contains_key(&(channel.frequency_hz, channel.modulation)))
            .map(|channel| NewChannel {
                name: channel.name.clone(),
                frequency_hz: channel.frequency_hz,
                modulation: channel.modulation,
            })
            .collect();

        if !unknown.is_empty() {
            diesel::insert_into(channels_schema::table)
                .values(&unknown)
                .on_conflict_do_nothing()
                .execute(conn)?;
            known = load_channel_ids(conn, &frequencies)?;
        }

        self.channels
            .iter()
            .map(|channel| {
                known
                    .get(&(channel.frequency_hz, channel.modulation))
                    .copied()
                    .ok_or(diesel::result::Error::NotFound)
            })
            .collect()
    }

    fn rows(&self, node_id: i64, channel_ids: &[i64]) -> Vec<NewMeasurement> {
        self.channels
            .iter()
            .zip(channel_ids)
            .flat_map(|(channel, &channel_id)| {
                channel.readings.iter().map(move |reading| NewMeasurement {
                    node_id,
                    channel_id,
                    metric: reading.metric,
                    window_start: self.window_start,
                    window_end: self.window_end,
                    min: reading.min,
                    max: reading.max,
                    mean: reading.mean,
                    median: reading.median,
                    stddev: reading.stddev,
                    p95: reading.p95,
                    sample_count: reading.sample_count,
                })
            })
            .collect()
    }
}

/// Build the row for one accepted health report.
///
/// # Errors
///
/// Returns [`Status::invalid_argument`] if `measured_at` is missing or not representable.
pub fn health_row(node_id: i64, report: &wire::HealthReport) -> Result<NewNodeHealth, Status> {
    let measured_at = report
        .measured_at
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("measured_at is required"))?;

    Ok(NewNodeHealth {
        node_id,
        measured_at: naive_utc(measured_at.seconds, measured_at.nanos, "measured_at")?,
        uptime_seconds: report.uptime_seconds,
        load_1m: report.load_1m,
        load_5m: report.load_5m,
        load_15m: report.load_15m,
        cpu_temperature_celsius: report.cpu_temperature_celsius,
        clock_offset_seconds: report.clock_offset_seconds,
    })
}

/// Persist a health report and mark the node as seen, in one transaction.
///
/// # Errors
///
/// Returns the diesel error if any statement fails.
pub fn write_health(conn: &mut PgConnection, row: &NewNodeHealth) -> QueryResult<usize> {
    let node_id = row.node_id;
    conn.transaction(|conn| {
        let written = diesel::insert_into(node_health_schema::table)
            .values(row)
            .on_conflict_do_nothing()
            .execute(conn)?;

        touch_node(conn, node_id)?;

        Ok(written)
    })
}

fn load_channel_ids(
    conn: &mut PgConnection,
    frequencies: &[i64],
) -> QueryResult<HashMap<(i64, Modulation), i64>> {
    Ok(channels_schema::table
        .filter(channels_schema::frequency_hz.eq_any(frequencies))
        .select((
            channels_schema::frequency_hz,
            channels_schema::modulation,
            channels_schema::id,
        ))
        .load::<(i64, Modulation, i64)>(conn)?
        .into_iter()
        .map(|(frequency_hz, modulation, id)| ((frequency_hz, modulation), id))
        .collect())
}

/// One assignment, as the plan serves it.
pub struct PlannedChannel {
    pub frequency_hz: i64,
    pub modulation: wire::Modulation,
    pub label: String,
    pub bandwidth_hz: Option<i32>,
}

/// Load a node's channel plan and the version it was issued at.
///
/// # Errors
///
/// Returns the diesel error if either query fails.
pub fn channel_plan(
    conn: &mut PgConnection,
    node_id: i64,
) -> QueryResult<(i64, Vec<PlannedChannel>)> {
    conn.transaction(|conn| {
        let plan_version: i64 = nodes_schema::table
            .filter(nodes_schema::id.eq(node_id))
            .select(nodes_schema::channel_plan_version)
            .first(conn)?;

        let assignments = node_channels_schema::table
            .inner_join(channels_schema::table)
            .filter(node_channels_schema::node_id.eq(node_id))
            .select((
                channels_schema::frequency_hz,
                channels_schema::modulation,
                channels_schema::name,
                node_channels_schema::bandwidth_hz,
            ))
            .order(channels_schema::frequency_hz.asc())
            .load::<(i64, Modulation, String, Option<i32>)>(conn)?
            .into_iter()
            .map(
                |(frequency_hz, modulation, label, bandwidth_hz)| PlannedChannel {
                    frequency_hz,
                    modulation: wire_modulation(modulation),
                    label,
                    bandwidth_hz,
                },
            )
            .collect();

        Ok((plan_version, assignments))
    })
}

const fn wire_modulation(value: Modulation) -> wire::Modulation {
    match value {
        Modulation::Fm => wire::Modulation::Fm,
        Modulation::Dab => wire::Modulation::Dab,
    }
}

const TOUCH_AFTER_SECONDS: i64 = 5;

fn touch_node(conn: &mut PgConnection, node_id: i64) -> QueryResult<usize> {
    let now = Utc::now().naive_utc();
    let stale = now
        .checked_sub_signed(TimeDelta::seconds(TOUCH_AFTER_SECONDS))
        .unwrap_or(now);

    diesel::update(
        nodes_schema::table
            .filter(nodes_schema::id.eq(node_id))
            .filter(
                nodes_schema::last_seen_at
                    .is_null()
                    .or(nodes_schema::last_seen_at.lt(stale)),
            ),
    )
    .set(nodes_schema::last_seen_at.eq(now))
    .execute(conn)
}

/// Translate a protobuf timestamp into the storage type.
///
/// # Errors
///
/// Returns [`Status::invalid_argument`] if the stamp is negative or not representable.
pub fn naive_utc(seconds: i64, nanos: i32, field: &str) -> Result<NaiveDateTime, Status> {
    let nanos = u32::try_from(nanos)
        .map_err(|_| Status::invalid_argument(format!("{field}.nanos must not be negative")))?;

    DateTime::from_timestamp(seconds, nanos)
        .map(|instant| instant.naive_utc())
        .ok_or_else(|| Status::invalid_argument(format!("{field} is not a representable instant")))
}

/// Translate a wire metric onto its stored counterpart.
///
/// # Errors
///
/// Returns [`Status::invalid_argument`] for a number that is not a known metric.
pub fn metric(value: i32) -> Result<Metric, Status> {
    match wire::Metric::try_from(value) {
        Ok(wire::Metric::SignalStrength) => Ok(Metric::SignalStrength),
        Ok(wire::Metric::SignalToNoise) => Ok(Metric::SignalToNoise),
        Ok(wire::Metric::CarrierOffset) => Ok(Metric::CarrierOffset),
        Ok(wire::Metric::DemodErrorRate) => Ok(Metric::DemodErrorRate),
        Ok(wire::Metric::SpectrumOccupancy) => Ok(Metric::SpectrumOccupancy),
        Ok(wire::Metric::Unspecified) | Err(_) => Err(Status::invalid_argument(format!(
            "metric {value} is not a known metric"
        ))),
    }
}

fn modulation(value: i32) -> Result<Modulation, Status> {
    match wire::Modulation::try_from(value) {
        Ok(wire::Modulation::Fm) => Ok(Modulation::Fm),
        Ok(wire::Modulation::Dab) => Ok(Modulation::Dab),
        Ok(wire::Modulation::Unspecified) | Err(_) => Err(Status::invalid_argument(format!(
            "modulation {value} is not a known modulation"
        ))),
    }
}
