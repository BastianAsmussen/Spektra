use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;

use crate::source::{Device, DeviceConfig};

const SERVER_ENV: &str = "SPEKTRA_SERVER";
const STATE_DIR_ENV: &str = "SPEKTRA_STATE_DIR";
const ENROLLMENT_TOKEN_ENV: &str = "SPEKTRA_ENROLLMENT_TOKEN";
const DEFAULT_SERVER: &str = "http://localhost:50051";
const DEFAULT_STATE_DIR: &str = "spektra-state";

/// Which receiver the agent drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Receiver {
    /// RTL-SDR Blog V4-class dongle.
    RtlSdr,
    /// Airspy Mini.
    AirspyMini,
    /// No hardware: an in-process generator of FM-modulated IQ.
    Synthetic,
}

/// Command-line flags; each has an env fallback or a default.
#[derive(Debug, Parser)]
#[command(author, version, about)]
pub struct Args {
    /// gRPC endpoint of the central server.
    #[arg(short, long, env = SERVER_ENV, default_value = DEFAULT_SERVER)]
    pub server: String,

    /// Directory holding the node identity, credential and pending reports.
    #[arg(long, env = STATE_DIR_ENV, default_value = DEFAULT_STATE_DIR)]
    pub state_dir: PathBuf,

    /// Human-readable node name, sent at registration.
    #[arg(short, long, default_value = "spektra-node")]
    pub name: String,

    /// Enrollment token minted for this node by an administrator.
    #[arg(long, env = ENROLLMENT_TOKEN_ENV, hide_env_values = true)]
    pub enrollment_token: Option<String>,

    /// Receiver family to drive.
    #[arg(long, value_enum, default_value_t = Receiver::RtlSdr)]
    pub receiver: Receiver,

    /// Sample rate in samples per second.
    #[arg(long, default_value_t = 2_400_000)]
    pub sample_rate_hz: u32,

    /// Fixed receiver gain in dB. Omit to leave the driver's AGC in charge.
    #[arg(long)]
    pub gain_db: Option<f64>,

    /// WGS84 latitude of the antenna, in decimal degrees.
    #[arg(long, default_value_t = 57.048)]
    pub latitude: f64,

    /// WGS84 longitude of the antenna, in decimal degrees.
    #[arg(long, default_value_t = 9.921)]
    pub longitude: f64,

    /// Antenna description, sent at registration.
    #[arg(long, default_value = "fixed dipole")]
    pub antenna: String,

    /// Seconds spent on each channel before moving to the next.
    #[arg(long, default_value_t = 1.0)]
    pub dwell_seconds: f64,

    /// Seconds of samples aggregated into one report.
    #[arg(long, default_value_t = 60)]
    pub window_seconds: u64,

    /// Seconds between health reports.
    #[arg(long, default_value_t = 60)]
    pub health_interval_seconds: u64,

    /// Seconds between channel plan polls.
    #[arg(long, default_value_t = 300)]
    pub plan_interval_seconds: u64,

    /// Transform length for the spectrum estimate. Must be a power of two.
    #[arg(long, default_value_t = 32_768)]
    pub fft_size: usize,

    /// Largest the offline report buffer may grow, in bytes.
    #[arg(long, default_value_t = 64 * 1024 * 1024)]
    pub buffer_limit_bytes: u64,
}

/// Validated configuration, derived from [`Args`].
#[derive(Debug, Clone)]
pub struct Config {
    pub server: String,
    pub state_dir: PathBuf,
    pub name: String,
    pub enrollment_token: Option<String>,
    pub receiver: Receiver,
    pub device: DeviceConfig,
    pub latitude: f64,
    pub longitude: f64,
    pub antenna: String,
    pub dwell: Duration,
    pub window: Duration,
    pub health_interval: Duration,
    pub plan_interval: Duration,
    pub fft_size: usize,
    pub buffer_limit_bytes: u64,
}

/// Why a configuration cannot be used.
#[derive(Debug)]
pub enum ConfigError {
    /// The transform length is not a power of two, which the FFT rejects.
    FftSizeNotPowerOfTwo { size: usize },
    /// A duration was given as zero or as something not representable.
    NonPositiveDuration { field: &'static str, value: f64 },
    /// The antenna location is not a point on the globe.
    LocationOutOfRange { latitude: f64, longitude: f64 },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::FftSizeNotPowerOfTwo { size } => {
                write!(f, "fft-size {size} is not a power of two")
            }
            Self::NonPositiveDuration { field, value } => {
                write!(
                    f,
                    "{field} must be a positive number of seconds, got {value}"
                )
            }
            Self::LocationOutOfRange {
                latitude,
                longitude,
            } => write!(
                f,
                "{latitude}, {longitude} is not a point on the globe; latitude is -90 to 90 and longitude -180 to 180"
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

impl TryFrom<Args> for Config {
    type Error = ConfigError;

    fn try_from(args: Args) -> Result<Self, Self::Error> {
        if !args.fft_size.is_power_of_two() {
            return Err(ConfigError::FftSizeNotPowerOfTwo {
                size: args.fft_size,
            });
        }
        if !args.dwell_seconds.is_finite() || args.dwell_seconds <= 0.0 {
            return Err(ConfigError::NonPositiveDuration {
                field: "dwell-seconds",
                value: args.dwell_seconds,
            });
        }
        if !(-90.0..=90.0).contains(&args.latitude) || !(-180.0..=180.0).contains(&args.longitude) {
            return Err(ConfigError::LocationOutOfRange {
                latitude: args.latitude,
                longitude: args.longitude,
            });
        }

        let device = DeviceConfig {
            device: match args.receiver {
                Receiver::RtlSdr | Receiver::Synthetic => Device::RtlSdr,
                Receiver::AirspyMini => Device::AirspyMini,
            },
            sample_rate_hz: args.sample_rate_hz,
            gain_db: args.gain_db,
        };

        Ok(Self {
            server: args.server,
            state_dir: args.state_dir,
            name: args.name,
            enrollment_token: args.enrollment_token,
            receiver: args.receiver,
            device,
            latitude: args.latitude,
            longitude: args.longitude,
            antenna: args.antenna,
            dwell: Duration::from_secs_f64(args.dwell_seconds),
            window: Duration::from_secs(args.window_seconds.max(1)),
            health_interval: Duration::from_secs(args.health_interval_seconds.max(1)),
            plan_interval: Duration::from_secs(args.plan_interval_seconds.max(1)),
            fft_size: args.fft_size,
            buffer_limit_bytes: args.buffer_limit_bytes,
        })
    }
}

impl Config {
    /// Parse the command line, with the environment as the fallback.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] when a value cannot be used.
    pub fn from_args() -> Result<Self, ConfigError> {
        Self::try_from(Args::parse())
    }

    /// Path of the file holding the node identity and its credential.
    #[must_use]
    pub fn identity_path(&self) -> PathBuf {
        self.state_dir.join("identity.json")
    }

    /// Path of the cached channel plan.
    #[must_use]
    pub fn plan_path(&self) -> PathBuf {
        self.state_dir.join("plan.pb")
    }

    /// Path of the offline report journal.
    #[must_use]
    pub fn pending_path(&self) -> PathBuf {
        self.state_dir.join("pending.pb")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> Args {
        Args::parse_from(["node-agent"])
    }

    #[test]
    fn defaults_parse_with_no_arguments() {
        let config = Config::try_from(args()).expect("the defaults are usable");

        assert_eq!(config.window, Duration::from_mins(1));
        assert!(config.fft_size.is_power_of_two());
    }

    #[test]
    fn rejects_a_transform_length_the_fft_cannot_plan() {
        let mut args = args();
        args.fft_size = 6000;

        match Config::try_from(args) {
            Err(ConfigError::FftSizeNotPowerOfTwo { size }) => assert_eq!(size, 6000),
            other => panic!("expected a power-of-two rejection, got {other:?}"),
        }
    }

    #[test]
    fn rejects_a_location_off_the_globe() {
        let mut args = args();
        args.latitude = 91.0;

        assert!(matches!(
            Config::try_from(args),
            Err(ConfigError::LocationOutOfRange { .. })
        ));
    }

    #[test]
    fn a_zero_dwell_is_not_a_dwell() {
        let mut args = args();
        args.dwell_seconds = 0.0;

        assert!(matches!(
            Config::try_from(args),
            Err(ConfigError::NonPositiveDuration { .. })
        ));
    }
}
