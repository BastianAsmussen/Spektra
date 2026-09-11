use std::collections::{HashMap, HashSet};

use chrono::{Days, NaiveDateTime, TimeDelta, Timelike as _};
use diesel::QueryableByName;
use diesel::pg::PgConnection;
use diesel::prelude::*;
use serde_json::json;

use crate::db::models::alarm_events::NewAlarmEvent;
use crate::db::models::alarms::NewAlarm;
use crate::db::models::enums::{AlarmState, Metric, RollupResolution};
use crate::db::schema::alarm_events as alarm_events_schema;
use crate::db::schema::alarms as alarms_schema;
use crate::db::schema::rollups as rollups_schema;
use crate::jobs::median;

pub const BASELINE_DAYS: u64 = 28;

/// Below a week of the same hour, the median is a guess, not a baseline.
pub const MIN_BASELINE_BUCKETS: usize = 7;

/// Band half-width, in robust standard deviations.
pub const K: f64 = 4.0;

pub const CONSECUTIVE_WINDOWS: usize = 3;

/// Median absolute deviation to standard-deviation equivalent.
const MAD_TO_SIGMA: f64 = 1.4826;

/// Floor for the spread when MAD is zero.
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

    /// Deviation in band half-widths; one sits exactly on the edge.
    #[must_use]
    pub fn severity(&self, value: f64) -> f64 {
        (value - self.center).abs() / self.threshold().max(MINIMUM_SPREAD)
    }
}

type Series = (i64, i64, Metric);

/// Run one detection pass over every series.
///
/// # Errors
///
/// Returns the diesel error if any query fails.
pub fn run(conn: &mut PgConnection, now: NaiveDateTime) -> QueryResult<Vec<i64>> {
    let hour = now.hour();
    let windows = recent_windows(conn, now, None)?;
    if windows.is_empty() {
        return Ok(Vec::new());
    }

    let baselines = baselines(conn, now, hour, None)?;
    let open = open_alarms(conn)?;

    let mut series: Vec<Series> = windows.keys().copied().collect();
    series.sort_unstable();

    let mut raised = Vec::new();
    for key in series {
        let (Some(recent), Some(baseline)) = (windows.get(&key), baselines.get(&key)) else {
            continue;
        };

        let Some(verdict) = judge(recent, baseline) else {
            continue;
        };

        if open.contains(&key) {
            continue;
        }

        raised.push(raise(
            conn,
            key.0,
            Some(key.1),
            Some(key.2),
            deviation(key.2, hour, baseline, &verdict),
        )?);
    }

    Ok(raised)
}

struct Verdict {
    window_start: NaiveDateTime,
    value: f64,
    below: bool,
}

fn judge(recent: &[(NaiveDateTime, f64)], baseline: &Baseline) -> Option<Verdict> {
    if recent.len() < CONSECUTIVE_WINDOWS {
        return None;
    }

    let (low, high) = baseline.band();
    let below = recent.iter().all(|(_, value)| *value < low);
    let above = recent.iter().all(|(_, value)| *value > high);
    if !below && !above {
        return None;
    }

    let (window_start, value) = recent.first().copied()?;

    Some(Verdict {
        window_start,
        value,
        below,
    })
}

fn deviation(
    metric: Metric,
    hour: u32,
    baseline: &Baseline,
    verdict: &Verdict,
) -> serde_json::Value {
    let (low, high) = baseline.band();

    json!({
        "kind": "baseline_deviation",
        "metric": metric.label(),
        "hour_bucket": hour,
        "window_start": verdict.window_start,
        "value": verdict.value,
        "direction": if verdict.below { "below" } else { "above" },
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
        "severity": baseline.severity(verdict.value),
        "consecutive_windows": CONSECUTIVE_WINDOWS,
    })
}

/// New alarm id, or `None` when the series is healthy, too new, or already open.
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
    let key = (node_id, channel_id, metric);
    let hour = now.hour();

    let windows = recent_windows(conn, now, Some(key))?;
    let Some(recent) = windows.get(&key) else {
        return Ok(None);
    };

    let Some(baseline) = baseline(conn, node_id, channel_id, metric, hour, now)? else {
        return Ok(None);
    };

    let Some(verdict) = judge(recent, &baseline) else {
        return Ok(None);
    };

    if has_open_alarm(conn, node_id, channel_id, metric)? {
        return Ok(None);
    }

    raise(
        conn,
        node_id,
        Some(channel_id),
        Some(metric),
        deviation(metric, hour, &baseline, &verdict),
    )
    .map(Some)
}

#[derive(Debug, QueryableByName)]
struct WindowRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    node_id: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    channel_id: i64,
    #[diesel(sql_type = crate::db::schema::sql_types::Metric)]
    metric: Metric,
    #[diesel(sql_type = diesel::sql_types::Timestamp)]
    window_start: NaiveDateTime,
    #[diesel(sql_type = diesel::sql_types::Double)]
    mean: f64,
}

fn recent_windows(
    conn: &mut PgConnection,
    now: NaiveDateTime,
    only: Option<Series>,
) -> QueryResult<HashMap<Series, Vec<(NaiveDateTime, f64)>>> {
    let Some(since) = now.checked_sub_signed(TimeDelta::hours(1)) else {
        return Ok(HashMap::new());
    };

    let sql = "
        SELECT node_id, channel_id, metric, window_start, mean
        FROM (
            SELECT node_id, channel_id, metric, window_start, mean,
                   row_number() OVER (
                       PARTITION BY node_id, channel_id, metric
                       ORDER BY window_start DESC
                   ) AS rank
            FROM measurements
            WHERE window_start >= $1
              AND ($2 IS NULL OR node_id = $2)
              AND ($3 IS NULL OR channel_id = $3)
        ) ranked
        WHERE rank <= $4
        ORDER BY node_id, channel_id, metric, window_start DESC";

    let rows: Vec<WindowRow> = diesel::sql_query(sql)
        .bind::<diesel::sql_types::Timestamp, _>(since)
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::BigInt>, _>(
            only.map(|(node_id, _, _)| node_id),
        )
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::BigInt>, _>(
            only.map(|(_, channel_id, _)| channel_id),
        )
        .bind::<diesel::sql_types::BigInt, _>(
            i64::try_from(CONSECUTIVE_WINDOWS).unwrap_or(i64::MAX),
        )
        .load(conn)?;

    let mut windows: HashMap<Series, Vec<(NaiveDateTime, f64)>> = HashMap::new();
    for row in rows {
        let key = (row.node_id, row.channel_id, row.metric);
        if only.is_some_and(|wanted| wanted != key) {
            continue;
        }

        windows
            .entry(key)
            .or_default()
            .push((row.window_start, row.mean));
    }

    Ok(windows)
}

/// Baseline for one series at one hour of day, or `None` if history is too short.
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
    let key = (node_id, channel_id, metric);

    Ok(baselines(conn, now, hour, Some(key))?.remove(&key))
}

fn baseline_buckets(now: NaiveDateTime, hour: u32) -> Vec<NaiveDateTime> {
    let Some(since) = now.checked_sub_days(Days::new(BASELINE_DAYS)) else {
        return Vec::new();
    };

    (0..=BASELINE_DAYS)
        .filter_map(|back| {
            now.checked_sub_days(Days::new(back))?
                .date()
                .and_hms_opt(hour, 0, 0)
        })
        .filter(|bucket| *bucket >= since && *bucket < now)
        .collect()
}

fn baselines(
    conn: &mut PgConnection,
    now: NaiveDateTime,
    hour: u32,
    only: Option<Series>,
) -> QueryResult<HashMap<Series, Baseline>> {
    let buckets = baseline_buckets(now, hour);
    if buckets.is_empty() {
        return Ok(HashMap::new());
    }

    let mut query = rollups_schema::table
        .filter(rollups_schema::resolution.eq(RollupResolution::Hourly))
        .filter(rollups_schema::bucket_start.eq_any(buckets))
        .select((
            rollups_schema::node_id,
            rollups_schema::channel_id,
            rollups_schema::metric,
            rollups_schema::mean,
            rollups_schema::stddev,
        ))
        .into_boxed();

    if let Some((node_id, channel_id, metric)) = only {
        query = query
            .filter(rollups_schema::node_id.eq(node_id))
            .filter(rollups_schema::channel_id.eq(channel_id))
            .filter(rollups_schema::metric.eq(metric));
    }

    let rows: Vec<(i64, i64, Metric, f64, f64)> = query.load(conn)?;

    let mut gathered: HashMap<Series, (Vec<f64>, Vec<f64>)> = HashMap::new();
    for (node_id, channel_id, metric, mean, stddev) in rows {
        let entry = gathered
            .entry((node_id, channel_id, metric))
            .or_insert_with(|| (Vec::new(), Vec::new()));
        entry.0.push(mean);
        entry.1.push(stddev);
    }

    Ok(gathered
        .into_iter()
        .filter_map(|(key, (mut means, mut spreads))| {
            Some((key, fold_baseline(&mut means, &mut spreads)?))
        })
        .collect())
}

fn fold_baseline(means: &mut [f64], spreads: &mut [f64]) -> Option<Baseline> {
    if means.len() < MIN_BASELINE_BUCKETS {
        return None;
    }

    let buckets = means.len();
    let center = median(means)?;

    let mut deviations: Vec<f64> = means.iter().map(|mean| (mean - center).abs()).collect();
    let between = median(&mut deviations).unwrap_or(0.0) * MAD_TO_SIGMA;
    let within = median(spreads).unwrap_or(0.0);

    Some(Baseline {
        center,
        between,
        within,
        buckets,
    })
}

fn open_alarms(conn: &mut PgConnection) -> QueryResult<HashSet<Series>> {
    let rows: Vec<(i64, Option<i64>, Option<Metric>)> = alarms_schema::table
        .filter(alarms_schema::state.ne(AlarmState::Closed))
        .select((
            alarms_schema::node_id,
            alarms_schema::channel_id,
            alarms_schema::metric,
        ))
        .load(conn)?;

    Ok(rows
        .into_iter()
        .filter_map(|(node_id, channel_id, metric)| Some((node_id, channel_id?, metric?)))
        .collect())
}

fn silent_nodes(conn: &mut PgConnection) -> QueryResult<HashSet<i64>> {
    alarms_schema::table
        .filter(alarms_schema::state.ne(AlarmState::Closed))
        .filter(alarms_schema::channel_id.is_null())
        .filter(alarms_schema::metric.is_null())
        .select(alarms_schema::node_id)
        .load(conn)
        .map(|rows: Vec<i64>| rows.into_iter().collect())
}

/// Whether this series already has an open alarm.
///
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

/// Insert an alarm and its opening event in one transaction.
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

/// Minutes a node may go quiet before it counts as silent.
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

    let already = silent_nodes(conn)?;

    let mut raised = Vec::new();
    for (node_id, since) in quiet {
        if already.contains(&node_id) {
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
}
