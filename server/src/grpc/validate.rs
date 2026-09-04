use protocol::v1::{
    ChannelPlanRequest, HealthReport, MeasurementReport, Metric, Modulation,
    NodeRegistrationRequest, SampleStats,
};
use tonic::Status;

pub const PROTOCOL_VERSION: &str = "1";

const MAX_WINDOW_SECONDS: i64 = 86_400;

const SNR_RANGE: std::ops::RangeInclusive<f64> = 0.0..=80.0;

const CARRIER_OFFSET_RANGE: std::ops::RangeInclusive<f64> = -200_000.0..=200_000.0;

const TEMPERATURE_RANGE: std::ops::RangeInclusive<f64> = -40.0..=150.0;

const CLOCK_OFFSET_RANGE: std::ops::RangeInclusive<f64> = -3_600.0..=3_600.0;

fn check_version(version: &str) -> Result<(), Status> {
    if version == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(Status::invalid_argument(format!(
            "unsupported protocol_version '{version}', expected '{PROTOCOL_VERSION}'"
        )))
    }
}

/// Validate a registration request.
///
/// # Errors
///
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
        if channel.frequency_hz == 0 {
            return Err(Status::invalid_argument("frequency_hz must not be zero"));
        }
        match Modulation::try_from(channel.modulation) {
            Ok(Modulation::Fm) => {
                if !(87_500_000..=108_000_000).contains(&channel.frequency_hz) {
                    return Err(Status::invalid_argument(format!(
                        "frequency {} Hz is outside the FM broadcast band",
                        channel.frequency_hz
                    )));
                }
            }
            Ok(Modulation::Dab) => {
                if !(174_000_000..=240_000_000).contains(&channel.frequency_hz) {
                    return Err(Status::invalid_argument(format!(
                        "frequency {} Hz is outside the DAB band",
                        channel.frequency_hz
                    )));
                }
            }
            Ok(Modulation::Unspecified) | Err(_) => {
                return Err(Status::invalid_argument("modulation must be FM or DAB"));
            }
        }

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
pub fn health(report: &HealthReport) -> Result<(), Status> {
    check_version(&report.protocol_version)?;

    if report.measured_at.is_none() {
        return Err(Status::invalid_argument("measured_at is required"));
    }
    if !report.uptime_seconds.is_finite() || report.uptime_seconds < 0.0 {
        return Err(Status::invalid_argument(
            "uptime_seconds must be finite and non-negative",
        ));
    }
    for (name, load) in [
        ("load_1m", report.load_1m),
        ("load_5m", report.load_5m),
        ("load_15m", report.load_15m),
    ] {
        if !load.is_finite() || load < 0.0 {
            return Err(Status::invalid_argument(format!(
                "{name} must be finite and non-negative"
            )));
        }
    }
    if !TEMPERATURE_RANGE.contains(&report.cpu_temperature_celsius) {
        return Err(Status::invalid_argument(format!(
            "cpu_temperature_celsius must be within {} to {}",
            TEMPERATURE_RANGE.start(),
            TEMPERATURE_RANGE.end()
        )));
    }
    if !CLOCK_OFFSET_RANGE.contains(&report.clock_offset_seconds) {
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
///
/// # Errors
///
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

    let range = match Metric::try_from(metric) {
        Ok(Metric::SignalStrength) => Some(-150.0..=0.0),
        Ok(Metric::SignalToNoise) => Some(SNR_RANGE.clone()),
        Ok(Metric::CarrierOffset) => Some(CARRIER_OFFSET_RANGE.clone()),
        Ok(Metric::DemodErrorRate | Metric::SpectrumOccupancy) => Some(0.0..=1.0),
        Ok(Metric::Unspecified) | Err(_) => None,
    };

    let Some(range) = range else {
        return Err(Status::invalid_argument("metric must be a known metric"));
    };

    if !range.contains(&stats.min) || !range.contains(&stats.max) {
        return Err(Status::invalid_argument(format!(
            "metric values must lie within {} to {}",
            range.start(),
            range.end()
        )));
    }

    Ok(())
}
