use std::collections::HashMap;

use askama::Template;
use axum::extract::{Path, Query, State};
use axum::{Router, response::Html, routing::get};
use chrono::{NaiveDateTime, Utc};
use diesel::dsl::count_star;
use diesel::prelude::*;

use super::auth::AuthPage;
use super::errors::ApiError;
use super::nodes::{self, SpanQuery};
use super::visibility;
use crate::{
    db::models::enums::{AlarmState, WorkOrderStatus},
    db::schema::{
        alarms as alarms_schema, nodes as nodes_schema, roles as roles_schema,
        users as users_schema, work_orders as orders_schema,
    },
    state::AppState,
    templates::{Chrome, IndexTemplate, NodeTile, stamp_or_empty},
};

///
const SILENCE_AFTER_SECONDS: i64 = 300;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(index))
        .route("/nodes/{id}", get(node))
}

async fn index(auth: AuthPage, State(state): State<AppState>) -> Result<Html<String>, ApiError> {
    dashboard(auth, state, None, SpanQuery::default()).await
}

///
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

///
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

    let html = IndexTemplate {
        nodes_total: chrome.nodes_total,
        silent: chrome.silent,
        open_alarms: chrome.open_alarms,
        open_orders: chrome.open_orders,
        user_name: chrome.user_name,
        user_role: chrome.user_role,
        nodes,
        panel,
        live: true,
    }
    .render()
    .map_err(ApiError::internal)?;

    Ok(Html(html))
}

///
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
                state: initial_state(suspended, last_seen_at, now),
                at: stamp_or_empty(last_seen_at),
                latitude,
                longitude,
                open_alarms: open.get(&id).copied().unwrap_or_default(),
            },
        )
        .collect();

    Ok((nodes, open))
}

///
/// # Errors
///
pub async fn chrome_for(state: &AppState, user_id: i64) -> Result<Chrome, ApiError> {
    let access = visibility::resolve(state, user_id).await?;
    let (nodes, open) = fleet(state, access.visibility.node_filter()).await?;

    chrome(state, user_id, &nodes, &open).await
}

///
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
        nodes_total: nodes.len(),
        silent: nodes.iter().filter(|node| node.state == "silent").count(),
        open_alarms: open_alarms.values().sum(),
        open_orders,
        user_name,
        user_role,
    })
}

const fn initial_state(
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
