use std::collections::{HashMap, HashSet};

use chrono::{
    DateTime, Datelike as _, Days, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Timelike as _,
    Utc,
};
use diesel::pg::PgConnection;
use diesel::prelude::*;

use crate::db::models::enums::{Metric, RollupResolution};
use crate::db::models::rollups::NewRollup;
use crate::db::schema::measurements as measurements_schema;
use crate::db::schema::rollups as rollups_schema;
use crate::jobs::median;

const MAX_BUCKETS_PER_RUN: usize = 512;

type Summary = (i64, i64, Metric, f64, f64, f64, f64, f64, i64);

/// The bucket `at` falls in, for one resolution.
#[must_use]
pub fn bucket_start(resolution: RollupResolution, at: NaiveDateTime) -> Option<NaiveDateTime> {
    let date = at.date();

    match resolution {
        RollupResolution::Hourly => date.and_hms_opt(at.time().hour(), 0, 0),
        RollupResolution::Daily => Some(date.and_time(NaiveTime::MIN)),
        RollupResolution::Weekly => {
            let weekday = u64::from(date.weekday().num_days_from_monday());

            date.checked_sub_days(Days::new(weekday))
                .map(|monday| monday.and_time(NaiveTime::MIN))
        }
    }
}

/// The end of the bucket starting at `start`.
#[must_use]
pub const fn bucket_end(
    resolution: RollupResolution,
    start: NaiveDateTime,
) -> Option<NaiveDateTime> {
    match resolution {
        RollupResolution::Hourly => start.checked_add_signed(TimeDelta::hours(1)),
        RollupResolution::Daily => start.checked_add_days(Days::new(1)),
        RollupResolution::Weekly => start.checked_add_days(Days::new(7)),
    }
}

/// The next bucket after the one starting at `start`.
#[must_use]
pub const fn next_bucket(
    resolution: RollupResolution,
    start: NaiveDateTime,
) -> Option<NaiveDateTime> {
    bucket_end(resolution, start)
}

/// Summarize completed buckets of one resolution.
///
/// # Errors
///
/// Returns the diesel error if any query fails.
pub fn run(
    conn: &mut PgConnection,
    resolution: RollupResolution,
    now: NaiveDateTime,
) -> QueryResult<usize> {
    let Some(mut start) = first_outstanding(conn, resolution)? else {
        return Ok(0);
    };

    let stored = stored_buckets(conn, resolution, start)?;

    let mut written: usize = 0;
    let mut built: usize = 0;
    while built < MAX_BUCKETS_PER_RUN {
        let Some(end) = bucket_end(resolution, start) else {
            break;
        };

        if end > now {
            break;
        }

        if !stored.contains(&start) {
            written = written.saturating_add(build(conn, resolution, start, end)?);
            built = built.saturating_add(1);
        }

        let Some(next) = next_bucket(resolution, start) else {
            break;
        };

        start = next;
    }

    Ok(written)
}

fn stored_buckets(
    conn: &mut PgConnection,
    resolution: RollupResolution,
    from: NaiveDateTime,
) -> QueryResult<HashSet<NaiveDateTime>> {
    Ok(rollups_schema::table
        .filter(rollups_schema::resolution.eq(resolution))
        .filter(rollups_schema::bucket_start.ge(from))
        .select(rollups_schema::bucket_start)
        .distinct()
        .load::<NaiveDateTime>(conn)?
        .into_iter()
        .collect())
}

/// Build all three resolutions.
///
/// # Errors
///
/// Returns the diesel error from the first resolution that fails.
pub fn run_all(conn: &mut PgConnection, now: NaiveDateTime) -> QueryResult<usize> {
    let mut written: usize = 0;

    for resolution in [
        RollupResolution::Hourly,
        RollupResolution::Daily,
        RollupResolution::Weekly,
    ] {
        written = written.saturating_add(run(conn, resolution, now)?);
    }

    Ok(written)
}

fn first_outstanding(
    conn: &mut PgConnection,
    resolution: RollupResolution,
) -> QueryResult<Option<NaiveDateTime>> {
    let oldest: Option<NaiveDateTime> = measurements_schema::table
        .select(diesel::dsl::min(measurements_schema::window_start))
        .first(conn)?;

    Ok(oldest.and_then(|at| bucket_start(resolution, at)))
}

fn build(
    conn: &mut PgConnection,
    resolution: RollupResolution,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> QueryResult<usize> {
    let summaries: Vec<Summary> = measurements_schema::table
        .filter(measurements_schema::window_start.ge(start))
        .filter(measurements_schema::window_start.lt(end))
        .select((
            measurements_schema::node_id,
            measurements_schema::channel_id,
            measurements_schema::metric,
            measurements_schema::min,
            measurements_schema::max,
            measurements_schema::mean,
            measurements_schema::median,
            measurements_schema::stddev,
            measurements_schema::sample_count,
        ))
        .order((
            measurements_schema::node_id,
            measurements_schema::channel_id,
            measurements_schema::metric,
        ))
        .load(conn)?;

    if summaries.is_empty() {
        return Ok(0);
    }

    let rows = fold(summaries, resolution, start, end);
    if rows.is_empty() {
        return Ok(0);
    }

    diesel::insert_into(rollups_schema::table)
        .values(&rows)
        .on_conflict_do_nothing()
        .execute(conn)
}

fn fold(
    summaries: Vec<Summary>,
    resolution: RollupResolution,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Vec<NewRollup> {
    let mut groups: HashMap<(i64, i64, Metric), Group> = HashMap::new();

    for (node_id, channel_id, metric, min, max, mean, median, stddev, sample_count) in summaries {
        groups
            .entry((node_id, channel_id, metric))
            .or_insert_with(|| Group::new(node_id, channel_id, metric))
            .push(min, max, mean, median, stddev, sample_count);
    }

    groups
        .into_values()
        .filter_map(|group| group.finish(resolution, start, end))
        .collect()
}

struct Group {
    node_id: i64,
    channel_id: i64,
    metric: Metric,
    min: f64,
    max: f64,
    weighted_sum: f64,
    within: f64,
    means: Vec<(f64, i64)>,
    medians: Vec<f64>,
    samples: i64,
}

impl Group {
    const fn new(node_id: i64, channel_id: i64, metric: Metric) -> Self {
        Self {
            node_id,
            channel_id,
            metric,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            weighted_sum: 0.0,
            within: 0.0,
            means: Vec::new(),
            medians: Vec::new(),
            samples: 0,
        }
    }

    fn push(&mut self, min: f64, max: f64, mean: f64, median: f64, stddev: f64, count: i64) {
        if count <= 0 {
            return;
        }
        let weight = count_as_f64(count);

        self.min = self.min.min(min);
        self.max = self.max.max(max);
        self.weighted_sum = mean.mul_add(weight, self.weighted_sum);
        self.within = (weight - 1.0)
            .max(0.0)
            .mul_add(stddev * stddev, self.within);
        self.means.push((mean, count));
        self.medians.push(median);
        self.samples = self.samples.saturating_add(count);
    }

    fn finish(
        mut self,
        resolution: RollupResolution,
        bucket_start: NaiveDateTime,
        bucket_end: NaiveDateTime,
    ) -> Option<NewRollup> {
        if self.samples <= 0 || self.medians.is_empty() {
            return None;
        }

        let total = count_as_f64(self.samples);
        let mean = self.weighted_sum / total;

        let between: f64 = self
            .means
            .iter()
            .map(|(window_mean, count)| {
                let difference = window_mean - mean;

                count_as_f64(*count) * difference * difference
            })
            .sum();
        let denominator = (total - 1.0).max(1.0);
        let variance = (self.within + between) / denominator;

        Some(NewRollup {
            node_id: self.node_id,
            channel_id: self.channel_id,
            metric: self.metric,
            resolution,
            bucket_start,
            bucket_end,
            min: self.min,
            max: self.max,
            mean: mean.clamp(self.min, self.max),
            median: median(&mut self.medians)?.clamp(self.min, self.max),
            stddev: variance.max(0.0).sqrt(),
            sample_count: self.samples,
        })
    }
}

fn count_as_f64(count: i64) -> f64 {
    u32::try_from(count.max(0)).map_or(f64::MAX, f64::from)
}

/// Now, by the server's clock.
#[must_use]
pub fn now() -> NaiveDateTime {
    Utc::now().naive_utc()
}

/// Midnight `days` ago, for a retention horizon.
#[must_use]
pub fn horizon(days: u64) -> Option<NaiveDateTime> {
    let today: NaiveDate = DateTime::<Utc>::from(std::time::SystemTime::now()).date_naive();

    today
        .checked_sub_days(Days::new(days))
        .map(|date| date.and_time(NaiveTime::MIN))
}

/// Delete raw windows older than `before`.
///
/// # Errors
///
/// Returns the diesel error if the delete fails.
pub fn prune_raw(conn: &mut PgConnection, before: NaiveDateTime) -> QueryResult<usize> {
    diesel::delete(measurements_schema::table.filter(measurements_schema::window_start.lt(before)))
        .execute(conn)
}

/// Delete rollups of `resolution` older than `before`.
///
/// # Errors
///
/// Returns the diesel error if the delete fails.
pub fn prune_rollups(
    conn: &mut PgConnection,
    resolution: RollupResolution,
    before: NaiveDateTime,
) -> QueryResult<usize> {
    diesel::delete(
        rollups_schema::table
            .filter(rollups_schema::resolution.eq(resolution))
            .filter(rollups_schema::bucket_start.lt(before)),
    )
    .execute(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hour: u32, minute: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 9, 16)
            .expect("a real date")
            .and_hms_opt(hour, minute, 0)
            .expect("a real time")
    }

    #[test]
    fn an_hourly_bucket_starts_on_the_hour() {
        assert_eq!(
            bucket_start(RollupResolution::Hourly, at(13, 47)),
            Some(at(13, 0))
        );
    }

    #[test]
    fn a_daily_bucket_starts_at_midnight() {
        assert_eq!(
            bucket_start(RollupResolution::Daily, at(13, 47)),
            Some(at(0, 0))
        );
    }

    #[test]
    fn a_weekly_bucket_starts_on_monday() {
        let monday = NaiveDate::from_ymd_opt(2026, 9, 14)
            .expect("a real date")
            .and_time(NaiveTime::MIN);

        assert_eq!(
            bucket_start(RollupResolution::Weekly, at(13, 47)),
            Some(monday)
        );
    }

    #[test]
    fn a_bucket_ends_one_period_after_it_starts() {
        assert_eq!(
            bucket_end(RollupResolution::Hourly, at(13, 0)),
            Some(at(14, 0))
        );
        assert_eq!(
            bucket_end(RollupResolution::Daily, at(0, 0)),
            NaiveDate::from_ymd_opt(2026, 9, 17).map(|date| date.and_time(NaiveTime::MIN))
        );
        assert_eq!(
            bucket_end(RollupResolution::Weekly, at(0, 0)),
            NaiveDate::from_ymd_opt(2026, 9, 23).map(|date| date.and_time(NaiveTime::MIN))
        );
    }

    #[test]
    fn folding_one_window_reproduces_it() {
        let rows = fold(
            vec![(1, 2, Metric::SignalToNoise, 20.0, 30.0, 25.0, 24.0, 2.0, 60)],
            RollupResolution::Hourly,
            at(13, 0),
            at(14, 0),
        );

        let row = rows.first().expect("one group");
        assert!((row.min - 20.0).abs() < 1e-9);
        assert!((row.max - 30.0).abs() < 1e-9);
        assert!((row.mean - 25.0).abs() < 1e-9);
        assert!((row.median - 24.0).abs() < 1e-9);
        assert!((row.stddev - 2.0).abs() < 1e-9);
        assert_eq!(row.sample_count, 60);
    }

    #[test]
    fn the_mean_is_weighted_by_sample_count() {
        let rows = fold(
            vec![
                (1, 2, Metric::SignalToNoise, 0.0, 10.0, 10.0, 10.0, 0.0, 90),
                (1, 2, Metric::SignalToNoise, 0.0, 10.0, 0.0, 0.0, 0.0, 10),
            ],
            RollupResolution::Hourly,
            at(13, 0),
            at(14, 0),
        );

        let row = rows.first().expect("one group");
        assert!((row.mean - 9.0).abs() < 1e-9, "the mean is {}", row.mean);
        assert_eq!(row.sample_count, 100);
    }

    #[test]
    fn the_pooled_variance_matches_the_raw_one() {
        let first = [1.0_f64, 2.0, 3.0];
        let second = [7.0_f64, 8.0, 9.0, 10.0];

        let summarize = |values: &[f64]| -> (f64, f64, i64) {
            let count = values.len();
            let widened = f64::from(u32::try_from(count).unwrap_or(0));
            let mean = values.iter().sum::<f64>() / widened;
            let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (widened - 1.0);

            (mean, variance.sqrt(), i64::try_from(count).unwrap_or(0))
        };

        let (mean_a, stddev_a, count_a) = summarize(&first);
        let (mean_b, stddev_b, count_b) = summarize(&second);

        let rows = fold(
            vec![
                (
                    1,
                    2,
                    Metric::SignalToNoise,
                    1.0,
                    3.0,
                    mean_a,
                    mean_a,
                    stddev_a,
                    count_a,
                ),
                (
                    1,
                    2,
                    Metric::SignalToNoise,
                    7.0,
                    10.0,
                    mean_b,
                    mean_b,
                    stddev_b,
                    count_b,
                ),
            ],
            RollupResolution::Hourly,
            at(13, 0),
            at(14, 0),
        );

        let combined: Vec<f64> = first.iter().chain(&second).copied().collect();
        let widened = f64::from(u32::try_from(combined.len()).unwrap_or(0));
        let mean = combined.iter().sum::<f64>() / widened;
        let expected =
            (combined.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (widened - 1.0)).sqrt();

        let row = rows.first().expect("one group");
        assert!(
            (row.stddev - expected).abs() < 1e-9,
            "pooled {} against raw {expected}",
            row.stddev
        );
    }

    #[test]
    fn separate_metrics_do_not_merge() {
        let rows = fold(
            vec![
                (1, 2, Metric::SignalToNoise, 20.0, 30.0, 25.0, 25.0, 1.0, 60),
                (
                    1,
                    2,
                    Metric::SignalStrength,
                    -50.0,
                    -40.0,
                    -45.0,
                    -45.0,
                    1.0,
                    60,
                ),
                (2, 2, Metric::SignalToNoise, 10.0, 20.0, 15.0, 15.0, 1.0, 60),
            ],
            RollupResolution::Hourly,
            at(13, 0),
            at(14, 0),
        );

        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn a_window_with_no_samples_is_ignored() {
        let rows = fold(
            vec![(1, 2, Metric::SignalToNoise, 20.0, 30.0, 25.0, 25.0, 1.0, 0)],
            RollupResolution::Hourly,
            at(13, 0),
            at(14, 0),
        );

        assert!(rows.is_empty());
    }

    #[test]
    fn the_median_of_an_even_set_averages_the_middle_pair() {
        assert!(median(&mut [1.0, 2.0, 3.0, 4.0]).is_some_and(|v| (v - 2.5).abs() < 1e-9));
        assert!(median(&mut [3.0, 1.0, 2.0]).is_some_and(|v| (v - 2.0).abs() < 1e-9));
        assert_eq!(median(&mut []), None);
    }
}
