use protocol::v1::{
    ChannelPlanRequest, HealthReport, MeasurementReport, Metric, Modulation,
    NodeRegistrationRequest, SampleStats,
};
use tonic::Status;

const MAX_WINDOW_SECONDS: i64 = 86_400;
const TEMPERATURE_RANGE: std::ops::RangeInclusive<f64> = -40.0..=150.0;
const CLOCK_OFFSET_RANGE: std::ops::RangeInclusive<f64> = -3_600.0..=3_600.0;

fn check_version(version: &str) -> Result<(), Status> {
    let expected = protocol::PROTOCOL_VERSION;
    if version == expected {
        Ok(())
    } else {
        Err(Status::invalid_argument(format!(
            "unsupported protocol_version '{version}', expected '{expected}'"
        )))
    }
}

/// Validate a registration request.
///
/// # Errors
///
/// Returns [`Status::invalid_argument`] when the request cannot be accepted.
pub fn registration(req: &NodeRegistrationRequest) -> Result<(), Status> {
    check_version(&req.protocol_version)?;

    if req.identity.is_empty() {
        return Err(Status::invalid_argument("identity must not be empty"));
    }
    if req.name.is_empty() {
        return Err(Status::invalid_argument("name must not be empty"));
    }

    match &req.location {
        Some(location) => {
            if !(-90.0..=90.0).contains(&location.latitude) {
                return Err(Status::invalid_argument(
                    "latitude must be within -90 to 90 degrees",
                ));
            }

            if !(-180.0..=180.0).contains(&location.longitude) {
                return Err(Status::invalid_argument(
                    "longitude must be within -180 to 180 degrees",
                ));
            }
        }
        None => return Err(Status::invalid_argument("location is required")),
    }

    match &req.hardware {
        Some(hardware) => {
            if hardware.device.is_empty() {
                return Err(Status::invalid_argument(
                    "hardware.device must not be empty",
                ));
            }
        }
        None => return Err(Status::invalid_argument("hardware is required")),
    }

    match &req.capabilities {
        Some(capabilities) => {
            if capabilities.metrics.is_empty() {
                return Err(Status::invalid_argument(
                    "capabilities must list at least one metric",
                ));
            }
            if capabilities
                .metrics
                .contains(&i32::from(Metric::Unspecified))
            {
                return Err(Status::invalid_argument(
                    "capabilities.metrics contains an unspecified metric",
                ));
            }
        }
        None => return Err(Status::invalid_argument("capabilities is required")),
    }

    Ok(())
}

/// Validate one measurement report.
///
/// # Errors
///
/// Returns [`Status::invalid_argument`] when the report or any reading cannot be accepted.
pub fn measurements(report: &MeasurementReport) -> Result<(), Status> {
    check_version(&report.protocol_version)?;

    let Some(start) = &report.window_start else {
        return Err(Status::invalid_argument("window_start is required"));
    };
    let Some(end) = &report.window_end else {
        return Err(Status::invalid_argument("window_end is required"));
    };
    if !(end.seconds > start.seconds || (end.seconds == start.seconds && end.nanos > start.nanos)) {
        return Err(Status::invalid_argument(
            "window_end must be after window_start",
        ));
    }
    let Some(window_seconds) = end.seconds.checked_sub(start.seconds) else {
        return Err(Status::invalid_argument("window bounds overflow"));
    };
    if window_seconds > MAX_WINDOW_SECONDS {
        return Err(Status::invalid_argument(
            "aggregation window must not exceed 24 hours",
        ));
    }

    if report.channels.is_empty() {
        return Err(Status::invalid_argument(
            "report must contain at least one channel",
        ));
    }

    for channel in &report.channels {
        check_band(channel.frequency_hz, channel.modulation)?;

        if channel.readings.is_empty() {
            return Err(Status::invalid_argument(
                "each channel must carry at least one reading",
            ));
        }
        for reading in &channel.readings {
            validate_stats(reading.metric, reading.stats.as_ref())?;
        }
    }

    Ok(())
}

/// Validate a health report.
///
/// # Errors
///
/// Returns [`Status::invalid_argument`] when the report cannot be accepted.
pub fn health(report: &HealthReport) -> Result<(), Status> {
    check_version(&report.protocol_version)?;

    if report.measured_at.is_none() {
        return Err(Status::invalid_argument("measured_at is required"));
    }
    for (name, value) in [
        ("uptime_seconds", report.uptime_seconds),
        ("load_1m", report.load_1m),
        ("load_5m", report.load_5m),
        ("load_15m", report.load_15m),
    ] {
        if value.is_some_and(|value| !value.is_finite() || value < 0.0) {
            return Err(Status::invalid_argument(format!(
                "{name} must be finite and non-negative"
            )));
        }
    }
    if report
        .cpu_temperature_celsius
        .is_some_and(|celsius| !TEMPERATURE_RANGE.contains(&celsius))
    {
        return Err(Status::invalid_argument(format!(
            "cpu_temperature_celsius must be within {} to {}",
            TEMPERATURE_RANGE.start(),
            TEMPERATURE_RANGE.end()
        )));
    }
    if report
        .clock_offset_seconds
        .is_some_and(|offset| !CLOCK_OFFSET_RANGE.contains(&offset))
    {
        return Err(Status::invalid_argument(format!(
            "clock_offset_seconds must be within {} to {}",
            CLOCK_OFFSET_RANGE.start(),
            CLOCK_OFFSET_RANGE.end()
        )));
    }

    Ok(())
}

/// Validate a channel plan request.
///
/// # Errors
///
/// Returns [`Status::invalid_argument`] when the protocol version does not match.
pub fn channel_plan(req: &ChannelPlanRequest) -> Result<(), Status> {
    check_version(&req.protocol_version)
}

fn validate_stats(metric: i32, stats: Option<&SampleStats>) -> Result<(), Status> {
    let Some(stats) = stats else {
        return Err(Status::invalid_argument("stats are required"));
    };

    for value in [
        stats.min,
        stats.max,
        stats.mean,
        stats.median,
        stats.stddev,
        stats.p95,
    ] {
        if !value.is_finite() {
            return Err(Status::invalid_argument("stats must be finite"));
        }
    }
    if stats.min > stats.max {
        return Err(Status::invalid_argument(
            "stats.min must not exceed stats.max",
        ));
    }
    if stats.mean < stats.min || stats.mean > stats.max {
        return Err(Status::invalid_argument(
            "stats.mean must lie within min and max",
        ));
    }
    if stats.median < stats.min || stats.median > stats.max {
        return Err(Status::invalid_argument(
            "stats.median must lie within min and max",
        ));
    }
    if stats.p95 < stats.min || stats.p95 > stats.max {
        return Err(Status::invalid_argument(
            "stats.p95 must lie within min and max",
        ));
    }
    if stats.stddev < 0.0 {
        return Err(Status::invalid_argument(
            "stats.stddev must be non-negative",
        ));
    }
    if stats.sample_count == 0 {
        return Err(Status::invalid_argument(
            "stats.sample_count must be at least 1",
        ));
    }

    let range = metric_range(metric)?;
    if !range.contains(&stats.min) || !range.contains(&stats.max) {
        return Err(Status::invalid_argument(format!(
            "metric values must lie within {} to {}",
            range.start(),
            range.end()
        )));
    }

    Ok(())
}

fn check_band(frequency_hz: u64, modulation: i32) -> Result<(), Status> {
    if frequency_hz == 0 {
        return Err(Status::invalid_argument("frequency_hz must not be zero"));
    }

    match Modulation::try_from(modulation) {
        Ok(Modulation::Fm) if (87_500_000..=108_000_000).contains(&frequency_hz) => Ok(()),
        Ok(Modulation::Fm) => Err(Status::invalid_argument(format!(
            "frequency {frequency_hz} Hz is outside the FM broadcast band"
        ))),
        Ok(Modulation::Dab) if (174_000_000..=240_000_000).contains(&frequency_hz) => Ok(()),
        Ok(Modulation::Dab) => Err(Status::invalid_argument(format!(
            "frequency {frequency_hz} Hz is outside the DAB band"
        ))),
        Ok(Modulation::Unspecified) | Err(_) => {
            Err(Status::invalid_argument("modulation must be FM or DAB"))
        }
    }
}

/// The physically meaningful range one metric is accepted in.
///
/// # Errors
///
/// Returns [`Status::invalid_argument`] for a number that is not a known metric.
pub fn metric_range(metric: i32) -> Result<std::ops::RangeInclusive<f64>, Status> {
    Metric::try_from(metric)
        .ok()
        .and_then(protocol::metric_range)
        .ok_or_else(|| Status::invalid_argument("metric must be a known metric"))
}

/// Reject a live watch request that is not for this protocol version.
///
/// # Errors
///
/// Returns [`Status::invalid_argument`] for any other version.
pub fn live_watch(request: &protocol::v1::LiveWatchRequest) -> Result<(), Status> {
    check_version(&request.protocol_version)
}

/// Reject a live sample that is not a plausible reading of a real channel.
///
/// # Errors
///
/// Returns [`Status::invalid_argument`] for a bad version, band, empty readings, or out-of-range value.
pub fn live(sample: &protocol::v1::LiveSample) -> Result<(), Status> {
    check_version(&sample.protocol_version)?;
    check_band(sample.frequency_hz, sample.modulation)?;

    if sample.readings.is_empty() {
        return Err(Status::invalid_argument(
            "a live sample carries no readings",
        ));
    }

    for reading in &sample.readings {
        let range = metric_range(reading.metric)?;
        if !reading.value.is_finite() || !range.contains(&reading.value) {
            return Err(Status::invalid_argument(format!(
                "metric values must lie within {} to {}",
                range.start(),
                range.end()
            )));
        }
    }

    Ok(())
}
