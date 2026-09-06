use std::collections::{BTreeMap, BTreeSet};

use askama::Template;
use axum::extract::{Path, Query, State};
use axum::response::Html;
use axum::{Form, Json, Router, routing::get};
use chrono::{NaiveDateTime, TimeDelta, Utc};
use diesel::prelude::*;
use serde::Deserialize;

use super::{
    auth::{AuthPage, AuthUser},
    errors::{ApiError, ErrorBody},
    visibility::{self, Access},
};
use crate::{
    db::models::enums::{Metric, Modulation},
    db::models::nodes::Node,
    db::schema::{
        channels as channels_schema, measurements as measurements_schema,
        node_channels as node_channels_schema, node_health as health_schema, nodes as nodes_schema,
        users as users_schema,
    },
    state::AppState,
    templates::{
        ChannelView, HealthView, Meter, NodeEditForm, NodePanel, Owner, SpanChoice, stamp,
        stamp_or_empty,
    },
};

const SILENCE_AFTER_SECONDS: i64 = 300;

///
const CHARTED: [Metric; 3] = [
    Metric::SignalStrength,
    Metric::SignalToNoise,
    Metric::DemodErrorRate,
];

const TEMPERATURE_CEILING: f64 = 85.0;

const LOAD_CEILING: f64 = 4.0;

const CLOCK_CEILING_SECONDS: f64 = 2.0;

/// All routes under `/api/nodes`, plus the fragment the dashboard swaps.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/nodes", get(list_nodes))
        .route("/fragments/nodes/{id}", get(node_panel))
        .route("/fragments/nodes/{id}/edit", get(edit_form).post(edit))
}

async fn edit_form(
    auth: AuthPage,
    State(state): State<AppState>,
    Path(node_id): Path<i64>,
) -> Result<Html<String>, ApiError> {
    let access = visibility::resolve(&state, auth.0.session.user_id).await?;
    if !visibility::may_edit_node(&state, &access, node_id).await? {
        return Err(ApiError::Forbidden(
            "You may only edit a node you own.".into(),
        ));
    }

    let conn = state.pool.get().await?;
    let (name, latitude, longitude, owner_id): (String, Option<f64>, Option<f64>, Option<i64>) =
        conn.interact(move |conn| {
            nodes_schema::table
                .filter(nodes_schema::id.eq(node_id))
                .select((
                    nodes_schema::name,
                    nodes_schema::latitude,
                    nodes_schema::longitude,
                    nodes_schema::owner_id,
                ))
                .first(conn)
        })
        .await??;

    let may_assign_owner = access.is_admin();
    let owners = if may_assign_owner {
        candidates(&conn, owner_id).await?
    } else {
        Vec::new()
    };

    let html = NodeEditForm {
        id: node_id,
        name,
        latitude: latitude.map(|value| value.to_string()).unwrap_or_default(),
        longitude: longitude.map(|value| value.to_string()).unwrap_or_default(),
        owner_id,
        owners,
        may_assign_owner,
    }
    .render()
    .map_err(ApiError::internal)?;

    Ok(Html(html))
}

async fn edit(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(node_id): Path<i64>,
    Query(span): Query<SpanQuery>,
    Form(request): Form<crate::api::admin::NodeUpdate>,
) -> Result<Html<String>, ApiError> {
    let user_id = auth.session.user_id;

    drop(
        crate::api::admin::update_node(auth, State(state.clone()), Path(node_id), Json(request))
            .await?,
    );

    let access = visibility::resolve(&state, user_id).await?;

    Ok(Html(panel(&state, &access, node_id, span).await?))
}

async fn candidates(
    conn: &deadpool_diesel::postgres::Connection,
    owner_id: Option<i64>,
) -> Result<Vec<Owner>, ApiError> {
    let rows: Vec<(i64, String)> = conn
        .interact(move |conn| {
            users_schema::table
                .filter(users_schema::deactivated.eq(false))
                .select((users_schema::id, users_schema::full_name))
                .order(users_schema::full_name.asc())
                .load(conn)
        })
        .await??;

    Ok(rows
        .into_iter()
        .map(|(id, name)| Owner {
            selected: owner_id == Some(id),
            id,
            name,
        })
        .collect())
}

///
///
const SPANS: [(&str, &str, i64); 4] = [
    ("6h", "6t", 6),
    ("24h", "24t", 24),
    ("7d", "7d", 168),
    ("90d", "90d", 2160),
];

/// What the panel is asked to show.
///
#[derive(Debug, Default, Deserialize)]
pub struct SpanQuery {
    /// One of the keys in [`SPANS`]. Anything else falls back to the default.
    pub span: Option<String>,
    /// `all` to chart every metric the node reported rather than [`CHARTED`].
    pub metrics: Option<String>,
}

impl SpanQuery {
    fn key(&self) -> &str {
        let asked = self.span.as_deref().unwrap_or_default();

        SPANS
            .iter()
            .find(|(key, _, _)| *key == asked)
            .map_or(SPANS[0].0, |(key, _, _)| *key)
    }

    fn hours(&self) -> i64 {
        let key = self.key();

        SPANS
            .iter()
            .find(|(candidate, _, _)| *candidate == key)
            .map_or(SPANS[0].2, |(_, _, hours)| *hours)
    }

    fn show_all(&self) -> bool {
        self.metrics.as_deref() == Some("all")
    }
}

fn panel_link(node_id: i64, span: &str, show_all: bool) -> (String, String) {
    let mut query = Vec::new();
    if span != SPANS[0].0 {
        query.push(format!("span={span}"));
    }
    if show_all {
        query.push("metrics=all".to_owned());
    }

    let query = if query.is_empty() {
        String::new()
    } else {
        format!("?{}", query.join("&"))
    };

    (
        format!("/nodes/{node_id}{query}"),
        format!("/fragments/nodes/{node_id}{query}"),
    )
}

/// List the nodes this user may see.
///
/// # Errors
///
#[utoipa::path(
    get,
    path = "/api/nodes",
    responses(
        (status = 200, description = "Nodes visible to this user", body = Vec<Node>),
        (status = 401, description = "Not authenticated", body = ErrorBody),
    ),
    security(("session_token" = [])),
    tag = "nodes"
)]
pub async fn list_nodes(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<Node>>, ApiError> {
    let access = visibility::resolve(&state, auth.session.user_id).await?;
    let visible = access.visibility.node_filter();

    let conn = state.pool.get().await?;

    let all_nodes: Vec<Node> = conn
        .interact(move |conn| {
            let mut query = nodes_schema::table
                .select(Node::as_select())
                .order(nodes_schema::name.asc())
                .into_boxed();

            if let Some(ref allowed) = visible {
                query = query.filter(nodes_schema::id.eq_any(allowed.clone()));
            }

            query.load(conn)
        })
        .await??;

    Ok(Json(all_nodes))
}

///
/// # Errors
///
async fn node_panel(
    auth: AuthPage,
    State(state): State<AppState>,
    Path(node_id): Path<i64>,
    Query(span): Query<SpanQuery>,
) -> Result<Html<String>, ApiError> {
    let access = visibility::resolve(&state, auth.0.session.user_id).await?;

    Ok(Html(panel(&state, &access, node_id, span).await?))
}

/// Render one node's panel.
///
///
/// # Errors
///
pub async fn panel(
    state: &AppState,
    access: &Access,
    node_id: i64,
    span: SpanQuery,
) -> Result<String, ApiError> {
    if !access.visibility.allows(node_id) {
        return Err(ApiError::Forbidden(
            "This node is not one you are dispatched to.".into(),
        ));
    }

    let conn = state.pool.get().await?;

    let node: Node = conn
        .interact(move |conn| {
            nodes_schema::table
                .find(node_id)
                .select(Node::as_select())
                .first(conn)
        })
        .await??;

    let health: Option<(NaiveDateTime, f64, f64, f64, f64, f64, f64)> = conn
        .interact(move |conn| {
            health_schema::table
                .filter(health_schema::node_id.eq(node_id))
                .order(health_schema::measured_at.desc())
                .select((
                    health_schema::measured_at,
                    health_schema::uptime_seconds,
                    health_schema::load_1m,
                    health_schema::load_5m,
                    health_schema::load_15m,
                    health_schema::cpu_temperature_celsius,
                    health_schema::clock_offset_seconds,
                ))
                .first(conn)
                .optional()
        })
        .await??;

    let hours = span.hours();
    let show_all = span.show_all();
    let key = span.key().to_owned();
    let channels = channels(&conn, node_id, hours, show_all).await?;

    let may_edit = visibility::may_edit_node(state, access, node_id).await?;

    let now = Utc::now().naive_utc();
    let panel = NodePanel {
        id: node.id,
        name: node.name,
        enrolled: node.external_identity.is_some(),
        external_identity: node.external_identity.unwrap_or_default(),
        may_edit,
        state: state_of(node.suspended, node.last_seen_at, now),
        at: stamp_or_empty(node.last_seen_at),
        position: position(node.latitude, node.longitude),
        health: health.map(
            |(measured_at, uptime, one, five, fifteen, temperature, clock)| HealthView {
                measured_at: stamp(measured_at),
                uptime: uptime_words(uptime),
                meters: meters(&Sample {
                    load: [one, five, fifteen],
                    temperature,
                    clock,
                }),
            },
        ),
        channels,
        hours,
        spans: SPANS
            .iter()
            .map(|(candidate, label, _)| {
                let (url, fragment) = panel_link(node_id, candidate, show_all);

                SpanChoice {
                    label,
                    url,
                    fragment,
                    current: *candidate == key,
                }
            })
            .collect(),
        toggle: {
            let (url, fragment) = panel_link(node_id, &key, !show_all);

            SpanChoice {
                label: if show_all {
                    "Vis kun hovedmålinger"
                } else {
                    "Vis alle målinger"
                },
                url,
                fragment,
                current: show_all,
            }
        },
        show_all,
    };

    panel.render().map_err(ApiError::internal)
}

///
async fn channels(
    conn: &deadpool_diesel::postgres::Connection,
    node_id: i64,
    hours: i64,
    show_all: bool,
) -> Result<Vec<ChannelView>, ApiError> {
    let since = Utc::now()
        .naive_utc()
        .checked_sub_signed(TimeDelta::hours(hours))
        .unwrap_or_default();

    let assigned: Vec<i64> = conn
        .interact(move |conn| {
            node_channels_schema::table
                .filter(node_channels_schema::node_id.eq(node_id))
                .select(node_channels_schema::channel_id)
                .load(conn)
        })
        .await??;

    let measured: Vec<(i64, Metric)> = conn
        .interact(move |conn| {
            measurements_schema::table
                .filter(measurements_schema::node_id.eq(node_id))
                .filter(measurements_schema::window_start.ge(since))
                .select((measurements_schema::channel_id, measurements_schema::metric))
                .distinct()
                .load(conn)
        })
        .await??;

    let wanted: Vec<i64> = assigned
        .iter()
        .chain(measured.iter().map(|(channel_id, _)| channel_id))
        .copied()
        .collect::<BTreeSet<i64>>()
        .into_iter()
        .collect();

    let rows: Vec<(i64, String, i64, Modulation)> = conn
        .interact(move |conn| {
            channels_schema::table
                .filter(channels_schema::id.eq_any(wanted))
                .order(channels_schema::frequency_hz.asc())
                .select((
                    channels_schema::id,
                    channels_schema::name,
                    channels_schema::frequency_hz,
                    channels_schema::modulation,
                ))
                .load(conn)
        })
        .await??;

    let mut charted: BTreeMap<i64, BTreeSet<Metric>> = BTreeMap::new();
    for (channel_id, metric) in measured {
        charted.entry(channel_id).or_default().insert(metric);
    }

    Ok(rows
        .into_iter()
        .map(|(id, name, frequency_hz, modulation)| ChannelView {
            id,
            name,
            frequency: megahertz(frequency_hz),
            modulation: modulation.label(),
            metrics: charted.get(&id).map_or_else(
                || CHARTED.iter().map(|metric| metric.label()).collect(),
                |metrics| {
                    metrics
                        .iter()
                        .filter(|metric| show_all || CHARTED.contains(metric))
                        .map(|metric| metric.label())
                        .collect()
                },
            ),
        })
        .collect())
}

const fn state_of(
    suspended: bool,
    last_seen_at: Option<NaiveDateTime>,
    now: NaiveDateTime,
) -> &'static str {
    if suspended {
        return "suspended";
    }

    match last_seen_at {
        None => "never seen",
        Some(at) if now.signed_duration_since(at).num_seconds() > SILENCE_AFTER_SECONDS => "silent",
        Some(_) => "reporting",
    }
}

fn position(latitude: Option<f64>, longitude: Option<f64>) -> Option<String> {
    match (latitude, longitude) {
        (Some(latitude), Some(longitude)) => Some(format!("{latitude:.4}, {longitude:.4}")),
        _ => None,
    }
}

fn megahertz(frequency_hz: i64) -> String {
    format!("{:.3} MHz", millionths(frequency_hz))
}

///
fn millionths(value: i64) -> f64 {
    f64::from(i32::try_from(value.clamp(i64::from(i32::MIN), i64::from(i32::MAX))).unwrap_or(0))
        / 1_000_000.0
}

fn uptime_words(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0.0 {
        return "ukendt".to_owned();
    }

    let total = seconds / 60.0;
    let days = (total / (60.0 * 24.0)).floor();
    let hours = ((total / 60.0) % 24.0).floor();
    let minutes = (total % 60.0).floor();

    if days >= 1.0 {
        format!("{days:.0} d {hours:.0} t")
    } else if hours >= 1.0 {
        format!("{hours:.0} t {minutes:.0} m")
    } else {
        format!("{minutes:.0} m")
    }
}

struct Sample {
    load: [f64; 3],
    temperature: f64,
    clock: f64,
}

///
fn meters(sample: &Sample) -> Vec<Meter> {
    let Sample {
        load: [one, five, fifteen],
        temperature,
        clock,
    } = *sample;
    let sustained = five.max(fifteen);

    vec![
        Meter {
            label: "CPU-temperatur",
            value: format!("{temperature:.1} °C"),
            percent: fraction(temperature, TEMPERATURE_CEILING),
            tone: tone(temperature, TEMPERATURE_CEILING),
        },
        Meter {
            label: "Belastning 1m",
            value: format!("{one:.2}"),
            percent: fraction(one, LOAD_CEILING),
            tone: tone(one, LOAD_CEILING),
        },
        Meter {
            label: "Belastning 5m / 15m",
            value: format!("{five:.2} / {fifteen:.2}"),
            percent: fraction(sustained, LOAD_CEILING),
            tone: tone(sustained, LOAD_CEILING),
        },
        Meter {
            label: "Urafvigelse",
            value: format!("{clock:+.3} s"),
            percent: fraction(clock.abs(), CLOCK_CEILING_SECONDS),
            tone: tone(clock.abs(), CLOCK_CEILING_SECONDS),
        },
    ]
}

fn fraction(value: f64, ceiling: f64) -> f64 {
    if !value.is_finite() || ceiling <= 0.0 {
        return 0.0;
    }

    ((value / ceiling) * 100.0).clamp(0.0, 100.0)
}

fn tone(value: f64, ceiling: f64) -> &'static str {
    if !value.is_finite() {
        return "bad";
    }

    let share = value / ceiling;
    if share >= 1.0 {
        "bad"
    } else if share >= 0.75 {
        "warn"
    } else {
        "ok"
    }
}
