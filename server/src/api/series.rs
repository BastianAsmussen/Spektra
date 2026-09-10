use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{NaiveDateTime, TimeDelta, Utc};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::auth::AuthUser;
use super::errors::{ApiError, ErrorBody};
use super::visibility;
use crate::db::models::enums::{Metric, RollupResolution};
use crate::db::schema::{measurements as measurements_schema, rollups as rollups_schema};
use crate::jobs::detector;
use crate::state::AppState;

/// Longest span a request may ask for, in days.
const MAX_SPAN_DAYS: i64 = 730;

const DEFAULT_SPAN_HOURS: i64 = 24;

const MAX_POINTS: i64 = 20_000;

/// All routes serving measurement history.
pub fn routes() -> Router<AppState> {
    Router::new().route("/api/series/{node}/{channel}/{metric}", get(get_series))
}

/// How far back a chart reaches.
#[derive(Debug, Deserialize, IntoParams)]
pub struct SpanQuery {
    /// Hours of history. Defaults to a day, capped at two years.
    pub hours: Option<i64>,
}

impl SpanQuery {
    /// The span as a duration, clamped to something serveable.
    #[must_use]
    pub fn duration(&self) -> TimeDelta {
        let hours = self.hours.unwrap_or(DEFAULT_SPAN_HOURS).max(1);

        TimeDelta::hours(hours.min(MAX_SPAN_DAYS.saturating_mul(24)))
    }
}

/// Where a span's points come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The raw aggregation windows a node reported.
    Raw,
    /// Hourly rollup buckets.
    Hourly,
    /// Daily rollup buckets.
    Daily,
    /// Weekly rollup buckets.
    Weekly,
}

impl Source {
    /// The source that can serve a span without reading pruned rows.
    #[must_use]
    pub const fn for_span(span: TimeDelta) -> Self {
        let hours = span.num_hours();

        if hours <= 6 {
            Self::Raw
        } else if hours <= 24 * 7 {
            Self::Hourly
        } else if hours <= 24 * 90 {
            Self::Daily
        } else {
            Self::Weekly
        }
    }

    /// The rollup resolution this source reads, or `None` for raw windows.
    #[must_use]
    pub const fn resolution(self) -> Option<RollupResolution> {
        match self {
            Self::Raw => None,
            Self::Hourly => Some(RollupResolution::Hourly),
            Self::Daily => Some(RollupResolution::Daily),
            Self::Weekly => Some(RollupResolution::Weekly),
        }
    }
}

/// One measured point.
#[derive(Debug, Default, Serialize, ToSchema)]
pub struct Points {
    /// Window starts as Unix seconds, oldest first.
    pub at: Vec<i64>,
    /// The mean over each window, in the metric's stored unit.
    pub value: Vec<f64>,
}

/// How a metric is drawn: what to call it, and in what unit.
#[derive(Debug, Serialize, ToSchema)]
pub struct Presentation {
    /// Danish name, for the chart's caption.
    pub name: &'static str,
    /// Unit the values carry once scaled, for the axis.
    pub unit: &'static str,
    /// What to multiply a stored value by to reach that unit.
    pub scale: f64,
}

/// One metric's history, and the baseline it is judged against.
#[derive(Debug, Serialize, ToSchema)]
pub struct Series {
    pub node_id: i64,
    pub channel_id: i64,
    #[schema(value_type = String)]
    pub metric: Metric,
    /// Which table the points came from.
    pub source: Source,
    /// What the source is called, in Danish, for the caption.
    pub source_name: &'static str,
    /// How to label and scale the values.
    pub presentation: Presentation,
    pub points: Points,
    /// Detector acceptance interval, when enough history exists.
    pub band: Option<(f64, f64)>,
}

const fn presentation(metric: Metric) -> Presentation {
    match metric {
        Metric::SignalStrength => Presentation {
            name: "Signalstyrke",
            unit: "dBFS",
            scale: 1.0,
        },
        Metric::SignalToNoise => Presentation {
            name: "Signal-støjforhold",
            unit: "dB",
            scale: 1.0,
        },
        Metric::CarrierOffset => Presentation {
            name: "Bærebølgeafvigelse",
            unit: "Hz",
            scale: 1.0,
        },
        Metric::DemodErrorRate => Presentation {
            name: "Demodulationsfejl",
            unit: "%",
            scale: 100.0,
        },
        Metric::SpectrumOccupancy => Presentation {
            name: "Båndudnyttelse",
            unit: "%",
            scale: 100.0,
        },
    }
}

/// One reading as the live strip writes it: Danish name and scaled value.
#[must_use]
pub fn reading(metric: Metric, value: f64) -> (&'static str, String) {
    let drawn = presentation(metric);
    let scaled = value * drawn.scale;
    let decimals = if drawn.unit == "%" { 2 } else { 1 };
    let unit = drawn.unit;

    (drawn.name, format!("{scaled:.decimals$} {unit}"))
}

/// One metric's history as JSON.
///
/// # Errors
///
/// Returns [`ApiError`] for a missing session, a forbidden node, or a database failure.
#[utoipa::path(
    get,
    path = "/api/series/{node}/{channel}/{metric}",
    params(
        ("node" = i64, Path, description = "Node id"),
        ("channel" = i64, Path, description = "Channel id"),
        ("metric" = String, Path, description = "Metric label, e.g. signal_to_noise"),
        SpanQuery,
    ),
    responses(
        (status = 200, description = "The metric's history", body = Series),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 403, description = "Not visible to this user", body = ErrorBody),
        (status = 422, description = "Not a known metric", body = ErrorBody),
    ),
    security(("session_token" = [])),
    tag = "series"
)]
pub async fn get_series(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((node_id, channel_id, metric)): Path<(i64, i64, String)>,
    Query(span): Query<SpanQuery>,
) -> Result<Json<Series>, ApiError> {
    let access = visibility::resolve(&state, auth.session.user_id).await?;
    if !access.visibility.allows(node_id) {
        return Err(ApiError::Forbidden(
            "This node is not one you are dispatched to.".into(),
        ));
    }

    let metric = parse_metric(&metric)?;

    load(&state, node_id, channel_id, metric, span.duration()).await
}

const fn source_label(source: Source) -> &'static str {
    match source {
        Source::Raw => "rå målinger",
        Source::Hourly => "timeopsummering",
        Source::Daily => "dagsopsummering",
        Source::Weekly => "ugeopsummering",
    }
}

fn parse_metric(label: &str) -> Result<Metric, ApiError> {
    label
        .parse()
        .map_err(|_| ApiError::UnprocessableEntity(format!("'{label}' is not a known metric.")))
}

async fn load(
    state: &AppState,
    node_id: i64,
    channel_id: i64,
    metric: Metric,
    span: TimeDelta,
) -> Result<Json<Series>, ApiError> {
    let now = Utc::now().naive_utc();
    let since = now.checked_sub_signed(span).unwrap_or(now);
    let source = Source::for_span(span);

    let conn = state.pool.get().await?;
    let points = conn
        .interact(move |conn| read(conn, node_id, channel_id, metric, source, since))
        .await??;
    let points = columns(points, presentation(metric).scale);

    let band = conn
        .interact(move |conn| {
            detector::baseline(conn, node_id, channel_id, metric, hour_of(now), now)
        })
        .await??
        .map(|baseline| baseline.band());

    let presentation = presentation(metric);
    let scale = presentation.scale;

    Ok(Json(Series {
        node_id,
        channel_id,
        metric,
        source,
        source_name: source_label(source),
        presentation,
        points,
        band: band.map(|(low, high)| (low * scale, high * scale)),
    }))
}

fn hour_of(at: NaiveDateTime) -> u32 {
    use chrono::Timelike as _;

    at.hour()
}

fn columns(rows: Vec<(NaiveDateTime, f64)>, scale: f64) -> Points {
    let mut points = Points {
        at: Vec::with_capacity(rows.len()),
        value: Vec::with_capacity(rows.len()),
    };

    for (at, value) in rows {
        points.at.push(at.and_utc().timestamp());
        points.value.push(value * scale);
    }

    points
}

fn read(
    conn: &mut diesel::pg::PgConnection,
    node_id: i64,
    channel_id: i64,
    metric: Metric,
    source: Source,
    since: NaiveDateTime,
) -> QueryResult<Vec<(NaiveDateTime, f64)>> {
    match source.resolution() {
        None => measurements_schema::table
            .filter(measurements_schema::node_id.eq(node_id))
            .filter(measurements_schema::channel_id.eq(channel_id))
            .filter(measurements_schema::metric.eq(metric))
            .filter(measurements_schema::window_start.ge(since))
            .select((measurements_schema::window_start, measurements_schema::mean))
            .order(measurements_schema::window_start.asc())
            .limit(MAX_POINTS)
            .load(conn),
        Some(resolution) => rollups_schema::table
            .filter(rollups_schema::node_id.eq(node_id))
            .filter(rollups_schema::channel_id.eq(channel_id))
            .filter(rollups_schema::metric.eq(metric))
            .filter(rollups_schema::resolution.eq(resolution))
            .filter(rollups_schema::bucket_start.ge(since))
            .select((rollups_schema::bucket_start, rollups_schema::mean))
            .order(rollups_schema::bucket_start.asc())
            .limit(MAX_POINTS)
            .load(conn),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_span_reads_the_raw_windows() {
        for hours in [1_i64, 3, 6] {
            assert_eq!(
                Source::for_span(TimeDelta::hours(hours)),
                Source::Raw,
                "{hours} hours"
            );
        }
    }

    #[test]
    fn a_span_past_the_raw_retention_reads_rollups() {
        assert_eq!(Source::for_span(TimeDelta::hours(7)), Source::Hourly);
        assert_eq!(Source::for_span(TimeDelta::days(7)), Source::Hourly);
        assert_eq!(Source::for_span(TimeDelta::days(8)), Source::Daily);
        assert_eq!(Source::for_span(TimeDelta::days(90)), Source::Daily);
        assert_eq!(Source::for_span(TimeDelta::days(91)), Source::Weekly);
        assert_eq!(Source::for_span(TimeDelta::days(365)), Source::Weekly);
    }

    #[test]
    fn every_source_names_the_table_it_reads() {
        assert_eq!(Source::Raw.resolution(), None);
        assert_eq!(Source::Hourly.resolution(), Some(RollupResolution::Hourly));
        assert_eq!(Source::Daily.resolution(), Some(RollupResolution::Daily));
        assert_eq!(Source::Weekly.resolution(), Some(RollupResolution::Weekly));
    }

    #[test]
    fn a_missing_span_is_a_day() {
        let span = SpanQuery { hours: None };

        assert_eq!(span.duration(), TimeDelta::hours(DEFAULT_SPAN_HOURS));
    }

    #[test]
    fn a_span_is_clamped_at_both_ends() {
        assert_eq!(SpanQuery { hours: Some(0) }.duration(), TimeDelta::hours(1));
        assert_eq!(
            SpanQuery { hours: Some(-5) }.duration(),
            TimeDelta::hours(1)
        );
        assert_eq!(
            SpanQuery {
                hours: Some(1_000_000)
            }
            .duration(),
            TimeDelta::days(MAX_SPAN_DAYS)
        );
    }

    #[test]
    fn a_metric_label_round_trips() {
        assert_eq!(
            parse_metric("signal_to_noise").ok(),
            Some(Metric::SignalToNoise)
        );
        assert!(matches!(
            parse_metric("signal to noise"),
            Err(ApiError::UnprocessableEntity(_))
        ));
        assert!(matches!(
            parse_metric(""),
            Err(ApiError::UnprocessableEntity(_))
        ));
    }
}
