use num_complex::Complex32;

use super::convert::{index_to_f64, narrow};

/// A low-pass filter and the decimation factor it was designed for.
#[derive(Debug, Clone)]
pub struct Decimator {
    taps: Vec<f32>,
    factor: usize,
    history: Vec<Complex32>,
    phase: usize,
}

/// Design a windowed-sinc low-pass. `cutoff_hz` is the -6 dB point.
#[must_use]
pub fn low_pass(cutoff_hz: f64, sample_rate_hz: u32, taps: usize) -> Vec<f32> {
    if taps == 0 || sample_rate_hz == 0 {
        return vec![1.0];
    }

    let normalized = (cutoff_hz / f64::from(sample_rate_hz)).clamp(0.0, 0.5);
    let center = index_to_f64(taps.saturating_sub(1)) / 2.0;

    let mut coefficients: Vec<f32> = (0..taps)
        .map(|index| {
            let position = index_to_f64(index) - center;
            let sinc = if position.abs() < f64::EPSILON {
                2.0 * normalized
            } else {
                (std::f64::consts::TAU * normalized * position).sin()
                    / (std::f64::consts::PI * position)
            };

            narrow(sinc * blackman(index, taps))
        })
        .collect();

    let sum: f32 = coefficients.iter().sum();
    if sum.abs() > f32::EPSILON {
        let scale = sum.recip();
        for tap in &mut coefficients {
            *tap *= scale;
        }
    }

    coefficients
}

fn blackman(index: usize, length: usize) -> f64 {
    if length <= 1 {
        return 1.0;
    }

    let phase =
        std::f64::consts::TAU * index_to_f64(index) / index_to_f64(length.saturating_sub(1));

    0.08_f64.mul_add((2.0 * phase).cos(), 0.5_f64.mul_add(-phase.cos(), 0.42))
}

impl Decimator {
    /// Build a decimator from designed taps. A `factor` of zero is treated as one.
    #[must_use]
    pub fn new(taps: Vec<f32>, factor: u32) -> Self {
        let width = taps.len();

        Self {
            taps,
            factor: usize::try_from(factor).unwrap_or(1).max(1),
            history: Vec::with_capacity(width),
            phase: 0,
        }
    }

    /// Design and build a low-pass at `cutoff_hz` decimating by `factor`.
    #[must_use]
    pub fn low_pass(cutoff_hz: f64, sample_rate_hz: u32, factor: u32, taps: usize) -> Self {
        Self::new(low_pass(cutoff_hz, sample_rate_hz, taps), factor)
    }

    /// The decimation factor.
    #[must_use]
    pub const fn factor(&self) -> usize {
        self.factor
    }

    /// Number of taps in the filter.
    #[must_use]
    pub const fn taps(&self) -> usize {
        self.taps.len()
    }

    /// Filter and decimate a block, appending the results to `out`.
    pub fn process(&mut self, block: &[Complex32], out: &mut Vec<Complex32>) {
        if self.taps.is_empty() {
            out.extend_from_slice(block);

            return;
        }

        let mut stream = std::mem::take(&mut self.history);
        stream.extend_from_slice(block);

        let width = self.taps.len();
        let mut position = self.phase;

        let reach = stream
            .len()
            .saturating_sub(width.saturating_sub(1))
            .saturating_sub(position);
        out.reserve(reach.checked_div(self.factor).unwrap_or(0));

        while let Some(segment) = stream.get(position..position.saturating_add(width)) {
            out.push(convolve(segment, &self.taps));
            position = position.saturating_add(self.factor);
        }

        let start = position.min(stream.len());
        self.phase = position.saturating_sub(start);
        stream.drain(..start);
        self.history = stream;
    }

    /// Forget the history, for a retune to an unrelated frequency.
    pub fn reset(&mut self) {
        self.history.clear();
        self.phase = 0;
    }
}

fn convolve(segment: &[Complex32], taps: &[f32]) -> Complex32 {
    debug_assert_eq!(
        segment.len(),
        taps.len(),
        "a segment shorter than the filter would silently convolve against part of it"
    );

    let mut real = 0.0_f32;
    let mut imaginary = 0.0_f32;

    for (sample, tap) in segment.iter().zip(taps) {
        real = sample.re.mul_add(*tap, real);
        imaginary = sample.im.mul_add(*tap, imaginary);
    }

    Complex32::new(real, imaginary)
}

/// Response of a set of taps at one frequency, in dB relative to DC.
#[must_use]
pub fn response_db(taps: &[f32], frequency_hz: f64, sample_rate_hz: u32) -> f64 {
    if taps.is_empty() || sample_rate_hz == 0 {
        return 0.0;
    }

    let step = std::f64::consts::TAU * frequency_hz / f64::from(sample_rate_hz);
    let mut real = 0.0_f64;
    let mut imaginary = 0.0_f64;

    for (index, tap) in taps.iter().enumerate() {
        let phase = step * index_to_f64(index);
        let weight = f64::from(*tap);

        real = weight.mul_add(phase.cos(), real);
        imaginary = weight.mul_add(-phase.sin(), imaginary);
    }

    let magnitude = real.hypot(imaginary);
    let dc: f64 = taps.iter().map(|tap| f64::from(*tap)).sum();

    20.0 * (magnitude / dc.abs().max(f64::MIN_POSITIVE)).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: u32 = 2_400_000;
    const CUTOFF: f64 = 100_000.0;
    const TAPS: usize = 101;

    fn tone(count: usize, frequency_hz: f64) -> Vec<Complex32> {
        let step = std::f64::consts::TAU * frequency_hz / f64::from(SAMPLE_RATE);

        (0..count)
            .map(|index| {
                let phase = step * index_to_f64(index);

                Complex32::new(narrow(phase.cos()), narrow(phase.sin()))
            })
            .collect()
    }

    fn power(samples: &[Complex32]) -> f64 {
        let total: f64 = samples.iter().map(|s| f64::from(s.norm_sqr())).sum();

        total / index_to_f64(samples.len().max(1))
    }

    #[test]
    fn the_design_has_unit_gain_at_dc() {
        let taps = low_pass(CUTOFF, SAMPLE_RATE, TAPS);
        let sum: f32 = taps.iter().sum();

        assert!((sum - 1.0).abs() < 1e-5, "DC gain is {sum}, not 1");
    }

    #[test]
    fn the_design_passes_the_passband_and_stops_the_stopband() {
        let taps = low_pass(CUTOFF, SAMPLE_RATE, TAPS);

        for frequency in [0.0, 20_000.0, 60_000.0] {
            let response = response_db(&taps, frequency, SAMPLE_RATE);
            assert!(
                response > -1.0,
                "{frequency} Hz is in the passband but reads {response} dB"
            );
        }

        for frequency in [200_000.0, 400_000.0, 1_000_000.0] {
            let response = response_db(&taps, frequency, SAMPLE_RATE);
            assert!(
                response < -50.0,
                "{frequency} Hz is in the stopband but reads {response} dB"
            );
        }
    }

    #[test]
    fn the_cutoff_is_the_six_db_point() {
        let taps = low_pass(CUTOFF, SAMPLE_RATE, TAPS);
        let response = response_db(&taps, CUTOFF, SAMPLE_RATE);

        assert!(
            (-8.0..-4.0).contains(&response),
            "the cutoff reads {response} dB, expected roughly -6 dB"
        );
    }

    #[test]
    fn decimation_produces_one_output_per_factor_inputs() {
        let mut decimator = Decimator::low_pass(CUTOFF, SAMPLE_RATE, 10, TAPS);
        let mut out = Vec::new();
        decimator.process(&tone(10_000, 0.0), &mut out);

        let expected = (10_000 - (TAPS - 1)) / 10;
        assert!(
            out.len().abs_diff(expected) <= 1,
            "{} outputs from 10000 inputs, expected about {expected}",
            out.len()
        );
    }

    #[test]
    fn the_output_rate_does_not_drift_across_block_boundaries() {
        let samples = tone(30_000, 0.0);

        let mut whole = Decimator::low_pass(CUTOFF, SAMPLE_RATE, 10, TAPS);
        let mut expected = Vec::new();
        whole.process(&samples, &mut expected);

        let mut split = Decimator::low_pass(CUTOFF, SAMPLE_RATE, 10, TAPS);
        let mut actual = Vec::new();
        for chunk in samples.chunks(313) {
            split.process(chunk, &mut actual);
        }

        assert_eq!(expected.len(), actual.len());
        for (index, (a, b)) in expected.iter().zip(&actual).enumerate() {
            assert!(
                (a - b).norm() < 1e-4,
                "sample {index} differs: {a} against {b}"
            );
        }
    }

    #[test]
    fn a_passband_tone_survives_and_a_stopband_tone_does_not() {
        let mut passband = Decimator::low_pass(CUTOFF, SAMPLE_RATE, 10, TAPS);
        let mut kept = Vec::new();
        passband.process(&tone(20_000, 30_000.0), &mut kept);

        let mut stopband = Decimator::low_pass(CUTOFF, SAMPLE_RATE, 10, TAPS);
        let mut rejected = Vec::new();
        stopband.process(&tone(20_000, 600_000.0), &mut rejected);

        assert!(power(&kept) > 0.9, "the passband tone lost too much power");
        assert!(
            power(&rejected) < 1e-4,
            "the stopband tone survived with {} power",
            power(&rejected)
        );
    }

    #[test]
    fn a_degenerate_design_is_a_pass_through() {
        assert_eq!(low_pass(0.0, 0, 0), vec![1.0]);

        let mut decimator = Decimator::new(Vec::new(), 4);
        let mut out = Vec::new();
        decimator.process(&tone(16, 0.0), &mut out);

        assert_eq!(out.len(), 16);
    }
}
