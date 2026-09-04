use std::f64::consts::TAU;

const DAY_SECONDS: f64 = 86_400.0;
const DIURNAL_DB: f64 = 1.6;
const FAULT_DEPTH_DB: f64 = 7.5;

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

const SPLITMIX_GAMMA: u64 = 0x9e37_79b9_7f4a_7c15;
const SPLITMIX_MUL_1: u64 = 0xbf58_476d_1ce4_e5b9;
const SPLITMIX_MUL_2: u64 = 0x94d0_49bb_1331_11eb;

/// One node's generator, seeded from its identity.
#[derive(Debug, Clone, Copy)]
pub struct Signal {
    seed: u64,
}

/// One window of metrics, in protocol units.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub strength_db: f64,
    pub snr_db: f64,
    pub offset_hz: f64,
    pub error_rate: f64,
    pub occupancy: f64,
}

impl Signal {
    /// Seed from the node identity (FNV-1a).
    #[must_use]
    pub fn for_identity(identity: &str) -> Self {
        let mut seed: u64 = FNV_OFFSET_BASIS;
        for byte in identity.as_bytes() {
            seed ^= u64::from(*byte);
            seed = seed.wrapping_mul(FNV_PRIME);
        }

        Self { seed }
    }

    /// Roughly one in five identities is faulty.
    #[must_use]
    pub const fn is_faulty(self) -> bool {
        self.seed.is_multiple_of(5)
    }

    /// Metrics for a window that ended `age_seconds` ago.
    #[must_use]
    pub fn at(self, age_seconds: f64) -> Sample {
        let phase = (self.offset() - age_seconds / DAY_SECONDS) * TAU;
        let diurnal = phase.sin();

        let jitter = |stream: u64, scale: f64| self.noise(age_seconds, stream) * scale;

        let fault = self.fault_depth(age_seconds);
        let snr_db = diurnal.mul_add(0.9, 28.4) + jitter(1, 0.45) - fault;

        let headroom = (snr_db - 18.0).max(0.0);
        let error_rate = (0.35f64.mul_add(1.0 / headroom.mul_add(headroom, 1.0), 0.001)
            + jitter(2, 0.0008))
        .clamp(0.0, 1.0);

        Sample {
            strength_db: fault.mul_add(-0.35, diurnal.mul_add(DIURNAL_DB, -46.3) + jitter(0, 0.55)),
            snr_db,
            offset_hz: diurnal.mul_add(22.0, 85.0) + jitter(3, 12.0),
            error_rate,
            occupancy: (diurnal.mul_add(0.03, 0.83) + jitter(4, 0.015)).clamp(0.0, 1.0),
        }
    }

    fn offset(self) -> f64 {
        Self::unit(self.seed)
    }

    fn fault_depth(self, age_seconds: f64) -> f64 {
        if !self.is_faulty() {
            return 0.0;
        }

        let span = 5_400.0;
        if age_seconds > span {
            return 0.0;
        }

        let progress = 1.0 - age_seconds / span;

        FAULT_DEPTH_DB * (1.0 - (progress * TAU / 2.0).cos()) / 2.0
    }

    fn noise(self, age_seconds: f64, stream: u64) -> f64 {
        let tick = (age_seconds / 60.0).round().abs().min(4.0e9);
        let mut state = self
            .seed
            .wrapping_mul(SPLITMIX_GAMMA)
            .wrapping_add(stream.wrapping_mul(SPLITMIX_MUL_1))
            .wrapping_add(tick_bits(tick));

        state ^= state >> 30;
        state = state.wrapping_mul(SPLITMIX_MUL_1);
        state ^= state >> 27;
        state = state.wrapping_mul(SPLITMIX_MUL_2);
        state ^= state >> 31;

        Self::unit(state).mul_add(2.0, -1.0)
    }

    fn unit(state: u64) -> f64 {
        f64::from(u32::try_from(state >> 32).unwrap_or(0)) / f64::from(u32::MAX)
    }
}

fn tick_bits(tick: f64) -> u64 {
    if tick.is_finite() && tick >= 0.0 {
        u64::from(f64_to_u32(tick))
    } else {
        0
    }
}

#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped into u32's range by the caller, so the narrowing is exact"
)]
fn f64_to_u32(value: f64) -> u32 {
    value.clamp(0.0, f64::from(u32::MAX)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_identity_and_age_give_the_same_sample() {
        let signal = Signal::for_identity("abc123");

        let first = signal.at(600.0);
        let second = signal.at(600.0);

        assert!((first.strength_db - second.strength_db).abs() < f64::EPSILON);
        assert!((first.snr_db - second.snr_db).abs() < f64::EPSILON);
    }

    #[test]
    fn two_identities_do_not_share_weather() {
        let one = Signal::for_identity("node-one").at(0.0);
        let two = Signal::for_identity("node-two").at(0.0);

        assert!((one.strength_db - two.strength_db).abs() > 1e-6);
    }

    #[test]
    fn every_metric_stays_inside_its_unit() {
        for identity in ["a", "b", "c", "d", "e", "f", "g", "h"] {
            let signal = Signal::for_identity(identity);

            for step in 0..600 {
                let sample = signal.at(f64::from(step) * 60.0);

                assert!(
                    sample.strength_db < 0.0,
                    "{identity}: strength went positive"
                );
                assert!(sample.snr_db > 0.0, "{identity}: snr went negative");
                assert!(
                    (0.0..=1.0).contains(&sample.error_rate),
                    "{identity}: error rate escaped"
                );
                assert!(
                    (0.0..=1.0).contains(&sample.occupancy),
                    "{identity}: occupancy escaped"
                );
            }
        }
    }

    #[test]
    fn a_faulty_node_is_worse_now_than_it_was_yesterday() {
        let faulty = (0..64)
            .map(|n| Signal::for_identity(&format!("n{n}")))
            .find(|signal| signal.is_faulty())
            .expect("one identity in five is faulty");

        let during = faulty.at(0.0).snr_db;
        let before = faulty.at(86_400.0).snr_db;

        assert!(
            before - during > 4.0,
            "the dip is not deep enough to detect: {before} then {during}"
        );
    }

    #[test]
    fn a_healthy_node_has_no_dip() {
        let healthy = (0..64)
            .map(|n| Signal::for_identity(&format!("n{n}")))
            .find(|signal| !signal.is_faulty())
            .expect("most identities are healthy");

        let during = healthy.at(0.0).snr_db;
        let before = healthy.at(86_400.0).snr_db;

        assert!((before - during).abs() < 3.0, "a healthy node drifted");
    }

    #[test]
    fn the_error_rate_follows_the_ratio_that_causes_it() {
        let faulty = (0..64)
            .map(|n| Signal::for_identity(&format!("n{n}")))
            .find(|signal| signal.is_faulty())
            .expect("one identity in five is faulty");

        assert!(faulty.at(0.0).error_rate > faulty.at(86_400.0).error_rate);
    }
}
