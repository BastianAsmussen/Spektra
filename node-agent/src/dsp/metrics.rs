use protocol::v1::{Metric, Modulation};

use super::convert::narrow;
use super::welch::Psd;
use crate::report::MetricSample;

/// Occupied FM bandwidth in Hz (Carson: 2*(75 kHz + 15 kHz)).
pub const FM_BANDWIDTH_HZ: u32 = 180_000;

/// Occupied DAB ensemble bandwidth in Hz.
pub const DAB_BANDWIDTH_HZ: u32 = 1_536_000;

const GUARD_INNER: f64 = 1.5;
const GUARD_OUTER: f64 = 0.95;
const MINIMUM_GUARD_BINS: usize = 16;
/// Occupancy threshold as a power ratio (4 = 6 dB).
const OCCUPANCY_THRESHOLD: f64 = 4.0;
const FLOOR: f64 = 1e-30;

/// What the node was asked to measure on one channel.
#[derive(Debug, Clone)]
pub struct ChannelSpec {
    pub frequency_hz: u64,
    pub modulation: Modulation,
    /// Occupied bandwidth in Hz, or zero for the modulation default.
    pub bandwidth_hz: u32,
}

impl ChannelSpec {
    /// The bandwidth to measure over, resolving a zero to the default.
    #[must_use]
    pub const fn effective_bandwidth_hz(&self) -> u32 {
        if self.bandwidth_hz > 0 {
            return self.bandwidth_hz;
        }

        match self.modulation {
            Modulation::Dab => DAB_BANDWIDTH_HZ,
            Modulation::Fm | Modulation::Unspecified => FM_BANDWIDTH_HZ,
        }
    }
}

/// Derive every spectrum metric this build supports for one channel.
#[must_use]
pub fn derive(psd: &Psd, spec: &ChannelSpec, lo_error_hz: f64) -> Vec<MetricSample> {
    let half_width = f64::from(spec.effective_bandwidth_hz()) / 2.0;
    let band = psd.band(half_width);
    if band.is_empty() {
        return Vec::new();
    }

    let Some(floor_per_bin) = noise_floor(psd, half_width) else {
        return Vec::new();
    };

    let in_band_bins = band.len();
    let total_power = psd.power_in(band.clone()).max(FLOOR);
    let noise_power = (floor_per_bin * bins_to_f64(in_band_bins)).max(FLOOR);
    let signal_power = (total_power - noise_power).max(FLOOR);

    let sample = |metric: Metric, value: f64| MetricSample {
        channel_frequency_hz: spec.frequency_hz,
        metric,
        value,
    };

    vec![
        sample(Metric::SignalStrength, 10.0 * total_power.log10()),
        sample(
            Metric::SignalToNoise,
            10.0 * (signal_power / noise_power).log10(),
        ),
        sample(
            Metric::CarrierOffset,
            carrier_offset(psd, &band, floor_per_bin) + lo_error_hz,
        ),
        sample(
            Metric::SpectrumOccupancy,
            occupancy(psd, &band, floor_per_bin),
        ),
    ]
}

fn noise_floor(psd: &Psd, half_width_hz: f64) -> Option<f64> {
    let mut guard = guard_bins(psd, half_width_hz * GUARD_INNER);
    if guard.len() < MINIMUM_GUARD_BINS {
        guard = guard_bins(psd, half_width_hz);
    }

    median(&mut guard)
}

fn guard_bins(psd: &Psd, inner_hz: f64) -> Vec<f64> {
    let outer_hz = span_limit(psd);
    if inner_hz >= outer_hz {
        return Vec::new();
    }

    let low_start = psd.index_at(-outer_hz);
    let low_end = psd.index_at(-inner_hz);
    let high_start = psd.index_at(inner_hz);
    let high_end = psd.index_at(outer_hz);

    let low = psd.band_bins(&(low_start..low_end.max(low_start)));
    let high = psd.band_bins(&(high_start..high_end.max(high_start)));

    let mut guard = Vec::with_capacity(low.len().saturating_add(high.len()));
    guard.extend(low.iter().chain(high).map(|power| f64::from(*power)));

    guard
}

fn span_limit(psd: &Psd) -> f64 {
    psd.offset_hz(psd.bins().len()).abs() * GUARD_OUTER
}

fn carrier_offset(psd: &Psd, band: &std::ops::Range<usize>, floor_per_bin: f64) -> f64 {
    let bins = psd.band_bins(band);
    let width = psd.bin_width_hz();
    let first_offset = psd.offset_hz(band.start);

    let mut weight = 0.0_f64;
    let mut moment = 0.0_f64;

    for (step, power) in bins.iter().enumerate() {
        let excess = (f64::from(*power) - floor_per_bin).max(0.0);
        let offset = bins_to_f64(step).mul_add(width, first_offset);

        weight += excess;
        moment = excess.mul_add(offset, moment);
    }

    if weight <= 0.0 { 0.0 } else { moment / weight }
}

fn occupancy(psd: &Psd, band: &std::ops::Range<usize>, floor_per_bin: f64) -> f64 {
    let bins = psd.band_bins(band);
    if bins.is_empty() {
        return 0.0;
    }

    let threshold = narrow(floor_per_bin * OCCUPANCY_THRESHOLD);
    let occupied = bins.iter().filter(|power| **power > threshold).count();

    bins_to_f64(occupied) / bins_to_f64(bins.len())
}

fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }

    let middle = values.len() / 2;
    values.select_nth_unstable_by(middle, f64::total_cmp);
    let upper = values.get(middle).copied()?;

    if values.len() % 2 == 1 {
        return Some(upper);
    }

    let lower = values
        .get(..middle)?
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);

    Some(f64::midpoint(lower, upper))
}

const fn bins_to_f64(count: usize) -> f64 {
    super::convert::index_to_f64(count)
}

#[cfg(test)]
mod tests {
    use num_complex::Complex32;

    use super::*;
    use crate::dsp::convert::{index_to_f64, narrow};
    use crate::dsp::welch::Welch;
    use crate::dsp::window::Window;

    const SAMPLE_RATE: u32 = 2_400_000;
    const FFT_SIZE: usize = 4096;

    struct Noise(u64);

    impl Noise {
        fn next_uniform(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;

            index_to_f64(usize::try_from(self.0 >> 11).unwrap_or(0)) / index_to_f64(1_usize << 53)
                - 0.5
        }

        fn next_gaussian(&mut self) -> (f64, f64) {
            let u = (self.next_uniform() + 0.5).max(1e-12);
            let v = self.next_uniform() + 0.5;
            let radius = (-2.0 * u.ln()).sqrt();
            let angle = std::f64::consts::TAU * v;

            (radius * angle.cos(), radius * angle.sin())
        }
    }

    fn signal(
        count: usize,
        offset_hz: f64,
        amplitude: f64,
        noise_amplitude: f64,
    ) -> Vec<Complex32> {
        let mut noise = Noise(0x2545_F491_4F6C_DD1D);
        let step = std::f64::consts::TAU * offset_hz / f64::from(SAMPLE_RATE);

        (0..count)
            .map(|index| {
                let phase = step * index_to_f64(index);
                let (real_noise, imaginary_noise) = noise.next_gaussian();

                Complex32::new(
                    narrow(amplitude.mul_add(phase.cos(), noise_amplitude * real_noise)),
                    narrow(amplitude.mul_add(phase.sin(), noise_amplitude * imaginary_noise)),
                )
            })
            .collect()
    }

    fn spectrum(samples: &[Complex32]) -> Psd {
        let mut welch = Welch::new(FFT_SIZE, SAMPLE_RATE, Window::Hann).expect("a power of two");
        welch.push(samples);

        welch.finish().expect("segments were pushed")
    }

    fn spec() -> ChannelSpec {
        ChannelSpec {
            frequency_hz: 89_700_000,
            modulation: Modulation::Fm,
            bandwidth_hz: 0,
        }
    }

    fn value(samples: &[MetricSample], metric: Metric) -> f64 {
        samples
            .iter()
            .find(|sample| sample.metric == metric)
            .map(|sample| sample.value)
            .expect("the metric was derived")
    }

    #[test]
    fn a_zero_bandwidth_takes_the_default_for_the_modulation() {
        assert_eq!(spec().effective_bandwidth_hz(), FM_BANDWIDTH_HZ);
        assert_eq!(
            ChannelSpec {
                modulation: Modulation::Dab,
                ..spec()
            }
            .effective_bandwidth_hz(),
            DAB_BANDWIDTH_HZ
        );
        assert_eq!(
            ChannelSpec {
                bandwidth_hz: 12_500,
                ..spec()
            }
            .effective_bandwidth_hz(),
            12_500
        );
    }

    #[test]
    fn every_derived_metric_is_produced() {
        let samples = derive(
            &spectrum(&signal(FFT_SIZE * 8, 0.0, 0.5, 0.01)),
            &spec(),
            0.0,
        );

        for metric in crate::dsp::DERIVED_METRICS {
            assert!(
                samples.iter().any(|sample| sample.metric == *metric),
                "{metric:?} was not derived"
            );
        }
    }

    #[test]
    fn the_carrier_offset_recovers_an_injected_offset() {
        for offset in [-40_000.0, -1_000.0, 0.0, 2_500.0, 50_000.0] {
            let psd = spectrum(&signal(FFT_SIZE * 16, offset, 0.5, 0.002));
            let recovered = value(&derive(&psd, &spec(), 0.0), Metric::CarrierOffset);

            assert!(
                (recovered - offset).abs() < psd.bin_width_hz(),
                "injected {offset} Hz, recovered {recovered} Hz, bin is {} Hz",
                psd.bin_width_hz()
            );
        }
    }

    #[test]
    fn a_stronger_carrier_reads_a_higher_signal_strength() {
        let weak = value(
            &derive(
                &spectrum(&signal(FFT_SIZE * 8, 0.0, 0.05, 0.002)),
                &spec(),
                0.0,
            ),
            Metric::SignalStrength,
        );
        let strong = value(
            &derive(
                &spectrum(&signal(FFT_SIZE * 8, 0.0, 0.5, 0.002)),
                &spec(),
                0.0,
            ),
            Metric::SignalStrength,
        );

        assert!(strong > weak, "{strong} dBFS is not above {weak} dBFS");
        assert!(strong <= 0.0, "{strong} dBFS is above full scale");
    }

    #[test]
    fn the_signal_to_noise_ratio_tracks_the_injected_one() {
        let quiet = value(
            &derive(
                &spectrum(&signal(FFT_SIZE * 8, 0.0, 0.5, 0.0005)),
                &spec(),
                0.0,
            ),
            Metric::SignalToNoise,
        );
        let noisy = value(
            &derive(
                &spectrum(&signal(FFT_SIZE * 8, 0.0, 0.5, 0.05)),
                &spec(),
                0.0,
            ),
            Metric::SignalToNoise,
        );

        assert!(
            quiet > noisy + 20.0,
            "a hundredfold quieter channel should read far better: {quiet} dB against {noisy} dB"
        );
    }

    #[test]
    fn a_bare_carrier_occupies_little_of_its_channel() {
        let occupancy = value(
            &derive(
                &spectrum(&signal(FFT_SIZE * 8, 0.0, 0.5, 0.002)),
                &spec(),
                0.0,
            ),
            Metric::SpectrumOccupancy,
        );

        assert!(
            (0.0..0.2).contains(&occupancy),
            "a single tone should occupy almost none of a 180 kHz channel, read {occupancy}"
        );
        assert!((0.0..=1.0).contains(&occupancy));
    }

    #[test]
    fn the_median_of_an_even_set_averages_the_middle_pair() {
        assert_eq!(median(&mut [1.0, 2.0, 3.0, 4.0]), Some(2.5));
        assert_eq!(median(&mut [3.0, 1.0]), Some(2.0));
        assert_eq!(median(&mut [5.0, 1.0, 3.0]), Some(3.0));
        assert_eq!(median(&mut []), None);
    }
}

#[cfg(test)]
mod invariants {
    use num_complex::Complex32;

    use super::*;
    use crate::dsp::welch::Welch;
    use crate::dsp::window::Window;

    fn spectrum(size: usize, sample_rate: u32) -> Psd {
        let mut welch = Welch::new(size, sample_rate, Window::Hann).expect("a power of two");
        welch.push(&vec![Complex32::new(0.25, 0.0); size.saturating_mul(4)]);

        welch.finish().expect("segments were pushed")
    }

    #[test]
    fn a_bin_index_round_trips_through_its_frequency() {
        let psd = spectrum(1024, 2_400_000);

        for index in [0_usize, 1, 511, 512, 513, 1023] {
            assert_eq!(
                psd.index_at(psd.offset_hz(index)),
                index,
                "bin {index} did not survive the round trip"
            );
        }
    }

    fn guard_by_predicate(psd: &Psd, inner_hz: f64, outer_hz: f64) -> Vec<f64> {
        psd.bins()
            .iter()
            .enumerate()
            .filter_map(|(index, power)| {
                let offset = psd.offset_hz(index).abs();

                (offset >= inner_hz && offset <= outer_hz).then(|| f64::from(*power))
            })
            .collect()
    }

    #[test]
    fn the_index_arithmetic_selects_what_the_predicate_did() {
        for (size, sample_rate) in [(1024, 2_400_000), (4096, 2_400_000), (256, 240_000)] {
            let psd = spectrum(size, sample_rate);
            let outer = span_limit(&psd);

            for half_width in [10_000.0_f64, 90_000.0, 250_000.0] {
                let inner = half_width * GUARD_INNER;
                if inner >= outer {
                    continue;
                }

                let expected = guard_by_predicate(&psd, inner, outer);
                let actual = guard_bins(&psd, inner);

                assert!(
                    actual.len().abs_diff(expected.len()) <= 4,
                    "size {size}, half width {half_width}: {} bins against {}",
                    actual.len(),
                    expected.len()
                );

                let sum = |bins: &[f64]| bins.iter().sum::<f64>();
                let difference = (sum(&actual) - sum(&expected)).abs();
                assert!(
                    difference <= sum(&expected).abs().mul_add(1e-6, 1e-12),
                    "size {size}, half width {half_width}: the selections carry different power"
                );
            }
        }
    }

    #[test]
    fn the_guard_region_excludes_the_channel() {
        let psd = spectrum(4096, 2_400_000);
        let half_width = 90_000.0;
        let inner = half_width * GUARD_INNER;

        let low_end = psd.index_at(-inner);
        let high_start = psd.index_at(inner);
        let in_band = psd.band(half_width);

        assert!(
            low_end <= in_band.start,
            "the guard reached into the channel"
        );
        assert!(
            high_start >= in_band.end,
            "the guard reached into the channel"
        );
    }

    #[test]
    fn a_span_too_narrow_for_a_guard_has_no_guard() {
        let psd = spectrum(256, 240_000);

        assert!(guard_bins(&psd, 90_000.0 * GUARD_INNER).is_empty());
    }
}
