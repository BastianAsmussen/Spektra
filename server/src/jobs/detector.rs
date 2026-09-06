use chrono::{Days, NaiveDateTime, TimeDelta, Timelike as _};
use diesel::pg::PgConnection;
use diesel::prelude::*;
use serde_json::json;

use crate::db::models::alarm_events::NewAlarmEvent;
use crate::db::models::alarms::NewAlarm;
use crate::db::models::enums::{AlarmState, Metric, RollupResolution};
use crate::db::schema::alarm_events as alarm_events_schema;
use crate::db::schema::alarms as alarms_schema;
use crate::db::schema::measurements as measurements_schema;
use crate::db::schema::rollups as rollups_schema;

pub const BASELINE_DAYS: u64 = 28;

pub const MIN_BASELINE_BUCKETS: usize = 7;

/// Band half-width, in robust standard deviations.
pub const K: f64 = 4.0;

pub const CONSECUTIVE_WINDOWS: usize = 3;

const MAD_TO_SIGMA: f64 = 1.4826;

const MINIMUM_SPREAD: f64 = 1e-6;

/// What a node normally reads for one metric at one hour of the day.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Baseline {
    pub center: f64,
    pub between: f64,
    pub within: f64,
    pub buckets: usize,
}

impl Baseline {
    /// Half-width of the acceptance band.
    #[must_use]
    pub fn threshold(&self) -> f64 {
        let spread = self.between.hypot(self.within).max(MINIMUM_SPREAD);

        K * spread
    }

    #[must_use]
    pub fn band(&self) -> (f64, f64) {
        let threshold = self.threshold();

        (self.center - threshold, self.center + threshold)
    }

    #[must_use]
    pub fn is_deviation(&self, value: f64) -> bool {
        let (low, high) = self.band();

        value < low || value > high
    }

    #[must_use]
    pub fn severity(&self, value: f64) -> f64 {
        (value - self.center).abs() / self.threshold().max(MINIMUM_SPREAD)
    }
}

type Series = (i64, i64, Metric);

/// # Errors
///
/// Returns the diesel error if any query fails.
pub fn run(conn: &mut PgConnection, now: NaiveDateTime) -> QueryResult<Vec<i64>> {
    let mut raised = Vec::new();

    for (node_id, channel_id, metric) in recent_series(conn, now)? {
        let Some(alarm) = examine(conn, node_id, channel_id, metric, now)? else {
            continue;
        };

        raised.push(alarm);
    }

    Ok(raised)
}

fn recent_series(conn: &mut PgConnection, now: NaiveDateTime) -> QueryResult<Vec<Series>> {
    let Some(since) = now.checked_sub_signed(TimeDelta::hours(1)) else {
        return Ok(Vec::new());
    };

    measurements_schema::table
        .filter(measurements_schema::window_start.ge(since))
        .select((
            measurements_schema::node_id,
            measurements_schema::channel_id,
            measurements_schema::metric,
        ))
        .distinct()
        .load(conn)
}

///
/// # Errors
///
/// Returns the diesel error if any query fails.
pub fn examine(
    conn: &mut PgConnection,
    node_id: i64,
    channel_id: i64,
    metric: Metric,
    now: NaiveDateTime,
) -> QueryResult<Option<i64>> {
    let recent = recent_windows(conn, node_id, channel_id, metric, CONSECUTIVE_WINDOWS)?;
    if recent.len() < CONSECUTIVE_WINDOWS {
        return Ok(None);
    }

    let hour = now.hour();
    let Some(baseline) = baseline(conn, node_id, channel_id, metric, hour, now)? else {
        return Ok(None);
    };

    let (low, high) = baseline.band();
    let all_below = recent.iter().all(|(_, value)| *value < low);
    let all_above = recent.iter().all(|(_, value)| *value > high);
    if !all_below && !all_above {
        return Ok(None);
    }

    if has_open_alarm(conn, node_id, channel_id, metric)? {
        return Ok(None);
    }

    let Some((window_start, value)) = recent.first().copied() else {
        return Ok(None);
    };

    let explanation = json!({
        "kind": "baseline_deviation",
        "metric": metric.label(),
        "hour_bucket": hour,
        "window_start": window_start,
        "value": value,
        "direction": if all_below { "below" } else { "above" },
        "baseline": {
            "center": baseline.center,
            "between_hours": baseline.between,
            "within_hour": baseline.within,
            "buckets": baseline.buckets,
            "days": BASELINE_DAYS,
        },
        "threshold": {
            "k": K,
            "half_width": baseline.threshold(),
            "low": low,
            "high": high,
        },
        "severity": baseline.severity(value),
        "consecutive_windows": CONSECUTIVE_WINDOWS,
    });

    raise(conn, node_id, Some(channel_id), Some(metric), explanation).map(Some)
}

fn recent_windows(
    conn: &mut PgConnection,
    node_id: i64,
    channel_id: i64,
    metric: Metric,
    count: usize,
) -> QueryResult<Vec<(NaiveDateTime, f64)>> {
    measurements_schema::table
        .filter(measurements_schema::node_id.eq(node_id))
        .filter(measurements_schema::channel_id.eq(channel_id))
        .filter(measurements_schema::metric.eq(metric))
        .select((measurements_schema::window_start, measurements_schema::mean))
        .order(measurements_schema::window_start.desc())
        .limit(i64::try_from(count).unwrap_or(i64::MAX))
        .load(conn)
}

///
/// # Errors
///
/// Returns the diesel error if the query fails.
pub fn baseline(
    conn: &mut PgConnection,
    node_id: i64,
    channel_id: i64,
    metric: Metric,
    hour: u32,
    now: NaiveDateTime,
) -> QueryResult<Option<Baseline>> {
    let Some(since) = now.checked_sub_days(Days::new(BASELINE_DAYS)) else {
        return Ok(None);
    };

    let buckets: Vec<(NaiveDateTime, f64, f64)> = rollups_schema::table
        .filter(rollups_schema::node_id.eq(node_id))
        .filter(rollups_schema::channel_id.eq(channel_id))
        .filter(rollups_schema::metric.eq(metric))
        .filter(rollups_schema::resolution.eq(RollupResolution::Hourly))
        .filter(rollups_schema::bucket_start.ge(since))
        .filter(rollups_schema::bucket_start.lt(now))
        .select((
            rollups_schema::bucket_start,
            rollups_schema::mean,
            rollups_schema::stddev,
        ))
        .load(conn)?;

    let mut means: Vec<f64> = Vec::new();
    let mut spreads: Vec<f64> = Vec::new();
    for (bucket_start, mean, stddev) in buckets {
        if bucket_start.hour() != hour {
            continue;
        }

        means.push(mean);
        spreads.push(stddev);
    }

    if means.len() < MIN_BASELINE_BUCKETS {
        return Ok(None);
    }

    let buckets = means.len();
    let Some(center) = median(&mut means) else {
        return Ok(None);
    };

    let mut deviations: Vec<f64> = means.iter().map(|mean| (mean - center).abs()).collect();
    let between = median(&mut deviations).unwrap_or(0.0) * MAD_TO_SIGMA;
    let within = median(&mut spreads).unwrap_or(0.0);

    Ok(Some(Baseline {
        center,
        between,
        within,
        buckets,
    }))
}

/// # Errors
///
/// Returns the diesel error if the query fails.
pub fn has_open_alarm(
    conn: &mut PgConnection,
    node_id: i64,
    channel_id: i64,
    metric: Metric,
) -> QueryResult<bool> {
    let count: i64 = alarms_schema::table
        .filter(alarms_schema::node_id.eq(node_id))
        .filter(alarms_schema::channel_id.eq(channel_id))
        .filter(alarms_schema::metric.eq(metric))
        .filter(alarms_schema::state.ne(AlarmState::Closed))
        .count()
        .get_result(conn)?;

    Ok(count > 0)
}

///
/// # Errors
///
/// Returns the diesel error if either write fails.
pub fn raise(
    conn: &mut PgConnection,
    node_id: i64,
    channel_id: Option<i64>,
    metric: Option<Metric>,
    explanation: serde_json::Value,
) -> QueryResult<i64> {
    conn.transaction(|conn| {
        let alarm_id: i64 = diesel::insert_into(alarms_schema::table)
            .values(&NewAlarm {
                node_id,
                channel_id,
                metric,
                state: AlarmState::Open,
                explanation,
            })
            .returning(alarms_schema::id)
            .get_result(conn)?;

        diesel::insert_into(alarm_events_schema::table)
            .values(&NewAlarmEvent {
                alarm_id,
                from_state: None,
                to_state: AlarmState::Open,
                changed_by_user_id: None,
                reason: "raised by the detector".to_owned(),
            })
            .execute(conn)?;

        Ok(alarm_id)
    })
}

///
pub const SILENCE_THRESHOLD_MINUTES: i64 = 10;

/// One node that has stopped delivering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Silence {
    pub node_id: i64,
    pub alarm_id: i64,
    pub since: NaiveDateTime,
}

/// Raise an alarm for every node that has stopped delivering.
///
///
/// # Errors
///
/// Returns the diesel error if any query fails.
pub fn silence(conn: &mut PgConnection, now: NaiveDateTime) -> QueryResult<Vec<Silence>> {
    use crate::db::schema::nodes as nodes_schema;

    let Some(cutoff) = now.checked_sub_signed(TimeDelta::minutes(SILENCE_THRESHOLD_MINUTES)) else {
        return Ok(Vec::new());
    };

    let quiet: Vec<(i64, NaiveDateTime)> = nodes_schema::table
        .filter(nodes_schema::suspended.eq(false))
        .filter(nodes_schema::last_seen_at.is_not_null())
        .filter(nodes_schema::last_seen_at.lt(cutoff))
        .select((
            nodes_schema::id,
            nodes_schema::last_seen_at.assume_not_null(),
        ))
        .load(conn)?;

    let mut raised = Vec::new();
    for (node_id, since) in quiet {
        if has_open_silence_alarm(conn, node_id)? {
            continue;
        }

        let explanation = json!({
            "kind": "node_silence",
            "last_seen_at": since,
            "detected_at": now,
            "threshold_minutes": SILENCE_THRESHOLD_MINUTES,
        });

        raised.push(Silence {
            node_id,
            alarm_id: raise(conn, node_id, None, None, explanation)?,
            since,
        });
    }

    Ok(raised)
}

fn has_open_silence_alarm(conn: &mut PgConnection, node_id: i64) -> QueryResult<bool> {
    let count: i64 = alarms_schema::table
        .filter(alarms_schema::node_id.eq(node_id))
        .filter(alarms_schema::channel_id.is_null())
        .filter(alarms_schema::metric.is_null())
        .filter(alarms_schema::state.ne(AlarmState::Closed))
        .count()
        .get_result(conn)?;

    Ok(count > 0)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn baseline_of(center: f64, between: f64, within: f64) -> Baseline {
        Baseline {
            center,
            between,
            within,
            buckets: 28,
        }
    }

    #[test]
    fn the_band_widens_with_both_kinds_of_spread() {
        let narrow = baseline_of(30.0, 0.5, 0.0).threshold();
        let wider = baseline_of(30.0, 0.5, 0.5).threshold();

        assert!(wider > narrow, "{wider} is not wider than {narrow}");
    }

    #[test]
    fn independent_spreads_add_in_quadrature() {
        let baseline = baseline_of(0.0, 3.0, 4.0);

        assert!(K.mul_add(-5.0, baseline.threshold()).abs() < 1e-9);
    }

    #[test]
    fn a_reading_at_the_center_is_not_a_deviation() {
        let baseline = baseline_of(30.0, 1.0, 1.0);

        assert!(!baseline.is_deviation(30.0));
        assert!((baseline.severity(30.0)).abs() < 1e-12);
    }

    #[test]
    fn a_reading_outside_the_band_is_a_deviation() {
        let baseline = baseline_of(30.0, 1.0, 0.0);
        let (low, high) = baseline.band();

        assert!(baseline.is_deviation(high + 0.1));
        assert!(baseline.is_deviation(low - 0.1));
        assert!(!baseline.is_deviation(high - 0.1));
        assert!(!baseline.is_deviation(low + 0.1));
    }

    #[test]
    fn severity_is_one_at_the_edge_of_the_band() {
        let baseline = baseline_of(30.0, 1.0, 0.0);
        let (_, high) = baseline.band();

        assert!((baseline.severity(high) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_pinned_series_still_has_a_usable_band() {
        let baseline = baseline_of(30.0, 0.0, 0.0);

        assert!(baseline.threshold() > 0.0);
        assert!(!baseline.is_deviation(30.0));
        assert!(baseline.severity(30.0).is_finite());
    }

    #[test]
    fn the_mad_scaling_matches_a_normal_distribution() {
        assert!(MAD_TO_SIGMA.mul_add(0.674_489_75, -1.0).abs() < 1e-4);
    }

    #[test]
    fn the_median_of_an_even_set_averages_the_middle_pair() {
        assert!(median(&mut [1.0, 2.0, 3.0, 4.0]).is_some_and(|v| (v - 2.5).abs() < 1e-9));
        assert!(median(&mut [3.0, 1.0, 2.0]).is_some_and(|v| (v - 2.0).abs() < 1e-9));
        assert_eq!(median(&mut []), None);
    }
}
