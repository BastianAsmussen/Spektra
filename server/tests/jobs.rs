#![expect(
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    reason = "test harness helpers are not `#[test]` functions, so clippy.toml's in-tests allowances do not reach them"
)]

mod common;

use common::{Pool, test_pool};

use chrono::{Days, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Timelike as _};
use diesel::prelude::*;
use serde_json::json;
use server::db::models::channels::NewChannel;
use server::db::models::enums::{AlarmState, Metric, Modulation, RollupResolution};
use server::db::models::measurements::NewMeasurement;
use server::db::models::nodes::NewNode;
use server::db::models::rollups::NewRollup;
use server::db::schema::{
    alarm_events as alarm_events_schema, alarms as alarms_schema, channels as channels_schema,
    measurements as measurements_schema, nodes as nodes_schema, rollups as rollups_schema,
};
use server::jobs::{detector, partitions, rollup};

async fn seed_node_and_channel(pool: &Pool) -> (i64, i64) {
    let conn = pool.get().await.expect("seed connection");

    conn.interact(|conn| {
        let node_id: i64 = diesel::insert_into(nodes_schema::table)
            .values(&NewNode {
                external_identity: Some("jobs-test-node".to_owned()),
                name: "Jobs test node".to_owned(),
                latitude: Some(57.05),
                longitude: Some(9.92),
                hardware: json!({"device": "test"}),
                capabilities: json!({"metrics": []}),
            })
            .returning(nodes_schema::id)
            .get_result(conn)
            .expect("node insert");

        let channel_id: i64 = diesel::insert_into(channels_schema::table)
            .values(&NewChannel {
                name: "Test channel".to_owned(),
                frequency_hz: 89_700_000,
                modulation: Modulation::Fm,
            })
            .returning(channels_schema::id)
            .get_result(conn)
            .expect("channel insert");

        (node_id, channel_id)
    })
    .await
    .expect("seed interact failed")
}

async fn seed_measurement(
    pool: &Pool,
    node_id: i64,
    channel_id: i64,
    window_start: NaiveDateTime,
    mean: f64,
    sample_count: i64,
) {
    let conn = pool.get().await.expect("seed connection");
    let row = NewMeasurement {
        node_id,
        channel_id,
        metric: Metric::SignalToNoise,
        window_start,
        window_end: window_start + TimeDelta::minutes(1),
        min: mean - 1.0,
        max: mean + 1.0,
        mean,
        median: mean,
        stddev: 0.5,
        p95: mean + 0.9,
        sample_count,
    };

    conn.interact(move |conn| {
        diesel::insert_into(measurements_schema::table)
            .values(&row)
            .execute(conn)
    })
    .await
    .expect("seed interact failed")
    .expect("measurement insert");
}

const fn day(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).expect("a real date")
}

const fn at(date: NaiveDate, hour: u32, minute: u32) -> NaiveDateTime {
    date.and_hms_opt(hour, minute, 0).expect("a real time")
}

#[tokio::test]
async fn a_fresh_database_has_only_the_default_partition() {
    let pool = test_pool("jobs", "fresh").await;
    let conn = pool.get().await.expect("connection");

    let found = conn
        .interact(partitions::existing)
        .await
        .expect("interact")
        .expect("catalog query");

    assert_eq!(found, vec!["measurements_default".to_owned()]);
}

#[tokio::test]
async fn partitions_are_created_ahead_of_the_current_month() {
    let pool = test_pool("jobs", "ahead").await;
    let conn = pool.get().await.expect("connection");

    let created = conn
        .interact(|conn| partitions::ensure_ahead(conn, day(2026, 9, 17), 2))
        .await
        .expect("interact")
        .expect("partition creation");

    assert_eq!(
        created,
        vec![
            "measurements_2026_09".to_owned(),
            "measurements_2026_10".to_owned(),
            "measurements_2026_11".to_owned(),
        ]
    );

    let found = conn
        .interact(partitions::existing)
        .await
        .expect("interact")
        .expect("catalog query");
    assert_eq!(found.len(), 4, "three months plus the default: {found:?}");
}

#[tokio::test]
async fn creating_a_partition_twice_is_a_no_op() {
    let pool = test_pool("jobs", "idempotent").await;
    let conn = pool.get().await.expect("connection");

    let first = conn
        .interact(|conn| partitions::ensure_ahead(conn, day(2026, 9, 17), 0))
        .await
        .expect("interact")
        .expect("first pass");
    let second = conn
        .interact(|conn| partitions::ensure_ahead(conn, day(2026, 9, 17), 0))
        .await
        .expect("interact")
        .expect("second pass");

    assert_eq!(first.len(), 1);
    assert!(second.is_empty(), "the second pass created {second:?}");
}

#[tokio::test]
async fn rows_already_in_the_default_partition_move_into_the_new_one() {
    let pool = test_pool("jobs", "migrate_rows").await;
    let (node_id, channel_id) = seed_node_and_channel(&pool).await;

    seed_measurement(
        &pool,
        node_id,
        channel_id,
        at(day(2026, 9, 17), 12, 0),
        30.0,
        60,
    )
    .await;

    let conn = pool.get().await.expect("connection");
    let default_before: i64 = conn
        .interact(|conn| {
            diesel::sql_query("SELECT count(*) AS count FROM measurements_default")
                .get_result::<Count>(conn)
                .map(|row| row.count)
        })
        .await
        .expect("interact")
        .expect("count");
    assert_eq!(default_before, 1);

    conn.interact(|conn| partitions::ensure_ahead(conn, day(2026, 9, 17), 0))
        .await
        .expect("interact")
        .expect("partition creation");

    let (default_after, month_after, total) = conn
        .interact(|conn| {
            let default_after =
                diesel::sql_query("SELECT count(*) AS count FROM measurements_default")
                    .get_result::<Count>(conn)
                    .expect("default count")
                    .count;
            let month_after =
                diesel::sql_query("SELECT count(*) AS count FROM measurements_2026_09")
                    .get_result::<Count>(conn)
                    .expect("month count")
                    .count;
            let total = measurements_schema::table
                .count()
                .get_result::<i64>(conn)
                .expect("total count");

            (default_after, month_after, total)
        })
        .await
        .expect("interact");

    assert_eq!(default_after, 0, "the row did not leave the default");
    assert_eq!(month_after, 1, "the row did not land in its month");
    assert_eq!(total, 1, "the row was duplicated rather than moved");
}

#[tokio::test]
async fn a_partition_past_the_horizon_is_dropped() {
    let pool = test_pool("jobs", "drop_old").await;
    let conn = pool.get().await.expect("connection");

    conn.interact(|conn| partitions::ensure_ahead(conn, day(2026, 1, 5), 2))
        .await
        .expect("interact")
        .expect("partition creation");

    let dropped = conn
        .interact(|conn| partitions::drop_before(conn, day(2026, 3, 1)))
        .await
        .expect("interact")
        .expect("drop");

    assert_eq!(
        dropped,
        vec![
            "measurements_2026_01".to_owned(),
            "measurements_2026_02".to_owned(),
        ]
    );

    let found = conn
        .interact(partitions::existing)
        .await
        .expect("interact")
        .expect("catalog query");
    assert!(
        found.contains(&"measurements_default".to_owned()),
        "the default partition was dropped: {found:?}"
    );
    assert!(found.contains(&"measurements_2026_03".to_owned()));
}

#[tokio::test]
async fn hourly_buckets_are_built_from_raw_windows() {
    let pool = test_pool("jobs", "hourly").await;
    let (node_id, channel_id) = seed_node_and_channel(&pool).await;
    let date = day(2026, 9, 17);

    for minute in 0..60 {
        seed_measurement(
            &pool,
            node_id,
            channel_id,
            at(date, 10, minute),
            f64::from(minute).mul_add(0.01, 30.0),
            60,
        )
        .await;
    }

    let conn = pool.get().await.expect("connection");
    let written = conn
        .interact(move |conn| rollup::run(conn, RollupResolution::Hourly, at(date, 12, 0)))
        .await
        .expect("interact")
        .expect("rollup");

    assert_eq!(written, 1, "one node, channel and metric is one row");

    let (bucket_start, sample_count, mean) = conn
        .interact(|conn| {
            rollups_schema::table
                .filter(rollups_schema::resolution.eq(RollupResolution::Hourly))
                .select((
                    rollups_schema::bucket_start,
                    rollups_schema::sample_count,
                    rollups_schema::mean,
                ))
                .first::<(NaiveDateTime, i64, f64)>(conn)
        })
        .await
        .expect("interact")
        .expect("one bucket");

    assert_eq!(bucket_start, at(date, 10, 0));
    assert_eq!(sample_count, 60 * 60);
    assert!((mean - 30.295).abs() < 1e-9, "the mean is {mean}");
}

#[tokio::test]
async fn an_incomplete_bucket_is_not_built() {
    let pool = test_pool("jobs", "incomplete").await;
    let (node_id, channel_id) = seed_node_and_channel(&pool).await;
    let date = day(2026, 9, 17);

    seed_measurement(&pool, node_id, channel_id, at(date, 10, 0), 30.0, 60).await;

    let conn = pool.get().await.expect("connection");
    let written = conn
        .interact(move |conn| rollup::run(conn, RollupResolution::Hourly, at(date, 10, 30)))
        .await
        .expect("interact")
        .expect("rollup");

    assert_eq!(written, 0);
}

#[tokio::test]
async fn rerunning_the_rollup_writes_nothing_new() {
    let pool = test_pool("jobs", "rerun").await;
    let (node_id, channel_id) = seed_node_and_channel(&pool).await;
    let date = day(2026, 9, 17);

    seed_measurement(&pool, node_id, channel_id, at(date, 10, 0), 30.0, 60).await;

    let conn = pool.get().await.expect("connection");
    let first = conn
        .interact(move |conn| rollup::run(conn, RollupResolution::Hourly, at(date, 12, 0)))
        .await
        .expect("interact")
        .expect("first run");
    let second = conn
        .interact(move |conn| rollup::run(conn, RollupResolution::Hourly, at(date, 12, 0)))
        .await
        .expect("interact")
        .expect("second run");

    assert_eq!(first, 1);
    assert_eq!(second, 0, "the second run duplicated a bucket");
}

#[tokio::test]
async fn all_three_resolutions_cover_the_same_windows() {
    let pool = test_pool("jobs", "resolutions").await;
    let (node_id, channel_id) = seed_node_and_channel(&pool).await;
    let monday = day(2026, 9, 14);

    for hour in 0..4 {
        seed_measurement(&pool, node_id, channel_id, at(monday, hour, 0), 30.0, 60).await;
    }

    let conn = pool.get().await.expect("connection");
    let later = monday
        .checked_add_days(Days::new(14))
        .expect("a real date")
        .and_time(NaiveTime::MIN);

    conn.interact(move |conn| rollup::run_all(conn, later))
        .await
        .expect("interact")
        .expect("rollups");

    for (resolution, expected_buckets) in [
        (RollupResolution::Hourly, 4_i64),
        (RollupResolution::Daily, 1),
        (RollupResolution::Weekly, 1),
    ] {
        let (buckets, samples) = conn
            .interact(move |conn| {
                let buckets = rollups_schema::table
                    .filter(rollups_schema::resolution.eq(resolution))
                    .count()
                    .get_result::<i64>(conn)
                    .expect("bucket count");
                let samples: i64 = rollups_schema::table
                    .filter(rollups_schema::resolution.eq(resolution))
                    .select(rollups_schema::sample_count)
                    .load::<i64>(conn)
                    .expect("sample counts")
                    .into_iter()
                    .sum();

                (buckets, samples)
            })
            .await
            .expect("interact");

        assert_eq!(buckets, expected_buckets, "{resolution:?} bucket count");
        assert_eq!(samples, 240, "{resolution:?} covers the same 240 samples");
    }
}

#[tokio::test]
async fn pruning_deletes_only_what_is_past_the_horizon() {
    let pool = test_pool("jobs", "prune").await;
    let (node_id, channel_id) = seed_node_and_channel(&pool).await;
    let old = at(day(2026, 8, 1), 10, 0);
    let recent = at(day(2026, 9, 17), 10, 0);

    seed_measurement(&pool, node_id, channel_id, old, 30.0, 60).await;
    seed_measurement(&pool, node_id, channel_id, recent, 31.0, 60).await;

    let conn = pool.get().await.expect("connection");
    let deleted = conn
        .interact(move |conn| rollup::prune_raw(conn, at(day(2026, 9, 1), 0, 0)))
        .await
        .expect("interact")
        .expect("prune");

    assert_eq!(deleted, 1);

    let remaining: Vec<NaiveDateTime> = conn
        .interact(|conn| {
            measurements_schema::table
                .select(measurements_schema::window_start)
                .load(conn)
        })
        .await
        .expect("interact")
        .expect("load");

    assert_eq!(remaining, vec![recent]);
}

#[derive(diesel::QueryableByName)]
struct Count {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    count: i64,
}

async fn seed_rollup(
    pool: &Pool,
    node_id: i64,
    channel_id: i64,
    bucket_start: NaiveDateTime,
    mean: f64,
    stddev: f64,
) {
    let conn = pool.get().await.expect("seed connection");
    let row = NewRollup {
        node_id,
        channel_id,
        metric: Metric::SignalToNoise,
        resolution: RollupResolution::Hourly,
        bucket_start,
        bucket_end: bucket_start + TimeDelta::hours(1),
        min: mean - 2.0,
        max: mean + 2.0,
        mean,
        median: mean,
        stddev,
        sample_count: 3_600,
    };

    conn.interact(move |conn| {
        diesel::insert_into(rollups_schema::table)
            .values(&row)
            .execute(conn)
    })
    .await
    .expect("seed interact failed")
    .expect("rollup insert");
}

async fn seed_baseline(pool: &Pool, node_id: i64, channel_id: i64, now: NaiveDateTime, mean: f64) {
    for back in 1..=28_u64 {
        let Some(day) = now.checked_sub_days(Days::new(back)) else {
            continue;
        };
        let bucket = day
            .date()
            .and_hms_opt(now.hour(), 0, 0)
            .expect("a real time");
        let wobble = f64::from(u32::try_from(back % 5).unwrap_or(0)) * 0.1 - 0.2;

        seed_rollup(pool, node_id, channel_id, bucket, mean + wobble, 0.4).await;
    }
}

async fn seed_recent_windows(
    pool: &Pool,
    node_id: i64,
    channel_id: i64,
    now: NaiveDateTime,
    values: [f64; 3],
) {
    for (index, value) in values.into_iter().enumerate() {
        let minutes = i64::try_from(index + 1).unwrap_or(1);
        let start = now - TimeDelta::minutes(minutes);

        seed_measurement(pool, node_id, channel_id, start, value, 60).await;
    }
}

async fn alarms(pool: &Pool) -> Vec<(i64, Option<i64>, serde_json::Value)> {
    let conn = pool.get().await.expect("connection");

    conn.interact(|conn| {
        alarms_schema::table
            .select((
                alarms_schema::node_id,
                alarms_schema::channel_id,
                alarms_schema::explanation,
            ))
            .order(alarms_schema::id.asc())
            .load(conn)
    })
    .await
    .expect("interact")
    .expect("alarm load")
}

#[tokio::test]
async fn a_node_with_no_history_is_not_judged() {
    let pool = test_pool("jobs", "no_history").await;
    let (node_id, channel_id) = seed_node_and_channel(&pool).await;
    let now = at(day(2026, 9, 17), 13, 30);

    seed_recent_windows(&pool, node_id, channel_id, now, [2.0, 2.0, 2.0]).await;

    let conn = pool.get().await.expect("connection");
    let raised = conn
        .interact(move |conn| {
            detector::examine(conn, node_id, channel_id, Metric::SignalToNoise, now)
        })
        .await
        .expect("interact")
        .expect("examine");

    assert_eq!(raised, None);
    assert!(alarms(&pool).await.is_empty());
}

#[tokio::test]
async fn a_healthy_series_raises_nothing() {
    let pool = test_pool("jobs", "healthy").await;
    let (node_id, channel_id) = seed_node_and_channel(&pool).await;
    let now = at(day(2026, 9, 17), 13, 30);

    seed_baseline(&pool, node_id, channel_id, now, 30.0).await;
    seed_recent_windows(&pool, node_id, channel_id, now, [30.1, 29.9, 30.0]).await;

    let conn = pool.get().await.expect("connection");
    let raised = conn
        .interact(move |conn| {
            detector::examine(conn, node_id, channel_id, Metric::SignalToNoise, now)
        })
        .await
        .expect("interact")
        .expect("examine");

    assert_eq!(raised, None);
    assert!(alarms(&pool).await.is_empty());
}

#[tokio::test]
async fn a_sustained_collapse_raises_an_alarm_that_explains_itself() {
    let pool = test_pool("jobs", "collapse").await;
    let (node_id, channel_id) = seed_node_and_channel(&pool).await;
    let now = at(day(2026, 9, 17), 13, 30);

    seed_baseline(&pool, node_id, channel_id, now, 30.0).await;
    seed_recent_windows(&pool, node_id, channel_id, now, [12.0, 11.5, 12.4]).await;

    let conn = pool.get().await.expect("connection");
    let raised = conn
        .interact(move |conn| {
            detector::examine(conn, node_id, channel_id, Metric::SignalToNoise, now)
        })
        .await
        .expect("interact")
        .expect("examine");

    assert!(raised.is_some(), "a 18 dB drop did not raise an alarm");

    let raised_alarms = alarms(&pool).await;
    assert_eq!(raised_alarms.len(), 1);

    let (alarm_node, alarm_channel, explanation) =
        raised_alarms.into_iter().next().expect("one alarm");
    assert_eq!(alarm_node, node_id);
    assert_eq!(alarm_channel, Some(channel_id));

    assert_eq!(explanation["kind"], "baseline_deviation");
    assert_eq!(explanation["metric"], "signal_to_noise");
    assert_eq!(explanation["hour_bucket"], 13);
    assert_eq!(explanation["direction"], "below");
    assert!(explanation["baseline"]["center"].is_number());
    assert!(explanation["threshold"]["half_width"].is_number());
    assert!(explanation["threshold"]["low"].is_number());
    assert!(
        explanation["severity"]
            .as_f64()
            .is_some_and(|severity| severity > 1.0),
        "a raised alarm should be past the edge of its band"
    );
}

#[tokio::test]
async fn a_raised_alarm_carries_the_event_that_raised_it() {
    let pool = test_pool("jobs", "event").await;
    let (node_id, channel_id) = seed_node_and_channel(&pool).await;
    let now = at(day(2026, 9, 17), 13, 30);

    seed_baseline(&pool, node_id, channel_id, now, 30.0).await;
    seed_recent_windows(&pool, node_id, channel_id, now, [12.0, 11.5, 12.4]).await;

    let conn = pool.get().await.expect("connection");
    let alarm_id = conn
        .interact(move |conn| {
            detector::examine(conn, node_id, channel_id, Metric::SignalToNoise, now)
        })
        .await
        .expect("interact")
        .expect("examine")
        .expect("an alarm");

    let events: Vec<(Option<AlarmState>, AlarmState, Option<i64>)> = conn
        .interact(move |conn| {
            alarm_events_schema::table
                .filter(alarm_events_schema::alarm_id.eq(alarm_id))
                .select((
                    alarm_events_schema::from_state,
                    alarm_events_schema::to_state,
                    alarm_events_schema::changed_by_user_id,
                ))
                .load(conn)
        })
        .await
        .expect("interact")
        .expect("events");

    assert_eq!(events, vec![(None, AlarmState::Open, None)]);
}

#[tokio::test]
async fn a_signal_crossing_both_edges_is_noise_not_a_fault() {
    let pool = test_pool("jobs", "oscillating").await;
    let (node_id, channel_id) = seed_node_and_channel(&pool).await;
    let now = at(day(2026, 9, 17), 13, 30);

    seed_baseline(&pool, node_id, channel_id, now, 30.0).await;
    seed_recent_windows(&pool, node_id, channel_id, now, [12.0, 48.0, 12.0]).await;

    let conn = pool.get().await.expect("connection");
    let raised = conn
        .interact(move |conn| {
            detector::examine(conn, node_id, channel_id, Metric::SignalToNoise, now)
        })
        .await
        .expect("interact")
        .expect("examine");

    assert_eq!(raised, None);
}

#[tokio::test]
async fn one_deviation_raises_one_alarm_however_often_the_detector_runs() {
    let pool = test_pool("jobs", "no_duplicate").await;
    let (node_id, channel_id) = seed_node_and_channel(&pool).await;
    let now = at(day(2026, 9, 17), 13, 30);

    seed_baseline(&pool, node_id, channel_id, now, 30.0).await;
    seed_recent_windows(&pool, node_id, channel_id, now, [12.0, 11.5, 12.4]).await;

    let conn = pool.get().await.expect("connection");
    for _ in 0..3 {
        conn.interact(move |conn| {
            detector::examine(conn, node_id, channel_id, Metric::SignalToNoise, now)
        })
        .await
        .expect("interact")
        .expect("examine");
    }

    assert_eq!(alarms(&pool).await.len(), 1);
}

#[tokio::test]
async fn the_detector_finds_a_deviation_without_being_told_where() {
    let pool = test_pool("jobs", "detector_run").await;
    let (node_id, channel_id) = seed_node_and_channel(&pool).await;
    let now = at(day(2026, 9, 17), 13, 30);

    seed_baseline(&pool, node_id, channel_id, now, 30.0).await;
    seed_recent_windows(&pool, node_id, channel_id, now, [12.0, 11.5, 12.4]).await;

    let conn = pool.get().await.expect("connection");
    let raised = conn
        .interact(move |conn| detector::run(conn, now))
        .await
        .expect("interact")
        .expect("detector run");

    assert_eq!(raised.len(), 1);
}

async fn set_last_seen(pool: &Pool, node_id: i64, at: Option<NaiveDateTime>) {
    let conn = pool.get().await.expect("connection");

    conn.interact(move |conn| {
        diesel::update(nodes_schema::table.filter(nodes_schema::id.eq(node_id)))
            .set(nodes_schema::last_seen_at.eq(at))
            .execute(conn)
    })
    .await
    .expect("interact")
    .expect("update");
}

#[tokio::test]
async fn a_node_that_stopped_reporting_raises_a_whole_node_alarm() {
    let pool = test_pool("jobs", "silent").await;
    let (node_id, _) = seed_node_and_channel(&pool).await;
    let now = at(day(2026, 9, 17), 13, 30);

    set_last_seen(&pool, node_id, Some(now - TimeDelta::minutes(30))).await;

    let conn = pool.get().await.expect("connection");
    let silences = conn
        .interact(move |conn| detector::silence(conn, now))
        .await
        .expect("interact")
        .expect("silence check");

    assert_eq!(silences.len(), 1);

    let raised = alarms(&pool).await;
    let (alarm_node, alarm_channel, explanation) = raised.into_iter().next().expect("one alarm");
    assert_eq!(alarm_node, node_id);
    assert_eq!(alarm_channel, None, "silence is not a channel's problem");
    assert_eq!(explanation["kind"], "node_silence");
    assert_eq!(explanation["threshold_minutes"], 10);
}

#[tokio::test]
async fn a_node_still_reporting_is_not_silent() {
    let pool = test_pool("jobs", "talkative").await;
    let (node_id, _) = seed_node_and_channel(&pool).await;
    let now = at(day(2026, 9, 17), 13, 30);

    set_last_seen(&pool, node_id, Some(now - TimeDelta::minutes(2))).await;

    let conn = pool.get().await.expect("connection");
    let silences = conn
        .interact(move |conn| detector::silence(conn, now))
        .await
        .expect("interact")
        .expect("silence check");

    assert!(silences.is_empty());
}

#[tokio::test]
async fn a_node_that_has_never_reported_is_new_not_silent() {
    let pool = test_pool("jobs", "never_reported").await;
    let (node_id, _) = seed_node_and_channel(&pool).await;
    let now = at(day(2026, 9, 17), 13, 30);

    set_last_seen(&pool, node_id, None).await;

    let conn = pool.get().await.expect("connection");
    let silences = conn
        .interact(move |conn| detector::silence(conn, now))
        .await
        .expect("interact")
        .expect("silence check");

    assert!(silences.is_empty());
}

#[tokio::test]
async fn a_suspended_node_is_quiet_on_purpose() {
    let pool = test_pool("jobs", "suspended").await;
    let (node_id, _) = seed_node_and_channel(&pool).await;
    let now = at(day(2026, 9, 17), 13, 30);

    set_last_seen(&pool, node_id, Some(now - TimeDelta::hours(3))).await;
    let conn = pool.get().await.expect("connection");
    conn.interact(move |conn| {
        diesel::update(nodes_schema::table.filter(nodes_schema::id.eq(node_id)))
            .set(nodes_schema::suspended.eq(true))
            .execute(conn)
    })
    .await
    .expect("interact")
    .expect("suspend");

    let silences = conn
        .interact(move |conn| detector::silence(conn, now))
        .await
        .expect("interact")
        .expect("silence check");

    assert!(silences.is_empty());
}

#[tokio::test]
async fn one_silence_raises_one_alarm() {
    let pool = test_pool("jobs", "silence_once").await;
    let (node_id, _) = seed_node_and_channel(&pool).await;
    let now = at(day(2026, 9, 17), 13, 30);

    set_last_seen(&pool, node_id, Some(now - TimeDelta::hours(3))).await;

    let conn = pool.get().await.expect("connection");
    for _ in 0..3 {
        conn.interact(move |conn| detector::silence(conn, now))
            .await
            .expect("interact")
            .expect("silence check");
    }

    assert_eq!(alarms(&pool).await.len(), 1);
}
