use protocol::v1::{Metric, Modulation};

use super::convert::narrow;
use super::welch::Psd;
use crate::report::MetricSample;

/// Occupied FM bandwidth in Hz (Carson: 2*(75 kHz + 15 kHz)).
pub const FM_BANDWIDTH_HZ: u32 = 180_000;

/// Occupied DAB ensemble bandwidth in Hz.
pub const DAB_BANDWIDTH_HZ: u32 = 1_536_000;

/// Band III block centres 5A to 13F (ETSI EN 300 401).
const DAB_BLOCKS_HZ: [u32; 38] = [
    174_928_000,
    176_640_000,
    178_352_000,
    180_064_000,
    181_936_000,
    183_648_000,
    185_360_000,
    187_072_000,
    188_928_000,
    190_640_000,
    192_352_000,
    194_064_000,
    195_936_000,
    197_648_000,
    199_360_000,
    201_072_000,
    202_928_000,
    204_640_000,
    206_352_000,
    208_064_000,
    209_936_000,
    211_648_000,
    213_360_000,
    215_072_000,
    216_928_000,
    218_640_000,
    220_352_000,
    222_064_000,
    223_936_000,
    225_648_000,
    227_360_000,
    229_072_000,
    230_784_000,
    232_496_000,
    234_208_000,
    235_776_000,
    237_488_000,
    239_200_000,
];

/// Clears a block's roll-off past its edge.
const DAB_SHOULDER_HZ: f64 = 40_000.0;

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

        default_bandwidth_hz(self.modulation)
    }
}

const fn default_bandwidth_hz(modulation: Modulation) -> u32 {
    match modulation {
        Modulation::Dab => DAB_BANDWIDTH_HZ,
        Modulation::Fm | Modulation::Unspecified => FM_BANDWIDTH_HZ,
    }
}

/// Whether `sample_rate_hz` covers a whole `modulation` channel plus guard bins for the noise floor.
#[must_use]
pub fn measurable(modulation: Modulation, sample_rate_hz: u32) -> bool {
    let span_hz = GUARD_OUTER * f64::from(sample_rate_hz) / 2.0;

    match modulation {
        Modulation::Fm => span_hz > f64::from(FM_BANDWIDTH_HZ) / 2.0,
        Modulation::Dab => span_hz > dab_reach_hz(),
        Modulation::Unspecified => false,
    }
}

fn dab_reach_hz() -> f64 {
    f64::from(DAB_BANDWIDTH_HZ) / 2.0 + DAB_SHOULDER_HZ
}

/// Derive every spectrum metric this build supports for one channel.
#[must_use]
pub fn derive(psd: &Psd, spec: &ChannelSpec, lo_error_hz: f64) -> Vec<MetricSample> {
    let half_width = f64::from(spec.effective_bandwidth_hz()) / 2.0;
    let band = psd.band(half_width);
    if band.is_empty() {
        return Vec::new();
    }

    let Some(floor_per_bin) = noise_floor(psd, spec, half_width) else {
        return Vec::new();
    };

    let in_band_bins = band.len();
    let total_power = psd.power_in(band.clone()).max(FLOOR);
    let noise_power = (floor_per_bin * bins_to_f64(in_band_bins)).max(FLOOR);

    let sample = |metric: Metric, value: f64| MetricSample {
        channel_frequency_hz: spec.frequency_hz,
        metric,
        value,
    };

    vec![
        sample(Metric::SignalStrength, 10.0 * total_power.log10()),
        // (S+N)/N, not S/N: a carrier lost in the noise reads 0 dB instead of going negative.
        sample(
            Metric::SignalToNoise,
            10.0 * (total_power.max(noise_power) / noise_power).log10(),
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

fn noise_floor(psd: &Psd, spec: &ChannelSpec, half_width_hz: f64) -> Option<f64> {
    let mut guard = match spec.modulation {
        Modulation::Dab => dab_gap_bins(psd, spec.frequency_hz, half_width_hz)?,
        Modulation::Fm | Modulation::Unspecified => {
            let guard = guard_bins(psd, half_width_hz * GUARD_INNER);
            if guard.len() < MINIMUM_GUARD_BINS {
                guard_bins(psd, half_width_hz)
            } else {
                guard
            }
        }
    };

    median(&mut guard)
}

/// Neighbouring blocks swamp a plain guard; only the raster gaps are free of them.
fn dab_gap_bins(psd: &Psd, frequency_hz: u64, half_width_hz: f64) -> Option<Vec<f64>> {
    let tuned_hz = f64::from(u32::try_from(frequency_hz).ok()?);
    let outer_hz = span_limit(psd);
    let reach_hz = dab_reach_hz();

    let blocks: Vec<f64> = DAB_BLOCKS_HZ
        .iter()
        .map(|block| f64::from(*block) - tuned_hz)
        .filter(|offset| offset.abs() < outer_hz + reach_hz)
        .collect();

    let low = psd.index_at(-outer_hz)..psd.index_at(-half_width_hz);
    let high = psd.index_at(half_width_hz)..psd.index_at(outer_hz);

    let gap: Vec<f64> = low
        .chain(high)
        .filter(|index| {
            let offset = psd.offset_hz(*index);
            blocks.iter().all(|block| (offset - block).abs() > reach_hz)
        })
        .filter_map(|index| psd.bins().get(index).map(|power| f64::from(*power)))
        .collect();

    (gap.len() >= MINIMUM_GUARD_BINS).then_some(gap)
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

    #[test]
    fn both_supported_receivers_can_measure_a_dab_block() {
        for sample_rate_hz in [2_400_000, 6_000_000] {
            assert!(measurable(Modulation::Fm, sample_rate_hz));
            assert!(measurable(Modulation::Dab, sample_rate_hz));
        }
    }

    #[test]
    fn a_rate_too_narrow_for_a_dab_block_leaves_it_out() {
        assert!(measurable(Modulation::Fm, 1_024_000));
        assert!(!measurable(Modulation::Dab, 1_024_000));
        assert!(!measurable(Modulation::Dab, 1_600_000));
        assert!(!measurable(Modulation::Unspecified, 6_000_000));
    }

    #[test]
    fn the_band_iii_raster_is_ordered_and_inside_the_dab_band() {
        assert!(
            DAB_BLOCKS_HZ
                .iter()
                .all(|block| (174_000_000..=240_000_000).contains(block))
        );
        assert!(
            DAB_BLOCKS_HZ
                .windows(2)
                .all(|pair| pair[1].saturating_sub(pair[0]) >= 1_568_000)
        );
    }

    /// Mode I carrier spacing, so a block is flat at the test's bin width.
    const DAB_TONES: usize = 1537;
    const DAB_TONE_SPACING_HZ: f64 = 1_000.0;
    const DAB_SNR: f64 = 100.0;
    /// 13B, between 13A and 13C.
    const DAB_BLOCK_HZ: u64 = 232_496_000;
    const DAB_NEIGHBOUR_HZ: f64 = 1_712_000.0;

    /// Tones past Nyquist are dropped, as an anti-alias filter would.
    fn dab_blocks(sample_rate: u32, blocks: &[(f64, f64)], noise_amplitude: f64) -> Vec<Complex32> {
        let count = FFT_SIZE.saturating_mul(8);
        let nyquist_hz = f64::from(sample_rate) / 2.0;
        let mut noise = Noise(0x9E37_79B9_7F4A_7C15);
        let mut real = vec![0.0_f64; count];
        let mut imaginary = vec![0.0_f64; count];

        for &(centre_hz, amplitude) in blocks {
            for tone in 0..DAB_TONES {
                let start = std::f64::consts::TAU * (noise.next_uniform() + 0.5);
                let offset_hz = index_to_f64(tone).mul_add(
                    DAB_TONE_SPACING_HZ,
                    centre_hz - f64::from(DAB_BANDWIDTH_HZ) / 2.0,
                );
                if offset_hz.abs() >= nyquist_hz {
                    continue;
                }

                let step = std::f64::consts::TAU * offset_hz / f64::from(sample_rate);
                let (step_cos, step_sin) = (step.cos(), step.sin());
                let (mut cos, mut sin) = (amplitude * start.cos(), amplitude * start.sin());

                for (re, im) in real.iter_mut().zip(imaginary.iter_mut()) {
                    *re += cos;
                    *im += sin;
                    (cos, sin) = (
                        cos.mul_add(step_cos, -sin * step_sin),
                        cos.mul_add(step_sin, sin * step_cos),
                    );
                }
            }
        }

        real.iter()
            .zip(&imaginary)
            .map(|(re, im)| {
                let (real_noise, imaginary_noise) = noise.next_gaussian();

                Complex32::new(
                    narrow(noise_amplitude.mul_add(real_noise, *re)),
                    narrow(noise_amplitude.mul_add(imaginary_noise, *im)),
                )
            })
            .collect()
    }

    fn dab_snr(sample_rate: u32, neighbour_gain_db: Option<f64>) -> f64 {
        let noise_amplitude = 0.01;
        let in_band_noise = 2.0 * noise_amplitude * noise_amplitude * f64::from(DAB_BANDWIDTH_HZ)
            / f64::from(sample_rate);
        let amplitude = (DAB_SNR * in_band_noise / index_to_f64(DAB_TONES)).sqrt();

        let mut blocks = vec![(0.0, amplitude)];
        if let Some(gain_db) = neighbour_gain_db {
            let neighbour = amplitude * 10.0_f64.powf(gain_db / 20.0);
            blocks.push((-DAB_NEIGHBOUR_HZ, neighbour));
            blocks.push((DAB_NEIGHBOUR_HZ, neighbour));
        }

        let samples = dab_blocks(sample_rate, &blocks, noise_amplitude);
        let mut welch = Welch::new(FFT_SIZE, sample_rate, Window::Hann).expect("a power of two");
        welch.push(&samples);
        let psd = welch.finish().expect("segments were pushed");

        let spec = ChannelSpec {
            frequency_hz: DAB_BLOCK_HZ,
            modulation: Modulation::Dab,
            bandwidth_hz: 0,
        };

        value(&derive(&psd, &spec, 0.0), Metric::SignalToNoise)
    }

    #[test]
    fn a_dab_block_reads_the_injected_signal_to_noise_ratio() {
        let expected = 10.0 * (1.0 + DAB_SNR).log10();
        let snr = dab_snr(6_000_000, None);

        assert!(
            (snr - expected).abs() < 1.0,
            "expected about {expected} dB, read {snr} dB"
        );
    }

    #[test]
    fn neighbouring_dab_blocks_do_not_move_the_floor() {
        for sample_rate in [6_000_000, 2_400_000] {
            let alone = dab_snr(sample_rate, None);
            let flanked = dab_snr(sample_rate, Some(10.0));

            assert!(
                (alone - flanked).abs() < 0.5,
                "at {sample_rate} S/s: {alone} dB alone, {flanked} dB between two blocks 10 dB stronger"
            );
        }
    }

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
    fn a_channel_of_pure_noise_reads_near_zero_and_inside_the_range() {
        let snr = value(
            &derive(
                &spectrum(&signal(FFT_SIZE * 8, 0.0, 0.0, 0.05)),
                &spec(),
                0.0,
            ),
            Metric::SignalToNoise,
        );

        assert!(
            (0.0..=1.0).contains(&snr),
            "a channel with no carrier should read close to 0 dB, not {snr} dB"
        );
        assert!(
            protocol::metric_range(Metric::SignalToNoise).is_some_and(|range| range.contains(&snr)),
            "{snr} dB falls outside the range the server accepts"
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
