use std::collections::HashMap;

use askama::Template;
use axum::extract::{Path, Query, State};
use axum::{Router, response::Html, routing::get};
use chrono::{NaiveDateTime, Utc};
use diesel::dsl::count_star;
use diesel::prelude::*;
use serde::Deserialize;

use super::auth::AuthPage;
use super::errors::ApiError;
use super::nodes::{self, SpanQuery};
use super::nodes::{SILENCE_AFTER_SECONDS, state_of};
use super::visibility;
use crate::{
    db::models::enums::{AlarmState, WorkOrderStatus},
    db::schema::{
        alarms as alarms_schema, nodes as nodes_schema, roles as roles_schema,
        users as users_schema, work_orders as orders_schema,
    },
    state::AppState,
    templates::{Chrome, ChromeCounts, FleetPage, IndexTemplate, NodeTile, stamp_or_empty},
};

/// Tiles rendered per page of the fleet list.
pub const FLEET_PAGE: usize = 200;

/// All page routes, plus the fleet list the dashboard pages through.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(index))
        .route("/nodes/{id}", get(node))
        .route("/fragments/fleet", get(fleet_page))
        .route("/fragments/chrome", get(chrome_counts))
}

/// What the fleet list is asked for.
#[derive(Debug, Default, Deserialize)]
pub struct FleetQuery {
    pub q: Option<String>,
    pub state: Option<String>,
    pub offset: Option<usize>,
}

impl FleetQuery {
    fn at(&self, offset: usize) -> String {
        let mut parts = vec![format!("offset={offset}")];
        if let Some(term) = self.q.as_deref().filter(|term| !term.is_empty()) {
            parts.push(format!("q={}", urlencode(term)));
        }
        if let Some(state) = self.state.as_deref().filter(|state| !state.is_empty()) {
            parts.push(format!("state={}", urlencode(state)));
        }

        format!("?{}", parts.join("&"))
    }

    fn matches(&self, node: &NodeTile) -> bool {
        let term = self.q.as_deref().unwrap_or_default().trim().to_lowercase();
        let wanted = self.state.as_deref().unwrap_or_default();

        (term.is_empty() || node.name.to_lowercase().contains(&term))
            && (wanted.is_empty() || node.state == wanted)
    }
}

fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(byte).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

async fn fleet_page(
    auth: AuthPage,
    State(state): State<AppState>,
    Query(query): Query<FleetQuery>,
) -> Result<Html<String>, ApiError> {
    let access = visibility::resolve(&state, auth.0.session.user_id).await?;
    let (nodes, _) = fleet(&state, access.visibility.node_filter()).await?;

    let matching: Vec<NodeTile> = nodes
        .into_iter()
        .filter(|node| query.matches(node))
        .collect();

    let offset = query.offset.unwrap_or_default().min(matching.len());
    let end = offset.saturating_add(FLEET_PAGE).min(matching.len());
    let next = if end < matching.len() {
        query.at(end)
    } else {
        String::new()
    };

    let html = FleetPage {
        nodes: matching.into_iter().skip(offset).take(FLEET_PAGE).collect(),
        next,
        first: offset == 0,
    }
    .render()
    .map_err(ApiError::internal)?;

    Ok(Html(html))
}

async fn index(auth: AuthPage, State(state): State<AppState>) -> Result<Html<String>, ApiError> {
    dashboard(auth, state, None, SpanQuery::default()).await
}

async fn node(
    auth: AuthPage,
    State(state): State<AppState>,
    Path(node_id): Path<i64>,
    Query(span): Query<SpanQuery>,
) -> Result<Html<String>, ApiError> {
    dashboard(auth, state, Some(node_id), span).await
}

type FleetRow = (
    i64,
    String,
    bool,
    Option<NaiveDateTime>,
    Option<f64>,
    Option<f64>,
);

async fn dashboard(
    auth: AuthPage,
    state: AppState,
    open_node: Option<i64>,
    span: SpanQuery,
) -> Result<Html<String>, ApiError> {
    let user_id = auth.0.session.user_id;
    let access = visibility::resolve(&state, user_id).await?;
    let visible = access.visibility.node_filter();

    let panel = match open_node {
        Some(node_id) => nodes::panel(&state, &access, node_id, span).await?,
        None => String::new(),
    };

    let (nodes, open) = fleet(&state, visible).await?;
    let chrome = chrome(&state, user_id, &nodes, &open).await?;

    let shown = nodes.len().min(FLEET_PAGE);
    let next = if nodes.len() > shown {
        format!("?offset={shown}")
    } else {
        String::new()
    };

    let html = IndexTemplate {
        shown,
        next,
        chrome,
        nodes,
        panel,
        live: true,
    }
    .render()
    .map_err(ApiError::internal)?;

    Ok(Html(html))
}

async fn fleet(
    state: &AppState,
    visible: Option<Vec<i64>>,
) -> Result<(Vec<NodeTile>, HashMap<i64, i64>), ApiError> {
    let conn = state.pool.get().await?;

    let rows: Vec<FleetRow> = {
        let visible = visible.clone();

        conn.interact(move |conn| {
            let mut query = nodes_schema::table
                .select((
                    nodes_schema::id,
                    nodes_schema::name,
                    nodes_schema::suspended,
                    nodes_schema::last_seen_at,
                    nodes_schema::latitude,
                    nodes_schema::longitude,
                ))
                .order(nodes_schema::name.asc())
                .into_boxed();

            if let Some(ref allowed) = visible {
                query = query.filter(nodes_schema::id.eq_any(allowed.clone()));
            }

            query.load(conn)
        })
        .await??
    };

    let open: HashMap<i64, i64> = conn
        .interact(move |conn| {
            let mut query = alarms_schema::table
                .filter(alarms_schema::state.ne(AlarmState::Closed))
                .group_by(alarms_schema::node_id)
                .select((alarms_schema::node_id, count_star()))
                .into_boxed();

            if let Some(ref allowed) = visible {
                query = query.filter(alarms_schema::node_id.eq_any(allowed.clone()));
            }

            query.load::<(i64, i64)>(conn)
        })
        .await??
        .into_iter()
        .collect();

    let now = Utc::now().naive_utc();
    let nodes = rows
        .into_iter()
        .map(
            |(id, name, suspended, last_seen_at, latitude, longitude)| NodeTile {
                id,
                name,
                state: state_of(suspended, last_seen_at, now),
                at: stamp_or_empty(last_seen_at),
                latitude,
                longitude,
                open_alarms: open.get(&id).copied().unwrap_or_default(),
            },
        )
        .collect();

    Ok((nodes, open))
}

/// Header counters for a page that is not the dashboard.
///
/// # Errors
///
/// Returns [`ApiError`] if the session points at a missing user or the database fails.
pub async fn chrome_for(state: &AppState, user_id: i64) -> Result<Chrome, ApiError> {
    let access = visibility::resolve(state, user_id).await?;
    let counts = counts(state, &access).await?;

    let conn = state.pool.get().await?;
    let (user_name, user_role): (String, String) = conn
        .interact(move |conn| {
            users_schema::table
                .inner_join(roles_schema::table)
                .filter(users_schema::id.eq(user_id))
                .select((users_schema::full_name, roles_schema::name))
                .first(conn)
        })
        .await??;

    Ok(Chrome {
        nodes_total: counts.nodes_total,
        silent: counts.silent,
        open_alarms: counts.open_alarms,
        open_orders: counts.open_orders,
        user_name,
        user_role,
    })
}

async fn counts(state: &AppState, access: &visibility::Access) -> Result<ChromeCounts, ApiError> {
    let visible = access.visibility.node_filter();
    let conn = state.pool.get().await?;

    let cutoff = Utc::now()
        .naive_utc()
        .checked_sub_signed(chrono::TimeDelta::seconds(SILENCE_AFTER_SECONDS))
        .unwrap_or_else(|| Utc::now().naive_utc());

    let (nodes_total, silent, open_alarms, open_orders) = conn
        .interact(move |conn| {
            let mut total = nodes_schema::table.into_boxed();
            let mut quiet = nodes_schema::table
                .filter(nodes_schema::suspended.eq(false))
                .filter(nodes_schema::last_seen_at.lt(cutoff))
                .into_boxed();
            let mut alarms = alarms_schema::table
                .filter(alarms_schema::state.ne(AlarmState::Closed))
                .into_boxed();
            let mut orders = orders_schema::table
                .inner_join(alarms_schema::table)
                .filter(orders_schema::status.eq(WorkOrderStatus::Assigned))
                .into_boxed();

            if let Some(ref ids) = visible {
                total = total.filter(nodes_schema::id.eq_any(ids.clone()));
                quiet = quiet.filter(nodes_schema::id.eq_any(ids.clone()));
                alarms = alarms.filter(alarms_schema::node_id.eq_any(ids.clone()));
                orders = orders.filter(alarms_schema::node_id.eq_any(ids.clone()));
            }

            Ok::<_, diesel::result::Error>((
                total.count().get_result::<i64>(conn)?,
                quiet.count().get_result::<i64>(conn)?,
                alarms.count().get_result::<i64>(conn)?,
                orders.count().get_result::<i64>(conn)?,
            ))
        })
        .await??;

    Ok(ChromeCounts {
        nodes_total,
        silent,
        open_alarms,
        open_orders,
    })
}

async fn chrome_counts(
    auth: AuthPage,
    State(state): State<AppState>,
) -> Result<Html<String>, ApiError> {
    let access = visibility::resolve(&state, auth.0.session.user_id).await?;
    let html = counts(&state, &access)
        .await?
        .render()
        .map_err(ApiError::internal)?;

    Ok(Html(html))
}

async fn chrome(
    state: &AppState,
    user_id: i64,
    nodes: &[NodeTile],
    open_alarms: &HashMap<i64, i64>,
) -> Result<Chrome, ApiError> {
    let conn = state.pool.get().await?;
    let (user_name, user_role): (String, String) = conn
        .interact(move |conn| {
            users_schema::table
                .inner_join(roles_schema::table)
                .filter(users_schema::id.eq(user_id))
                .select((users_schema::full_name, roles_schema::name))
                .first(conn)
        })
        .await??;

    let visible: Vec<i64> = nodes.iter().map(|node| node.id).collect();
    let open_orders: i64 = conn
        .interact(move |conn| {
            orders_schema::table
                .inner_join(alarms_schema::table)
                .filter(orders_schema::status.eq(WorkOrderStatus::Assigned))
                .filter(alarms_schema::node_id.eq_any(visible))
                .count()
                .get_result(conn)
        })
        .await??;

    Ok(Chrome {
        nodes_total: i64::try_from(nodes.len()).unwrap_or(i64::MAX),
        silent: i64::try_from(nodes.iter().filter(|node| node.state == "silent").count())
            .unwrap_or(i64::MAX),
        open_alarms: open_alarms.values().sum(),
        open_orders,
        user_name,
        user_role,
    })
}
