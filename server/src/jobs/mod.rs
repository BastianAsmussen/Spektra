pub mod detector;
pub mod partitions;
pub mod rollup;

use std::time::Duration;

use chrono::{Days, Utc};
use deadpool_diesel::postgres::Pool;

use crate::notify::{Notice, Ntfy};
use crate::state::{AlarmEvent, AppState, NodeEvent};

///
const INTERVAL: Duration = Duration::from_hours(1);

///
const DETECTION_INTERVAL: Duration = Duration::from_mins(1);

///
const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

/// Days of partitions kept ahead of the current one.
const PARTITIONS_AHEAD: u64 = 7;

///
const RAW_RETENTION_DAYS: u64 = 14;

const HOURLY_RETENTION_DAYS: u64 = 90;

const DAILY_RETENTION_DAYS: u64 = 730;

///
pub async fn run(state: AppState) {
    let mut timer = tokio::time::interval(INTERVAL);

    loop {
        timer.tick().await;

        match once(&state.ingest_pool).await {
            Ok(()) => state.metrics.maintenance_finished(),
            Err(err) => tracing::error!(error = %err, "the maintenance pass failed"),
        }
    }
}

/// One maintenance pass.
///
///
/// # Errors
///
pub async fn once(pool: &Pool) -> Result<(), String> {
    let conn = pool.get().await.map_err(|err| err.to_string())?;
    conn.interact(move |conn| {
        let today = partitions::today();

        let first = today
            .checked_sub_days(Days::new(RAW_RETENTION_DAYS))
            .unwrap_or(today);
        let last = today
            .checked_add_days(Days::new(PARTITIONS_AHEAD))
            .unwrap_or(today);

        match partitions::ensure_range(conn, first, last) {
            Ok(created) if !created.is_empty() => {
                tracing::info!(?created, "measurement partitions created");
            }
            Ok(_) => {}
            Err(err) => tracing::error!(error = %err, "could not create measurement partitions"),
        }

        let now = rollup::now();
        match rollup::run_all(conn, now) {
            Ok(0) => {}
            Ok(written) => tracing::info!(written, "rollup buckets written"),
            Err(err) => {
                tracing::error!(error = %err, "the rollup job failed");

                return;
            }
        }

        prune(conn);
    })
    .await
    .map_err(|err| format!("the maintenance task panicked: {err:?}"))
}

/// Run the detector forever.
///
pub async fn detect(state: AppState, notifier: Option<Ntfy>) {
    let mut timer = tokio::time::interval(DETECTION_INTERVAL);

    loop {
        timer.tick().await;

        if let Err(err) = detect_once(&state, notifier.as_ref()).await {
            tracing::error!(error = %err, "the detection pass failed");
        }
    }
}

/// One detection pass: deviations first, then silence.
///
/// # Errors
///
pub async fn detect_once(state: &AppState, notifier: Option<&Ntfy>) -> Result<(), String> {
    let conn = state
        .ingest_pool
        .get()
        .await
        .map_err(|err| err.to_string())?;

    let (deviations, silences) = conn
        .interact(move |conn| {
            let now = rollup::now();
            let deviations = match detector::run(conn, now) {
                Ok(raised) => raised,
                Err(err) => {
                    tracing::error!(error = %err, "the deviation detector failed");

                    Vec::new()
                }
            };

            let silences = match detector::silence(conn, now) {
                Ok(raised) => raised,
                Err(err) => {
                    tracing::error!(error = %err, "the silence check failed");

                    Vec::new()
                }
            };

            (deviations, silences)
        })
        .await
        .map_err(|err| format!("the detection task panicked: {err:?}"))?;

    state
        .metrics
        .detection_finished(deviations.len().saturating_add(silences.len()));

    publish(state, notifier, &deviations, &silences).await;

    Ok(())
}

async fn publish(
    state: &AppState,
    notifier: Option<&Ntfy>,
    deviations: &[i64],
    silences: &[detector::Silence],
) {
    if deviations.is_empty() && silences.is_empty() {
        return;
    }

    for silence in silences {
        state.publish_node(NodeEvent::Silent {
            node_id: silence.node_id,
            since: silence.since,
        });
    }

    let ids: Vec<i64> = deviations
        .iter()
        .copied()
        .chain(silences.iter().map(|silence| silence.alarm_id))
        .collect();

    for notice in raised_notices(state, ids).await {
        state.publish_alarm(AlarmEvent::Raised {
            alarm_id: notice.alarm_id,
            node_id: notice.node_id,
            metric: notice.metric,
        });

        if let Some(ntfy) = notifier {
            ntfy.publish(&notice).await;
        }
    }

    tracing::info!(
        deviations = deviations.len(),
        silences = silences.len(),
        at = %Utc::now().naive_utc(),
        "alarms raised"
    );
}

async fn raised_notices(state: &AppState, ids: Vec<i64>) -> Vec<Notice> {
    let conn = match state.ingest_pool.get().await {
        Ok(conn) => conn,
        Err(err) => {
            tracing::error!(error = %err, "could not read back the raised alarms");

            return Vec::new();
        }
    };

    let rows = conn
        .interact(move |conn| {
            use crate::db::models::enums::Metric;
            use crate::db::schema::{alarms as alarms_schema, nodes as nodes_schema};
            use diesel::prelude::*;

            alarms_schema::table
                .inner_join(nodes_schema::table)
                .filter(alarms_schema::id.eq_any(&ids))
                .select((
                    alarms_schema::id,
                    alarms_schema::node_id,
                    nodes_schema::name,
                    alarms_schema::metric,
                    alarms_schema::explanation,
                ))
                .load::<(i64, i64, String, Option<Metric>, serde_json::Value)>(conn)
        })
        .await;

    match rows {
        Ok(Ok(rows)) => rows.into_iter().map(notice).collect(),
        Ok(Err(err)) => {
            tracing::error!(error = %err, "could not read back the raised alarms");

            Vec::new()
        }
        Err(err) => {
            tracing::error!(error = ?err, "the alarm read-back task panicked");

            Vec::new()
        }
    }
}

fn notice(
    (alarm_id, node_id, node_name, metric, explanation): (
        i64,
        i64,
        String,
        Option<crate::db::models::enums::Metric>,
        serde_json::Value,
    ),
) -> Notice {
    let severity = explanation
        .get("severity")
        .and_then(serde_json::Value::as_f64);

    let summary = metric.map_or_else(
        || silence_summary(&explanation),
        |metric| deviation_summary(metric, &explanation),
    );

    Notice {
        alarm_id,
        node_id,
        node_name,
        metric,
        severity,
        summary,
    }
}

fn silence_summary(explanation: &serde_json::Value) -> String {
    explanation.get("last_seen_at").map_or_else(
        || "the node stopped reporting".to_owned(),
        |seen| format!("no delivery since {seen}"),
    )
}

///
fn deviation_summary(
    metric: crate::db::models::enums::Metric,
    explanation: &serde_json::Value,
) -> String {
    let number = |pointer: &str| {
        explanation
            .pointer(pointer)
            .and_then(serde_json::Value::as_f64)
            .unwrap_or_default()
    };

    let direction = explanation
        .get("direction")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("outside");

    format!(
        "{} read {:.1}, {direction} a baseline of {:.1}",
        metric.label(),
        number("/value"),
        number("/baseline/center")
    )
}

fn prune(conn: &mut diesel::pg::PgConnection) {
    use crate::db::models::enums::RollupResolution;

    if let Some(before) = rollup::horizon(RAW_RETENTION_DAYS) {
        match rollup::prune_raw(conn, before) {
            Ok(0) => {}
            Ok(deleted) => tracing::info!(deleted, %before, "raw measurement windows pruned"),
            Err(err) => tracing::error!(error = %err, "could not prune raw measurements"),
        }

        match partitions::drop_before(conn, before.date()) {
            Ok(dropped) if !dropped.is_empty() => {
                tracing::info!(?dropped, "empty measurement partitions dropped");
            }
            Ok(_) => {}
            Err(err) => tracing::error!(error = %err, "could not drop old partitions"),
        }
    }

    for (resolution, days) in [
        (RollupResolution::Hourly, HOURLY_RETENTION_DAYS),
        (RollupResolution::Daily, DAILY_RETENTION_DAYS),
    ] {
        let Some(before) = rollup::horizon(days) else {
            continue;
        };

        match rollup::prune_rollups(conn, resolution, before) {
            Ok(0) => {}
            Ok(deleted) => tracing::info!(deleted, ?resolution, "rollup buckets pruned"),
            Err(err) => tracing::error!(error = %err, ?resolution, "could not prune rollups"),
        }
    }
}

///
pub async fn sample_throughput(state: AppState) {
    let mut timer = tokio::time::interval(SAMPLE_INTERVAL);

    loop {
        timer.tick().await;

        state.metrics.sample();
        state.live.sweep();
    }
}
