use super::convert::index_to_f32;

/// Window functions applied before each FFT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Window {
    /// Rectangular, meaning no window at all. Kept for comparison in tests.
    Rectangular,
    /// Hann, periodic form.
    Hann,
}

impl Window {
    /// The window's coefficients for a segment of `length` samples.
    #[must_use]
    pub fn coefficients(self, length: usize) -> Vec<f32> {
        match self {
            Self::Rectangular => vec![1.0; length],
            Self::Hann => {
                let n = index_to_f32(length);

                (0..length)
                    .map(|index| {
                        let phase = std::f32::consts::TAU * index_to_f32(index) / n;

                        0.5 * (1.0 - phase.cos())
                    })
                    .collect()
            }
        }
    }

    /// `sum(w[n]^2)`, the normalizer that makes a windowed power spectrum comparable.
    #[must_use]
    pub fn power(coefficients: &[f32]) -> f32 {
        coefficients.iter().map(|value| value * value).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rectangular_window_is_all_ones() {
        assert_eq!(Window::Rectangular.coefficients(4), vec![1.0; 4]);
        assert!((Window::power(&Window::Rectangular.coefficients(8)) - 8.0).abs() < f32::EPSILON);
    }

    #[test]
    fn hann_starts_at_zero_and_peaks_in_the_middle() {
        let window = Window::Hann.coefficients(8);

        assert!(window[0].abs() < 1e-6);
        assert!((window[4] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn hann_is_symmetric_about_its_peak() {
        let window = Window::Hann.coefficients(16);

        for offset in 1..8 {
            assert!(
                (window[8 - offset] - window[8 + offset]).abs() < 1e-6,
                "the window is not symmetric at offset {offset}"
            );
        }
    }

    #[test]
    fn hann_carries_three_eighths_of_the_power_of_a_rectangle() {
        let window = Window::Hann.coefficients(1024);

        assert!((Window::power(&window) - 3.0 * 1024.0 / 8.0).abs() < 1e-2);
    }
}
