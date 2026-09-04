use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

const INGEST_STALL: Duration = Duration::from_mins(15);

const DETECTOR_STALL: Duration = Duration::from_mins(3);

const REJECTION_RATE: f64 = 0.33;

const MIN_CALLS_FOR_RATE: u64 = 20;

/// Live counters, shared by every handler.
#[derive(Debug, Default)]
pub struct Metrics {
    pub measurements_accepted: AtomicU64,
    pub health_accepted: AtomicU64,
    pub registrations: AtomicU64,
    pub ingest_rejected: AtomicU64,
    pub ingest_failed: AtomicU64,
    pub last_ingest_at: AtomicI64,
    pub last_detection_at: AtomicI64,
    pub last_maintenance_at: AtomicI64,
    pub alarms_raised: AtomicU64,
    pub http_requests: AtomicU64,
    pub http_server_errors: AtomicU64,
    /// Total time spent in HTTP handlers, in microseconds.
    pub http_micros: AtomicU64,
    /// Longest single HTTP handler, in microseconds.
    pub http_slowest_micros: AtomicU64,
}

impl Metrics {
    /// A fresh, shareable set of counters.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Record an accepted measurement report.
    pub fn measurement_accepted(&self) {
        self.measurements_accepted.fetch_add(1, Ordering::Relaxed);
        self.stamp_ingest();
    }

    /// Record an accepted health report.
    pub fn health_accepted(&self) {
        self.health_accepted.fetch_add(1, Ordering::Relaxed);
        self.stamp_ingest();
    }

    /// Record a node registration.
    pub fn registered(&self) {
        self.registrations.fetch_add(1, Ordering::Relaxed);
        self.stamp_ingest();
    }

    /// Record an ingest call the caller could have made correctly.
    pub fn ingest_rejected(&self) {
        self.ingest_rejected.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an ingest call that failed inside the server.
    pub fn ingest_failed(&self) {
        self.ingest_failed.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a finished detection pass and what it raised.
    pub fn detection_finished(&self, raised: usize) {
        self.last_detection_at
            .store(Utc::now().timestamp(), Ordering::Relaxed);
        self.alarms_raised
            .fetch_add(raised.try_into().unwrap_or(u64::MAX), Ordering::Relaxed);
    }

    /// Record a finished maintenance pass.
    pub fn maintenance_finished(&self) {
        self.last_maintenance_at
            .store(Utc::now().timestamp(), Ordering::Relaxed);
    }

    /// Record one served HTTP request.
    pub fn http_served(&self, micros: u64, status: u16) {
        self.http_requests.fetch_add(1, Ordering::Relaxed);
        self.http_micros.fetch_add(micros, Ordering::Relaxed);
        self.http_slowest_micros
            .fetch_max(micros, Ordering::Relaxed);

        if status >= 500 {
            self.http_server_errors.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn stamp_ingest(&self) {
        self.last_ingest_at
            .store(Utc::now().timestamp(), Ordering::Relaxed);
    }

    /// Take a consistent-enough snapshot for the ops endpoint.
    #[must_use]
    pub fn snapshot(&self, pool: &deadpool_diesel::postgres::Pool) -> Snapshot {
        let status = pool.status();
        let requests = self.http_requests.load(Ordering::Relaxed);
        let micros = self.http_micros.load(Ordering::Relaxed);

        Snapshot {
            measurements_accepted: self.measurements_accepted.load(Ordering::Relaxed),
            health_accepted: self.health_accepted.load(Ordering::Relaxed),
            registrations: self.registrations.load(Ordering::Relaxed),
            ingest_rejected: self.ingest_rejected.load(Ordering::Relaxed),
            ingest_failed: self.ingest_failed.load(Ordering::Relaxed),
            last_ingest_at: instant(self.last_ingest_at.load(Ordering::Relaxed)),
            last_detection_at: instant(self.last_detection_at.load(Ordering::Relaxed)),
            last_maintenance_at: instant(self.last_maintenance_at.load(Ordering::Relaxed)),
            alarms_raised: self.alarms_raised.load(Ordering::Relaxed),
            http_requests: requests,
            http_server_errors: self.http_server_errors.load(Ordering::Relaxed),
            http_mean_micros: mean(micros, requests),
            http_slowest_micros: self.http_slowest_micros.load(Ordering::Relaxed),
            pool_size: status.size,
            pool_available: status.available,
            pool_waiting: status.waiting,
        }
    }
}

/// One reading of every counter, plus the pool's own state.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Snapshot {
    pub measurements_accepted: u64,
    pub health_accepted: u64,
    pub registrations: u64,
    pub ingest_rejected: u64,
    pub ingest_failed: u64,
    pub last_ingest_at: Option<NaiveDateTime>,
    pub last_detection_at: Option<NaiveDateTime>,
    pub last_maintenance_at: Option<NaiveDateTime>,
    pub alarms_raised: u64,
    pub http_requests: u64,
    pub http_server_errors: u64,
    /// Mean handler time in microseconds, over every request since start.
    pub http_mean_micros: u64,
    pub http_slowest_micros: u64,
    pub pool_size: usize,
    pub pool_available: usize,
    pub pool_waiting: usize,
}

impl Snapshot {
    #[must_use]
    pub fn problems(&self, now: NaiveDateTime, fleet_size: u64) -> Vec<String> {
        let mut problems = Vec::new();

        if fleet_size > 0 {
            match self.last_ingest_at {
                None => {
                    problems
                        .push("ingest has accepted nothing since the server started".to_owned());
                }
                Some(at) if stale(now, at, INGEST_STALL) => problems.push(format!(
                    "ingest has accepted nothing since {at}, which is more than {} minutes",
                    INGEST_STALL.as_secs() / 60
                )),
                Some(_) => {}
            }
        }

        if let Some(at) = self.last_detection_at
            && stale(now, at, DETECTOR_STALL)
        {
            problems.push(format!("the detector last finished a pass at {at}"));
        }

        let calls = self
            .measurements_accepted
            .saturating_add(self.health_accepted)
            .saturating_add(self.registrations)
            .saturating_add(self.ingest_rejected)
            .saturating_add(self.ingest_failed);
        if calls >= MIN_CALLS_FOR_RATE {
            let rejected = self.ingest_rejected.saturating_add(self.ingest_failed);
            let rate = ratio(rejected, calls);
            if rate > REJECTION_RATE {
                problems.push(format!(
                    "{:.0}% of ingest calls are being refused",
                    rate * 100.0
                ));
            }
        }

        if self.ingest_failed > 0 && self.measurements_accepted == 0 {
            problems.push("every ingest call has failed inside the server".to_owned());
        }

        problems
    }

    /// Whether the server considers itself healthy.
    #[must_use]
    pub fn is_healthy(&self, now: NaiveDateTime, fleet_size: u64) -> bool {
        self.problems(now, fleet_size).is_empty()
    }
}

fn instant(seconds: i64) -> Option<NaiveDateTime> {
    if seconds == 0 {
        return None;
    }

    DateTime::from_timestamp(seconds, 0).map(|at| at.naive_utc())
}

fn stale(now: NaiveDateTime, at: NaiveDateTime, limit: Duration) -> bool {
    now.signed_duration_since(at)
        .to_std()
        .is_ok_and(|elapsed| elapsed > limit)
}

const fn mean(total: u64, count: u64) -> u64 {
    match total.checked_div(count) {
        Some(mean) => mean,
        None => 0,
    }
}

/// A ratio of two counts.
fn ratio(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        return 0.0;
    }

    let part = u32::try_from(part.min(u64::from(u32::MAX))).unwrap_or(u32::MAX);
    let whole = u32::try_from(whole.min(u64::from(u32::MAX))).unwrap_or(u32::MAX);

    f64::from(part) / f64::from(whole)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minutes: i64) -> NaiveDateTime {
        DateTime::from_timestamp(minutes.saturating_mul(60), 0)
            .expect("a real instant")
            .naive_utc()
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            measurements_accepted: 100,
            health_accepted: 10,
            registrations: 1,
            ingest_rejected: 0,
            ingest_failed: 0,
            last_ingest_at: Some(at(100)),
            last_detection_at: Some(at(100)),
            last_maintenance_at: Some(at(60)),
            alarms_raised: 0,
            http_requests: 50,
            http_server_errors: 0,
            http_mean_micros: 1_200,
            http_slowest_micros: 9_000,
            pool_size: 8,
            pool_available: 8,
            pool_waiting: 0,
        }
    }

    #[test]
    fn a_working_server_reports_no_problems() {
        assert!(snapshot().is_healthy(at(101), 3));
    }

    #[test]
    fn a_stalled_ingest_is_a_problem() {
        let problems = snapshot().problems(at(200), 3);

        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems.iter().any(|line| line.contains("ingest")));
    }

    #[test]
    fn an_empty_fleet_cannot_stall_ingest() {
        let quiet = Snapshot {
            last_ingest_at: None,
            ..snapshot()
        };

        assert!(quiet.problems(at(101), 0).is_empty());
        assert!(!quiet.problems(at(101), 1).is_empty());
    }

    #[test]
    fn a_stuck_detector_is_a_problem() {
        let problems = snapshot().problems(at(110), 3);

        assert!(
            problems.iter().any(|line| line.contains("detector")),
            "{problems:?}"
        );
    }

    #[test]
    fn a_few_rejections_are_normal() {
        let noisy = Snapshot {
            measurements_accepted: 100,
            ingest_rejected: 5,
            ..snapshot()
        };

        assert!(
            noisy.is_healthy(at(101), 3),
            "{:?}",
            noisy.problems(at(101), 3)
        );
    }

    #[test]
    fn a_flood_of_rejections_is_not() {
        let broken = Snapshot {
            measurements_accepted: 10,
            health_accepted: 0,
            registrations: 0,
            ingest_rejected: 40,
            ..snapshot()
        };

        assert!(
            broken
                .problems(at(101), 3)
                .iter()
                .any(|line| line.contains("refused")),
            "{:?}",
            broken.problems(at(101), 3)
        );
    }

    #[test]
    fn a_handful_of_calls_does_not_trigger_a_rate() {
        let early = Snapshot {
            measurements_accepted: 1,
            health_accepted: 0,
            registrations: 0,
            ingest_rejected: 2,
            ingest_failed: 0,
            ..snapshot()
        };

        assert!(
            !early
                .problems(at(101), 3)
                .iter()
                .any(|line| line.contains("refused"))
        );
    }

    #[test]
    fn a_never_stamped_counter_reads_as_never() {
        assert_eq!(instant(0), None);
        assert!(instant(1_700_000_000).is_some());
    }

    #[test]
    fn the_counters_count() {
        let metrics = Metrics::new();

        metrics.measurement_accepted();
        metrics.measurement_accepted();
        metrics.health_accepted();
        metrics.registered();
        metrics.ingest_rejected();
        metrics.ingest_failed();
        metrics.detection_finished(3);
        metrics.http_served(500, 200);
        metrics.http_served(1_500, 503);

        assert_eq!(metrics.measurements_accepted.load(Ordering::Relaxed), 2);
        assert_eq!(metrics.health_accepted.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.registrations.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.ingest_rejected.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.ingest_failed.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.alarms_raised.load(Ordering::Relaxed), 3);
        assert_eq!(metrics.http_requests.load(Ordering::Relaxed), 2);
        assert_eq!(metrics.http_server_errors.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.http_slowest_micros.load(Ordering::Relaxed), 1_500);
        assert!(metrics.last_ingest_at.load(Ordering::Relaxed) > 0);
    }

    #[test]
    fn a_mean_over_nothing_is_zero() {
        assert_eq!(mean(0, 0), 0);
        assert_eq!(mean(1_000, 4), 250);
    }

    #[test]
    fn a_ratio_of_nothing_is_zero() {
        assert!(ratio(0, 0).abs() < f64::EPSILON);
        assert!((ratio(1, 4) - 0.25).abs() < f64::EPSILON);
    }
}
