use std::fmt::{self, Display};

use num_complex::Complex32;
use soapysdr::{Direction::Rx, Error as SoapyError, ErrorCode, RxStream};

use crate::dsp::convert;

const READ_TIMEOUT_US: i64 = 1_000_000;

/// Receiver families the agent supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    /// RTL-SDR Blog V4-class dongle, up to 2.4 MSPS.
    RtlSdr,
    /// Airspy Mini, up to 6 MSPS.
    AirspyMini,
}

impl Device {
    /// The `SoapySDR` driver key that selects this family.
    #[must_use]
    pub const fn driver(self) -> &'static str {
        match self {
            Self::RtlSdr => "rtlsdr",
            Self::AirspyMini => "airspy",
        }
    }

    #[must_use]
    pub const fn max_sample_rate_hz(self) -> u32 {
        match self {
            Self::RtlSdr => 2_400_000,
            Self::AirspyMini => 6_000_000,
        }
    }
}

impl Display for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.driver())
    }
}

/// Receiver configuration for one sampling session.
#[derive(Debug, Clone, Copy)]
pub struct DeviceConfig {
    pub device: Device,
    pub sample_rate_hz: u32,
    /// Receiver gain in dB, or `None` for the driver's AGC.
    pub gain_db: Option<f64>,
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            device: Device::RtlSdr,
            sample_rate_hz: 2_400_000,
            gain_db: Some(30.0),
        }
    }
}

/// Why the front end could not be opened or read.
#[derive(Debug)]
pub enum SourceError {
    /// No device of the configured family is attached.
    NoDevice { device: Device },
    /// The configured sample rate is above what the family supports.
    SampleRateTooHigh { requested: u32, maximum: u32 },
    /// The driver dropped samples.
    Overflow,
    /// The requested frequency is outside what a tuner can be asked for.
    FrequencyOutOfRange { frequency_hz: u64 },
    /// `SoapySDR` refused a call.
    Soapy(SoapyError),
}

impl Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NoDevice { device } => write!(f, "no {device} device is attached"),
            Self::SampleRateTooHigh { requested, maximum } => write!(
                f,
                "{requested} S/s is above the {maximum} S/s this receiver supports"
            ),
            Self::Overflow => f.write_str("the receiver overflowed and samples were lost"),
            Self::FrequencyOutOfRange { frequency_hz } => {
                write!(f, "{frequency_hz} Hz is not a tunable frequency")
            }
            Self::Soapy(ref err) => write!(f, "SoapySDR: {err}"),
        }
    }
}

impl std::error::Error for SourceError {}

impl From<SoapyError> for SourceError {
    fn from(err: SoapyError) -> Self {
        if err.code == ErrorCode::Overflow {
            Self::Overflow
        } else {
            Self::Soapy(err)
        }
    }
}

/// An open receiver, streaming.
pub struct Source {
    device: soapysdr::Device,
    stream: RxStream<Complex32>,
    config: DeviceConfig,
}

/// Reject a sample rate the configured family cannot deliver.
///
/// # Errors
///
/// [`SourceError::SampleRateTooHigh`] when the rate exceeds the maximum.
pub const fn check_sample_rate(config: &DeviceConfig) -> Result<(), SourceError> {
    let maximum = config.device.max_sample_rate_hz();

    if config.sample_rate_hz > maximum {
        return Err(SourceError::SampleRateTooHigh {
            requested: config.sample_rate_hz,
            maximum,
        });
    }

    Ok(())
}

/// Open the configured receiver and start streaming.
///
/// # Errors
///
/// [`SourceError::SampleRateTooHigh`], [`SourceError::NoDevice`], or [`SourceError::Soapy`].
pub fn open(config: &DeviceConfig) -> Result<Source, SourceError> {
    check_sample_rate(config)?;

    let filter = format!("driver={}", config.device.driver());
    if soapysdr::enumerate(filter.as_str())?.is_empty() {
        return Err(SourceError::NoDevice {
            device: config.device,
        });
    }

    let device = soapysdr::Device::new(filter.as_str())?;

    device.set_sample_rate(Rx, 0, f64::from(config.sample_rate_hz))?;

    let maximum = usize::try_from(config.device.max_sample_rate_hz()).unwrap_or(usize::MAX);
    let settled = u32::try_from(convert::to_index(
        device.sample_rate(Rx, 0)?.round(),
        maximum,
    ))
    .unwrap_or(config.sample_rate_hz);

    if settled != config.sample_rate_hz {
        tracing::warn!(
            requested_hz = config.sample_rate_hz,
            settled_hz = settled,
            "the driver did not accept the configured sample rate"
        );
    }

    match config.gain_db {
        Some(gain) => {
            device.set_gain_mode(Rx, 0, false)?;
            device.set_gain(Rx, 0, gain)?;
        }
        None => device.set_gain_mode(Rx, 0, true)?,
    }

    let mut stream = device.rx_stream::<Complex32>(&[0])?;
    stream.activate(None)?;

    Ok(Source {
        device,
        stream,
        config: DeviceConfig {
            sample_rate_hz: settled,
            ..*config
        },
    })
}

impl Drop for Source {
    fn drop(&mut self) {
        if let Err(err) = self.stream.deactivate(None) {
            tracing::warn!(error = %err, "failed to deactivate the receive stream");
        }
    }
}

/// A receiver the sampling loop can drive.
pub trait IqSource {
    /// Retune to a channel's center frequency, returning how far the receiver landed from it in Hz.
    ///
    /// # Errors
    ///
    /// [`SourceError`] as reported by the receiver.
    fn tune(&mut self, frequency_hz: u64) -> Result<f64, SourceError>;

    /// Fill `buffer` with IQ samples, returning how many landed.
    ///
    /// # Errors
    ///
    /// [`SourceError`] as reported by the receiver.
    fn read_iq(&mut self, buffer: &mut [Complex32]) -> Result<usize, SourceError>;

    /// The configuration this source runs under.
    fn config(&self) -> &DeviceConfig;
}

impl IqSource for Source {
    fn tune(&mut self, frequency_hz: u64) -> Result<f64, SourceError> {
        let frequency = f64::from(
            u32::try_from(frequency_hz)
                .map_err(|_| SourceError::FrequencyOutOfRange { frequency_hz })?,
        );

        self.device.set_frequency(Rx, 0, frequency, ())?;

        Ok(self.device.frequency(Rx, 0)? - frequency)
    }

    fn read_iq(&mut self, buffer: &mut [Complex32]) -> Result<usize, SourceError> {
        Ok(self.stream.read(&mut [buffer], READ_TIMEOUT_US)?)
    }

    fn config(&self) -> &DeviceConfig {
        &self.config
    }
}

const SYNTHETIC_DEVIATION_HZ: f64 = 60_000.0;
const SYNTHETIC_TONE_HZ: f64 = 1_000.0;

/// An in-process FM carrier, for running without hardware.
pub struct Synthetic {
    config: DeviceConfig,
    phase: f64,
    tone_phase: f64,
    offset_hz: f64,
    amplitude: f64,
    noise_amplitude: f64,
    random: u64,
}

impl Synthetic {
    /// A synthetic source running at `config`'s sample rate.
    #[must_use]
    pub const fn new(config: DeviceConfig) -> Self {
        Self {
            config,
            phase: 0.0,
            tone_phase: 0.0,
            offset_hz: 0.0,
            amplitude: 0.5,
            noise_amplitude: 0.01,
            random: 0x2545_F491_4F6C_DD1D,
        }
    }

    fn next_uniform(&mut self) -> f64 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;

        let bits = self.random >> 11;

        crate::dsp::convert::index_to_f64(usize::try_from(bits).unwrap_or(0))
            / crate::dsp::convert::index_to_f64(1_usize << 53)
            - 0.5
    }
}

impl IqSource for Synthetic {
    fn tune(&mut self, frequency_hz: u64) -> Result<f64, SourceError> {
        if !(87_500_000..=240_000_000).contains(&frequency_hz) {
            return Err(SourceError::FrequencyOutOfRange { frequency_hz });
        }

        let spread = crate::dsp::convert::index_to_f64(
            usize::try_from(frequency_hz.wrapping_div(100_000) % 97).unwrap_or(0),
        ) / 97.0;

        self.offset_hz = spread.mul_add(4_000.0, -2_000.0);
        self.amplitude = spread.mul_add(0.35, 0.1);
        self.noise_amplitude = spread.mul_add(0.02, 0.002);
        self.phase = 0.0;
        self.tone_phase = 0.0;

        Ok(0.0)
    }

    fn read_iq(&mut self, buffer: &mut [Complex32]) -> Result<usize, SourceError> {
        let sample_rate = f64::from(self.config.sample_rate_hz);
        if sample_rate <= 0.0 {
            return Err(SourceError::SampleRateTooHigh {
                requested: self.config.sample_rate_hz,
                maximum: self.config.device.max_sample_rate_hz(),
            });
        }

        let tone_step = std::f64::consts::TAU * SYNTHETIC_TONE_HZ / sample_rate;
        let carrier_step = std::f64::consts::TAU / sample_rate;

        for slot in buffer.iter_mut() {
            let deviation = SYNTHETIC_DEVIATION_HZ * self.tone_phase.sin();
            self.phase = carrier_step.mul_add(self.offset_hz + deviation, self.phase);
            self.tone_phase += tone_step;

            if self.phase > std::f64::consts::TAU {
                self.phase -= std::f64::consts::TAU;
            }

            if self.tone_phase > std::f64::consts::TAU {
                self.tone_phase -= std::f64::consts::TAU;
            }

            let carrier_real = self.amplitude * self.phase.cos();
            let carrier_imaginary = self.amplitude * self.phase.sin();
            let real = self
                .noise_amplitude
                .mul_add(self.next_uniform(), carrier_real);
            let imaginary = self
                .noise_amplitude
                .mul_add(self.next_uniform(), carrier_imaginary);

            *slot = Complex32::new(
                crate::dsp::convert::narrow(real),
                crate::dsp::convert::narrow(imaginary),
            );
        }

        Ok(buffer.len())
    }

    fn config(&self) -> &DeviceConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_rate_the_receiver_cannot_reach() {
        let config = DeviceConfig {
            device: Device::RtlSdr,
            sample_rate_hz: 6_000_000,
            gain_db: None,
        };

        match check_sample_rate(&config) {
            Err(SourceError::SampleRateTooHigh { requested, maximum }) => {
                assert_eq!(requested, 6_000_000);
                assert_eq!(maximum, 2_400_000);
            }
            other => panic!("expected a rate rejection, got {other:?}"),
        }
    }

    #[test]
    fn accepts_a_rate_the_receiver_can_reach() {
        let config = DeviceConfig {
            device: Device::AirspyMini,
            sample_rate_hz: 6_000_000,
            gain_db: None,
        };

        check_sample_rate(&config).expect("6 MSPS is inside the Airspy Mini's range");
    }

    #[test]
    fn driver_keys_are_the_soapy_ones() {
        assert_eq!(Device::RtlSdr.driver(), "rtlsdr");
        assert_eq!(Device::AirspyMini.driver(), "airspy");
    }
}
