use num_complex::Complex32;

use super::convert::narrow;

/// A stateful FM discriminator, carrying the last sample across blocks.
#[derive(Debug, Clone)]
pub struct Discriminator {
    previous: Option<Complex32>,
    /// Radians per sample to Hz.
    scale: f32,
}

impl Discriminator {
    /// A discriminator for a stream at `sample_rate_hz`.
    #[must_use]
    pub fn new(sample_rate_hz: u32) -> Self {
        Self {
            previous: None,
            scale: narrow(f64::from(sample_rate_hz) / std::f64::consts::TAU),
        }
    }

    /// Demodulate a block, appending instantaneous frequency in Hz to `out`.
    pub fn process(&mut self, block: &[Complex32], out: &mut Vec<f32>) {
        let Some((first, rest)) = block.split_first() else {
            return;
        };

        out.reserve(block.len());

        if let Some(previous) = self.previous {
            out.push(discriminate(*first, previous, self.scale));
        }

        for (sample, previous) in rest.iter().zip(block) {
            out.push(discriminate(*sample, *previous, self.scale));
        }

        self.previous = block.last().copied();
    }

    /// Forget the reference sample, for a retune.
    pub const fn reset(&mut self) {
        self.previous = None;
    }
}

fn discriminate(sample: Complex32, previous: Complex32, scale: f32) -> f32 {
    let real = sample.re.mul_add(previous.re, sample.im * previous.im);
    let imaginary = sample.im.mul_add(previous.re, -(sample.re * previous.im));

    imaginary.atan2(real) * scale
}

/// Demodulate a whole block at once.
#[must_use]
pub fn demodulate(iq: &[Complex32], sample_rate_hz: u32) -> Vec<f32> {
    let mut out = Vec::with_capacity(iq.len().saturating_sub(1));
    Discriminator::new(sample_rate_hz).process(iq, &mut out);

    out
}

#[cfg(test)]
mod tests {
    use super::super::convert::index_to_f64;
    use super::*;

    const SAMPLE_RATE: u32 = 240_000;

    fn modulated(count: usize, tone_hz: f64, deviation_hz: f64) -> Vec<Complex32> {
        let tone_step = std::f64::consts::TAU * tone_hz / f64::from(SAMPLE_RATE);
        let index = deviation_hz / tone_hz;

        (0..count)
            .map(|sample| {
                let phase = index * (tone_step * index_to_f64(sample)).sin();

                Complex32::new(narrow(phase.cos()), narrow(phase.sin()))
            })
            .collect()
    }

    fn peak_deviation(baseband: &[f32]) -> f64 {
        baseband
            .iter()
            .map(|value| f64::from(value.abs()))
            .fold(0.0, f64::max)
    }

    #[test]
    fn an_unmodulated_carrier_demodulates_to_its_offset() {
        let offset = 5_000.0;
        let step = std::f64::consts::TAU * offset / f64::from(SAMPLE_RATE);
        let carrier: Vec<Complex32> = (0..4096)
            .map(|sample| {
                let phase = step * index_to_f64(sample);

                Complex32::new(narrow(phase.cos()), narrow(phase.sin()))
            })
            .collect();

        let baseband = demodulate(&carrier, SAMPLE_RATE);
        let mean = baseband.iter().map(|v| f64::from(*v)).sum::<f64>()
            / index_to_f64(baseband.len().max(1));

        assert!(
            (mean - offset).abs() < 1.0,
            "an unmodulated carrier {offset} Hz off should demodulate to a constant {offset} Hz, got {mean}"
        );
    }

    #[test]
    fn the_recovered_deviation_matches_the_injected_one() {
        for deviation in [10_000.0, 45_000.0, 75_000.0] {
            let baseband = demodulate(&modulated(48_000, 1_000.0, deviation), SAMPLE_RATE);
            let recovered = peak_deviation(&baseband);

            assert!(
                (recovered - deviation).abs() / deviation < 0.02,
                "injected {deviation} Hz deviation, recovered {recovered} Hz"
            );
        }
    }

    #[test]
    fn the_output_is_one_sample_shorter_than_the_input() {
        assert_eq!(
            demodulate(&modulated(1000, 1_000.0, 50_000.0), SAMPLE_RATE).len(),
            999
        );
    }

    #[test]
    fn a_split_stream_demodulates_the_same_as_a_whole_one() {
        let samples = modulated(8192, 400.0, 60_000.0);

        let expected = demodulate(&samples, SAMPLE_RATE);

        let mut discriminator = Discriminator::new(SAMPLE_RATE);
        let mut actual = Vec::new();
        for chunk in samples.chunks(97) {
            discriminator.process(chunk, &mut actual);
        }

        assert_eq!(expected.len(), actual.len());
        for (index, (a, b)) in expected.iter().zip(&actual).enumerate() {
            assert!((a - b).abs() < 1e-3, "sample {index}: {a} != {b}");
        }
    }
}
