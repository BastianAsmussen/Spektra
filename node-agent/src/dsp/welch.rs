use num_complex::Complex32;
use spektra_fft::numbers::complex::Complex;
use spektra_fft::{Fft, FftError};

use super::convert::{index_to_f32, index_to_f64, to_index};
use super::window::Window;

const OVERLAP: usize = 2;

/// A streaming Welch estimator for one transform length.
pub struct Welch {
    plan: Fft<f32>,
    window: Vec<f32>,
    window_power: f32,
    scratch: Vec<Complex<f32>>,
    accumulator: Vec<f32>,
    carry: Vec<Complex32>,
    segments: u64,
    sample_rate_hz: u32,
}

impl Welch {
    /// Plan an estimator of `fft_size` bins at `sample_rate_hz`.
    ///
    /// # Errors
    ///
    /// [`FftError::NotPowerOfTwo`] if `fft_size` is not one.
    pub fn new(fft_size: usize, sample_rate_hz: u32, window: Window) -> Result<Self, FftError> {
        let plan = Fft::<f32>::new(fft_size)?;
        let coefficients = window.coefficients(fft_size);

        Ok(Self {
            plan,
            window_power: Window::power(&coefficients),
            window: coefficients,
            scratch: vec![Complex::new(0.0, 0.0); fft_size],
            accumulator: vec![0.0; fft_size],
            carry: Vec::with_capacity(fft_size),
            segments: 0,
            sample_rate_hz,
        })
    }

    /// The transform length this estimator was planned for.
    #[must_use]
    pub const fn fft_size(&self) -> usize {
        self.plan.size()
    }

    /// How many segments have been folded in since the last [`Welch::finish`].
    #[must_use]
    pub const fn segments(&self) -> u64 {
        self.segments
    }

    /// Fold a block of samples in, however many segments it completes.
    pub fn push(&mut self, block: &[Complex32]) {
        let size = self.fft_size();
        let hop = size / OVERLAP;

        let mut carry = std::mem::take(&mut self.carry);
        carry.extend_from_slice(block);

        let mut consumed: usize = 0;
        while let Some(segment) = carry.get(consumed..consumed.saturating_add(size)) {
            self.absorb(segment);
            consumed = consumed.saturating_add(hop);
        }

        carry.drain(..consumed.min(carry.len()));
        self.carry = carry;
    }

    /// Average everything folded in so far and reset for the next dwell.
    #[must_use]
    pub fn finish(&mut self) -> Option<Psd> {
        if self.segments == 0 {
            self.carry.clear();

            return None;
        }

        let size = self.fft_size();
        let half = size / OVERLAP;

        let normalizer = index_to_f32(size) * self.window_power * segments_to_f32(self.segments);
        let scale = if normalizer > 0.0 {
            normalizer.recip()
        } else {
            0.0
        };

        let (positive, negative) = self.accumulator.split_at(half);
        let bins: Vec<f32> = negative
            .iter()
            .chain(positive)
            .map(|power| power * scale)
            .collect();

        let psd = Psd {
            bin_width_hz: if bins.is_empty() {
                0.0
            } else {
                f64::from(self.sample_rate_hz) / index_to_f64(bins.len())
            },
            center: index_to_f64(half),
            bins,
            sample_rate_hz: self.sample_rate_hz,
            segments: self.segments,
        };

        self.accumulator.iter_mut().for_each(|bin| *bin = 0.0);
        self.carry.clear();
        self.segments = 0;

        Some(psd)
    }

    fn absorb(&mut self, segment: &[Complex32]) {
        for ((slot, sample), coefficient) in self.scratch.iter_mut().zip(segment).zip(&self.window)
        {
            *slot = Complex::new(sample.re * coefficient, sample.im * coefficient);
        }

        if let Err(err) = self.plan.transform_in_place(&mut self.scratch) {
            tracing::error!(error = %err, "the spectrum estimator was handed a mismatched buffer");

            return;
        }

        for (bin, value) in self.accumulator.iter_mut().zip(&self.scratch) {
            *bin += value.norm_sqr();
        }

        self.segments = self.segments.saturating_add(1);
    }
}

fn segments_to_f32(segments: u64) -> f32 {
    index_to_f32(usize::try_from(segments).unwrap_or(usize::MAX))
}

/// One averaged power spectrum, in frequency order from `-fs/2`.
#[derive(Debug, Clone)]
pub struct Psd {
    bins: Vec<f32>,
    sample_rate_hz: u32,
    bin_width_hz: f64,
    center: f64,
    segments: u64,
}

impl Psd {
    /// The bins, lowest frequency first.
    #[must_use]
    pub fn bins(&self) -> &[f32] {
        &self.bins
    }

    /// The bins of one band, bounds-checked once.
    #[must_use]
    pub fn band_bins(&self, band: &std::ops::Range<usize>) -> &[f32] {
        self.bins.get(band.clone()).unwrap_or_default()
    }

    /// How many segments were averaged into this estimate.
    #[must_use]
    pub const fn segments(&self) -> u64 {
        self.segments
    }

    /// The rate the spectrum was sampled at, in samples per second.
    #[must_use]
    pub const fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }

    /// Width of one bin, in Hz.
    #[must_use]
    pub const fn bin_width_hz(&self) -> f64 {
        self.bin_width_hz
    }

    /// Frequency of bin `index` relative to the tuned frequency, in Hz.
    #[must_use]
    pub fn offset_hz(&self, index: usize) -> f64 {
        (index_to_f64(index) - self.center) * self.bin_width_hz
    }

    /// The bin covering an offset from the tuned frequency, clamped to the spectrum.
    #[must_use]
    pub fn index_at(&self, offset_hz: f64) -> usize {
        if self.bin_width_hz <= 0.0 || !offset_hz.is_finite() {
            return 0;
        }

        to_index(offset_hz / self.bin_width_hz + self.center, self.bins.len())
    }

    /// The half-open range of bins covering `+-half_width_hz` around the tuned frequency.
    #[must_use]
    pub fn band(&self, half_width_hz: f64) -> std::ops::Range<usize> {
        if self.bin_width_hz <= 0.0 || !half_width_hz.is_finite() || half_width_hz <= 0.0 {
            return 0..0;
        }

        let center = self.bins.len() / OVERLAP;
        let reach = to_index(half_width_hz / self.bin_width_hz, self.bins.len());

        let start = center.saturating_sub(reach);
        let end = center
            .saturating_add(reach)
            .saturating_add(1)
            .min(self.bins.len());

        start..end
    }

    /// Total power in a range of bins.
    #[must_use]
    pub fn power_in(&self, range: std::ops::Range<usize>) -> f64 {
        self.bins
            .get(range)
            .map(|slice| slice.iter().map(|power| f64::from(*power)).sum())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(count: usize, sample_rate_hz: u32, offset_hz: f64) -> Vec<Complex32> {
        let step = std::f64::consts::TAU * offset_hz / f64::from(sample_rate_hz);

        (0..count)
            .map(|index| {
                let phase = step * index_to_f64(index);

                Complex32::new(
                    super::super::convert::narrow(phase.cos()),
                    super::super::convert::narrow(phase.sin()),
                )
            })
            .collect()
    }

    #[test]
    fn a_dwell_shorter_than_a_segment_is_not_a_measurement() {
        let mut welch = Welch::new(1024, 48_000, Window::Hann).expect("1024 is a power of two");
        welch.push(&tone(512, 48_000, 0.0));

        assert!(welch.finish().is_none());
    }

    #[test]
    fn a_full_scale_carrier_reads_zero_dbfs() {
        let mut welch = Welch::new(1024, 48_000, Window::Hann).expect("a power of two");
        welch.push(&tone(8192, 48_000, 0.0));

        let psd = welch.finish().expect("segments were pushed");
        let total = psd.power_in(0..psd.bins().len());

        assert!(
            (10.0 * total.log10()).abs() < 0.1,
            "a unit-amplitude carrier should read 0 dBFS, read {} dBFS",
            10.0 * total.log10()
        );
    }

    #[test]
    fn a_carrier_lands_in_the_bin_its_frequency_names() {
        let sample_rate = 48_000;
        let size = 1024;
        let offset = 12.0 * f64::from(sample_rate) / index_to_f64(size);

        let mut welch = Welch::new(size, sample_rate, Window::Hann).expect("a power of two");
        welch.push(&tone(size * 8, sample_rate, offset));
        let psd = welch.finish().expect("segments were pushed");

        let peak = psd
            .bins()
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(index, _)| index)
            .expect("the spectrum is not empty");

        assert!(
            (psd.offset_hz(peak) - offset).abs() < psd.bin_width_hz(),
            "peak at {} Hz, expected {offset} Hz",
            psd.offset_hz(peak)
        );
    }

    #[test]
    fn bin_zero_is_the_bottom_of_the_band() {
        let mut welch = Welch::new(256, 2_400_000, Window::Hann).expect("a power of two");
        welch.push(&tone(2048, 2_400_000, 0.0));
        let psd = welch.finish().expect("segments were pushed");

        assert!((psd.offset_hz(0) + 1_200_000.0).abs() < f64::EPSILON);
        assert!(psd.offset_hz(128).abs() < f64::EPSILON);
        assert!((psd.bin_width_hz() - 2_400_000.0 / 256.0).abs() < f64::EPSILON);
    }

    #[test]
    fn the_in_band_range_covers_the_requested_width() {
        let mut welch = Welch::new(1024, 2_400_000, Window::Hann).expect("a power of two");
        welch.push(&tone(4096, 2_400_000, 0.0));
        let psd = welch.finish().expect("segments were pushed");

        let band = psd.band(90_000.0);
        let width = psd.bin_width_hz();

        assert!(psd.offset_hz(band.start) <= -90_000.0 + width);
        assert!(psd.offset_hz(band.end.saturating_sub(1)) >= 90_000.0 - width);
    }

    #[test]
    fn finishing_twice_does_not_double_count() {
        let mut welch = Welch::new(512, 48_000, Window::Hann).expect("a power of two");
        welch.push(&tone(4096, 48_000, 0.0));

        let first = welch.finish().expect("segments were pushed");
        assert!(welch.finish().is_none());
        assert!(first.segments() > 1);
    }

    #[test]
    fn blocks_are_stitched_across_pushes() {
        let sample_rate = 48_000;
        let size = 256;
        let samples = tone(size * 4, sample_rate, 0.0);

        let mut whole = Welch::new(size, sample_rate, Window::Hann).expect("a power of two");
        whole.push(&samples);
        let expected = whole.finish().expect("segments were pushed");

        let mut split = Welch::new(size, sample_rate, Window::Hann).expect("a power of two");
        for chunk in samples.chunks(37) {
            split.push(chunk);
        }
        let actual = split.finish().expect("segments were pushed");

        assert_eq!(expected.segments(), actual.segments());
        for (bin, (a, b)) in expected.bins().iter().zip(actual.bins()).enumerate() {
            assert!((a - b).abs() < 1e-9, "bin {bin}: {a} != {b}");
        }
    }
}
