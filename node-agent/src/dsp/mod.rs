pub mod fir;
pub mod fm;
pub mod metrics;
pub mod rds;
pub mod welch;
pub mod window;

use protocol::v1::Metric;

pub use metrics::{ChannelSpec, derive};
pub use welch::{Psd, Welch};

/// Metrics this build actually derives, advertised at registration.
pub const DERIVED_METRICS: &[Metric] = &[
    Metric::SignalStrength,
    Metric::SignalToNoise,
    Metric::CarrierOffset,
    Metric::SpectrumOccupancy,
];

/// Numeric conversions with no lossless `From`.
pub mod convert {
    /// Widen an index or a count to `f32`.
    #[expect(
        clippy::as_conversions,
        clippy::cast_precision_loss,
        reason = "usize has no lossless From into f32; callers pass transform lengths and tap counts, all below 2^24"
    )]
    #[must_use]
    pub const fn index_to_f32(index: usize) -> f32 {
        index as f32
    }

    /// Widen an index or a count to `f64`.
    #[expect(
        clippy::as_conversions,
        clippy::cast_precision_loss,
        reason = "usize has no lossless From into f64; callers pass transform lengths and sample counts, all below 2^53"
    )]
    #[must_use]
    pub const fn index_to_f64(index: usize) -> f64 {
        index as f64
    }

    /// Narrow a `f64` to `f32`.
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "f32 has no From<f64>; the values narrowed here are configured quantities, not sample data"
    )]
    #[must_use]
    pub const fn narrow(value: f64) -> f32 {
        value as f32
    }

    /// Round a non-negative `f64` down to an index, saturating at `limit`.
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the value is checked to be finite and non-negative first, and the result is clamped to limit"
    )]
    #[must_use]
    pub fn to_index(value: f64, limit: usize) -> usize {
        if !value.is_finite() || value <= 0.0 {
            return 0;
        }

        let floored = value.floor();
        if floored >= index_to_f64(limit) {
            return limit;
        }

        (floored as usize).min(limit)
    }
}

#[cfg(test)]
mod tests {
    use super::convert::{index_to_f32, index_to_f64, narrow, to_index};

    #[test]
    fn indices_widen_exactly() {
        assert!((index_to_f32(4096) - 4096.0_f32).abs() < f32::EPSILON);
        assert!((index_to_f64(2_400_000) - 2_400_000.0_f64).abs() < f64::EPSILON);
    }

    #[test]
    fn narrowing_keeps_the_magnitude() {
        assert!((narrow(180_000.0) - 180_000.0_f32).abs() < f32::EPSILON);
    }

    #[test]
    fn an_index_saturates_at_the_limit() {
        assert_eq!(to_index(3.9, 10), 3);
        assert_eq!(to_index(-1.0, 10), 0);
        assert_eq!(to_index(f64::NAN, 10), 0);
        assert_eq!(to_index(1e30, 10), 10);
        assert_eq!(to_index(10.0, 10), 10);
    }

    #[test]
    fn the_demod_error_rate_is_not_advertised_yet() {
        assert!(!super::DERIVED_METRICS.contains(&protocol::v1::Metric::DemodErrorRate));
    }
}
