use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use num_complex::Complex32;
use tokio::sync::{mpsc, watch};

use crate::dsp::window::Window;
use crate::dsp::{ChannelSpec, Welch, derive};
use crate::report::{ChannelIdentity, MetricSample};
use crate::source::IqSource;

///
const BLOCK_SAMPLES: usize = 0x0001_0000;

///
const SETTLE: Duration = Duration::from_millis(50);

const IDLE: Duration = Duration::from_secs(5);

/// One channel the sampler visits.
#[derive(Debug, Clone)]
pub struct Assignment {
    pub identity: ChannelIdentity,
    pub spec: ChannelSpec,
}

/// The set of channels a node is currently assigned, and its version.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub version: u64,
    pub channels: Vec<Assignment>,
}

/// What one dwell produced.
#[derive(Debug)]
pub enum Event {
    /// A channel was measured.
    Measured {
        channel: ChannelIdentity,
        samples: Vec<MetricSample>,
    },
    /// A channel could not be measured, with the reason.
    ///
    Failed { frequency_hz: u64, reason: String },
}

/// A running sampler.
pub struct Sampler<S> {
    source: S,
    welch: Welch,
    block: Vec<Complex32>,
    dwell: Duration,
}

impl<S: IqSource> Sampler<S> {
    ///
    /// # Errors
    ///
    /// [`spektra_fft::FftError`] if `fft_size` is not a power of two.
    pub fn new(source: S, fft_size: usize, dwell: Duration) -> Result<Self, spektra_fft::FftError> {
        let sample_rate = source.config().sample_rate_hz;

        Ok(Self {
            welch: Welch::new(fft_size, sample_rate, Window::Hann)?,
            source,
            block: vec![Complex32::new(0.0, 0.0); BLOCK_SAMPLES],
            dwell,
        })
    }

    /// Visit the assigned channels in turn until `running` clears.
    ///
    pub fn run(
        mut self,
        plan: &watch::Receiver<Plan>,
        events: &mpsc::UnboundedSender<Event>,
        running: &Arc<AtomicBool>,
    ) {
        while running.load(Ordering::Relaxed) {
            let assignments = plan.borrow().channels.clone();

            if assignments.is_empty() {
                tracing::debug!("no channels assigned, waiting for a plan");
                std::thread::sleep(IDLE);

                continue;
            }

            let version = plan.borrow().version;
            for assignment in &assignments {
                if !running.load(Ordering::Relaxed) {
                    return;
                }
                if plan.borrow().version != version {
                    break;
                }

                if events.send(self.dwell_on(assignment)).is_err() {
                    tracing::info!("the reporting side is gone, stopping the sampler");

                    return;
                }
            }
        }
    }

    fn dwell_on(&mut self, assignment: &Assignment) -> Event {
        let frequency_hz = assignment.spec.frequency_hz;

        if let Err(err) = self.source.tune(frequency_hz) {
            return Event::Failed {
                frequency_hz,
                reason: err.to_string(),
            };
        }

        if let Err(err) = self.discard(SETTLE) {
            return Event::Failed {
                frequency_hz,
                reason: err,
            };
        }

        let started = Instant::now();
        while started.elapsed() < self.dwell {
            match self.source.read_iq(&mut self.block) {
                Ok(0) => break,
                Ok(count) => {
                    if let Some(read) = self.block.get(..count) {
                        self.welch.push(read);
                    }
                }
                Err(err) => {
                    return Event::Failed {
                        frequency_hz,
                        reason: err.to_string(),
                    };
                }
            }
        }

        let Some(psd) = self.welch.finish() else {
            return Event::Failed {
                frequency_hz,
                reason: "the dwell was too short to complete one transform".to_owned(),
            };
        };

        Event::Measured {
            channel: assignment.identity.clone(),
            samples: derive(&psd, &assignment.spec),
        }
    }

    fn discard(&mut self, duration: Duration) -> Result<(), String> {
        let started = Instant::now();

        while started.elapsed() < duration {
            match self.source.read_iq(&mut self.block) {
                Ok(0) => break,
                Ok(_) => {}
                Err(err) => return Err(err.to_string()),
            }
        }

        Ok(())
    }
}

/// Start a sampler on its own thread.
///
pub fn spawn<S: IqSource + Send + 'static>(
    sampler: Sampler<S>,
    plan: watch::Receiver<Plan>,
    events: mpsc::UnboundedSender<Event>,
    running: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("spektra-sampler".to_owned())
        .spawn(move || sampler.run(&plan, &events, &running))
        .unwrap_or_else(|err| {
            tracing::error!(error = %err, "could not name the sampler thread");

            std::thread::spawn(move || {})
        })
}

#[cfg(test)]
mod tests {
    use protocol::v1::{Metric, Modulation};

    use super::*;
    use crate::source::{Device, DeviceConfig, Synthetic};

    fn config() -> DeviceConfig {
        DeviceConfig {
            device: Device::RtlSdr,
            sample_rate_hz: 2_400_000,
            gain_db: Some(30.0),
        }
    }

    fn assignment(frequency_hz: u64) -> Assignment {
        Assignment {
            identity: ChannelIdentity {
                frequency_hz,
                modulation: Modulation::Fm,
                label: format!("{frequency_hz} Hz"),
            },
            spec: ChannelSpec {
                frequency_hz,
                modulation: Modulation::Fm,
                bandwidth_hz: 0,
            },
        }
    }

    #[test]
    fn a_dwell_produces_every_derived_metric() {
        let mut sampler = Sampler::new(Synthetic::new(config()), 4096, Duration::from_millis(120))
            .expect("4096 is a power of two");

        match sampler.dwell_on(&assignment(89_700_000)) {
            Event::Measured { channel, samples } => {
                assert_eq!(channel.frequency_hz, 89_700_000);

                for metric in crate::dsp::DERIVED_METRICS {
                    assert!(
                        samples.iter().any(|sample| sample.metric == *metric),
                        "{metric:?} was not measured"
                    );
                }
            }
            Event::Failed { reason, .. } => panic!("the dwell failed: {reason}"),
        }
    }

    #[test]
    fn an_untunable_frequency_fails_the_dwell_rather_than_the_agent() {
        let mut sampler = Sampler::new(Synthetic::new(config()), 1024, Duration::from_millis(50))
            .expect("a power of two");

        assert!(matches!(
            sampler.dwell_on(&assignment(1_000)),
            Event::Failed {
                frequency_hz: 1_000,
                ..
            }
        ));
    }

    #[test]
    fn the_synthetic_carrier_reads_a_plausible_level() {
        let mut sampler = Sampler::new(Synthetic::new(config()), 4096, Duration::from_millis(200))
            .expect("a power of two");

        let Event::Measured { samples, .. } = sampler.dwell_on(&assignment(97_300_000)) else {
            panic!("the dwell failed");
        };

        let strength = samples
            .iter()
            .find(|sample| sample.metric == Metric::SignalStrength)
            .map(|sample| sample.value)
            .expect("signal strength was measured");

        assert!(
            (-60.0..=0.0).contains(&strength),
            "a synthetic carrier read {strength} dBFS"
        );
    }

    #[test]
    fn the_loop_stops_when_told_to() {
        let sampler = Sampler::new(Synthetic::new(config()), 1024, Duration::from_millis(20))
            .expect("a power of two");

        let (plan_tx, plan_rx) = watch::channel(Plan {
            version: 1,
            channels: vec![assignment(89_700_000)],
        });
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let running = Arc::new(AtomicBool::new(true));

        let handle = spawn(sampler, plan_rx, events_tx, Arc::clone(&running));

        let mut seen = 0;
        while seen < 2 {
            if events_rx.blocking_recv().is_some() {
                seen += 1;
            } else {
                break;
            }
        }

        running.store(false, Ordering::Relaxed);
        handle.join().expect("the sampler thread joins");
        drop(plan_tx);

        assert_eq!(seen, 2);
    }
}
