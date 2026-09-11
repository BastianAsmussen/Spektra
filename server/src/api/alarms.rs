use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::auth::AuthUser;
use super::errors::{ApiError, ErrorBody};
use super::visibility;
use crate::db::models::alarm_events::{AlarmEvent as AlarmEventRow, NewAlarmEvent};
use crate::db::models::alarms::Alarm;
use crate::db::models::enums::AlarmState;
use crate::db::schema::{alarm_events as events_schema, alarms as alarms_schema};
use crate::state::{AlarmEvent, AppState};

/// Most alarms one listing returns.
const PAGE_LIMIT: i64 = 200;

/// All routes under `/api/alarms`.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/alarms", get(list_alarms))
        .route("/api/alarms/{id}", get(get_alarm))
        .route("/api/alarms/{id}/transition", post(transition_alarm))
}

/// Filters accepted by the listing.
#[derive(Debug, Default, Deserialize, IntoParams)]
pub struct AlarmFilter {
    /// Only alarms in this state, by its stored label.
    pub state: Option<AlarmState>,
    /// Only alarms on this node.
    pub node_id: Option<i64>,
}

/// One alarm and the history behind it.
#[derive(Debug, Serialize, ToSchema)]
pub struct AlarmDetail {
    #[serde(flatten)]
    pub alarm: Alarm,
    /// Oldest first, so the list reads as the path the alarm took.
    pub events: Vec<AlarmEventRow>,
}

/// The move an operator or technician is asking for.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TransitionRequest {
    pub to: AlarmState,
    /// Why. Recorded verbatim on the event.
    pub reason: String,
}

/// Whether a move along the lifecycle is allowed.
#[must_use]
pub const fn may_transition(from: AlarmState, to: AlarmState) -> bool {
    matches!(
        (from, to),
        (AlarmState::Open, AlarmState::Acknowledged)
            | (AlarmState::Acknowledged, AlarmState::UnderVerification)
            | (
                AlarmState::Open | AlarmState::Acknowledged | AlarmState::UnderVerification,
                AlarmState::Closed,
            )
    )
}

/// List alarms, newest first.
///
/// # Errors
///
/// Returns [`ApiError`] for a missing session or a database failure.
#[utoipa::path(
    get,
    path = "/api/alarms",
    params(AlarmFilter),
    responses(
        (status = 200, description = "Alarms this user may see", body = Vec<Alarm>),
        (status = 401, description = "Not authenticated", body = ErrorBody),
    ),
    security(("session_token" = [])),
    tag = "alarms"
)]
pub async fn list_alarms(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(filter): Query<AlarmFilter>,
) -> Result<Json<Vec<Alarm>>, ApiError> {
    let access = visibility::resolve(&state, auth.session.user_id).await?;
    let conn = state.pool.get().await?;

    let alarms = conn
        .interact(move |conn| {
            let mut query = alarms_schema::table
                .select(Alarm::as_select())
                .order(alarms_schema::raised_at.desc())
                .limit(PAGE_LIMIT)
                .into_boxed();

            if let Some(wanted) = filter.state {
                query = query.filter(alarms_schema::state.eq(wanted));
            }
            if let Some(node_id) = filter.node_id {
                query = query.filter(alarms_schema::node_id.eq(node_id));
            }
            if let Some(allowed) = access.visibility.node_filter() {
                query = query.filter(alarms_schema::node_id.eq_any(allowed));
            }

            query.load(conn)
        })
        .await??;

    Ok(Json(alarms))
}

/// One alarm with its full history.
///
/// # Errors
///
/// Returns [`ApiError`] for a missing session, a forbidden node, or a missing alarm.
#[utoipa::path(
    get,
    path = "/api/alarms/{id}",
    params(("id" = i64, Path, description = "Alarm id")),
    responses(
        (status = 200, description = "The alarm and its events", body = AlarmDetail),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 403, description = "Not visible to this user", body = ErrorBody),
        (status = 404, description = "No such alarm", body = ErrorBody),
    ),
    security(("session_token" = [])),
    tag = "alarms"
)]
pub async fn get_alarm(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<AlarmDetail>, ApiError> {
    let access = visibility::resolve(&state, auth.session.user_id).await?;
    let conn = state.pool.get().await?;

    let detail = conn
        .interact(move |conn| {
            let alarm: Alarm = alarms_schema::table
                .filter(alarms_schema::id.eq(id))
                .select(Alarm::as_select())
                .first(conn)?;

            let events: Vec<AlarmEventRow> = events_schema::table
                .filter(events_schema::alarm_id.eq(id))
                .select(AlarmEventRow::as_select())
                .order(events_schema::created_at.asc())
                .load(conn)?;

            Ok::<_, diesel::result::Error>(AlarmDetail { alarm, events })
        })
        .await??;

    if !access.visibility.allows(detail.alarm.node_id) {
        return Err(ApiError::Forbidden(
            "This alarm is on a node you are not dispatched to.".into(),
        ));
    }

    Ok(Json(detail))
}

enum TransitionError {
    Forbidden,
    Illegal,
    Db(diesel::result::Error),
}

impl From<diesel::result::Error> for TransitionError {
    fn from(err: diesel::result::Error) -> Self {
        Self::Db(err)
    }
}

/// Move an alarm along its lifecycle.
///
/// # Errors
///
/// Returns [`ApiError`] if the move is forbidden, missing, or not allowed by the lifecycle.
#[utoipa::path(
    post,
    path = "/api/alarms/{id}/transition",
    params(("id" = i64, Path, description = "Alarm id")),
    request_body = TransitionRequest,
    responses(
        (status = 200, description = "The alarm after the move", body = AlarmDetail),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 403, description = "Not allowed to act on this alarm", body = ErrorBody),
        (status = 404, description = "No such alarm", body = ErrorBody),
        (status = 422, description = "Not a legal transition", body = ErrorBody),
    ),
    security(("session_token" = [])),
    tag = "alarms"
)]
pub async fn transition_alarm(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(request): Json<TransitionRequest>,
) -> Result<Json<AlarmDetail>, ApiError> {
    if request.reason.trim().is_empty() {
        return Err(ApiError::UnprocessableEntity(
            "A transition must carry a reason.".into(),
        ));
    }

    let access = visibility::resolve(&state, auth.session.user_id).await?;
    if !access.may_act() {
        return Err(ApiError::Forbidden(
            "Your role may read alarms but not change them.".into(),
        ));
    }

    let conn = state.pool.get().await?;
    let user_id = auth.session.user_id;
    let to = request.to;
    let reason = request.reason;
    let visibility = access.visibility.clone();

    let (node_id, from, detail) = conn
        .interact(move |conn| {
            conn.transaction(|conn| {
                let (node_id, from): (i64, AlarmState) = alarms_schema::table
                    .filter(alarms_schema::id.eq(id))
                    .select((alarms_schema::node_id, alarms_schema::state))
                    .for_update()
                    .first(conn)?;

                if !visibility.allows(node_id) {
                    return Err(TransitionError::Forbidden);
                }
                if !may_transition(from, to) {
                    return Err(TransitionError::Illegal);
                }

                let closed_at = (to == AlarmState::Closed).then(|| Utc::now().naive_utc());
                diesel::update(alarms_schema::table.filter(alarms_schema::id.eq(id)))
                    .set((
                        alarms_schema::state.eq(to),
                        alarms_schema::updated_at.eq(Utc::now().naive_utc()),
                        alarms_schema::closed_at.eq(closed_at),
                    ))
                    .execute(conn)?;

                diesel::insert_into(events_schema::table)
                    .values(&NewAlarmEvent {
                        alarm_id: id,
                        from_state: Some(from),
                        to_state: to,
                        changed_by_user_id: Some(user_id),
                        reason,
                    })
                    .execute(conn)?;

                let alarm: Alarm = alarms_schema::table
                    .filter(alarms_schema::id.eq(id))
                    .select(Alarm::as_select())
                    .first(conn)?;
                let events: Vec<AlarmEventRow> = events_schema::table
                    .filter(events_schema::alarm_id.eq(id))
                    .select(AlarmEventRow::as_select())
                    .order(events_schema::created_at.asc())
                    .load(conn)?;

                Ok((node_id, from, AlarmDetail { alarm, events }))
            })
        })
        .await?
        .map_err(|err| match err {
            TransitionError::Forbidden => {
                ApiError::Forbidden("This alarm is on a node you are not dispatched to.".into())
            }
            TransitionError::Illegal => ApiError::UnprocessableEntity(format!(
                "An alarm cannot move to '{to}' from where it is."
            )),
            TransitionError::Db(other) => ApiError::from(other),
        })?;

    state.publish_alarm(AlarmEvent::StateChanged {
        alarm_id: id,
        node_id,
        from,
        to,
    });

    Ok(Json(detail))
}

/// The states an alarm may move to from where it is.
#[must_use]
pub fn next_states(from: AlarmState) -> Vec<AlarmState> {
    [
        AlarmState::Open,
        AlarmState::Acknowledged,
        AlarmState::UnderVerification,
        AlarmState::Closed,
    ]
    .into_iter()
    .filter(|to| may_transition(from, *to))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lifecycle_runs_forwards() {
        assert!(may_transition(AlarmState::Open, AlarmState::Acknowledged));
        assert!(may_transition(
            AlarmState::Acknowledged,
            AlarmState::UnderVerification
        ));
        assert!(may_transition(
            AlarmState::UnderVerification,
            AlarmState::Closed
        ));
    }

    #[test]
    fn the_lifecycle_does_not_run_backwards() {
        assert!(!may_transition(AlarmState::Acknowledged, AlarmState::Open));
        assert!(!may_transition(
            AlarmState::UnderVerification,
            AlarmState::Acknowledged
        ));
    }

    #[test]
    fn a_step_may_not_be_skipped_except_to_closed() {
        assert!(!may_transition(
            AlarmState::Open,
            AlarmState::UnderVerification
        ));
        assert!(may_transition(AlarmState::Open, AlarmState::Closed));
        assert!(may_transition(AlarmState::Acknowledged, AlarmState::Closed));
    }

    #[test]
    fn a_closed_alarm_is_final() {
        for to in [
            AlarmState::Open,
            AlarmState::Acknowledged,
            AlarmState::UnderVerification,
            AlarmState::Closed,
        ] {
            assert!(
                !may_transition(AlarmState::Closed, to),
                "a closed alarm moved to {to}"
            );
        }
    }

    #[test]
    fn an_alarm_never_transitions_to_where_it_already_is() {
        for state in [
            AlarmState::Open,
            AlarmState::Acknowledged,
            AlarmState::UnderVerification,
            AlarmState::Closed,
        ] {
            assert!(!may_transition(state, state), "{state} moved to itself");
        }
    }

    #[test]
    fn the_offered_moves_match_the_rule() {
        assert_eq!(
            next_states(AlarmState::Open),
            vec![AlarmState::Acknowledged, AlarmState::Closed]
        );
        assert_eq!(
            next_states(AlarmState::Acknowledged),
            vec![AlarmState::UnderVerification, AlarmState::Closed]
        );
        assert_eq!(
            next_states(AlarmState::UnderVerification),
            vec![AlarmState::Closed]
        );
        assert!(next_states(AlarmState::Closed).is_empty());
    }
}

/// Dashboard fragments for the alarm lifecycle.
pub mod fragments {
    use askama::Template;
    use axum::Form;
    use axum::extract::{Path, State};
    use axum::response::Html;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use diesel::prelude::*;

    use super::{AlarmState, TransitionRequest, alarms_schema, next_states};
    use crate::api::auth::AuthPage;
    use crate::api::errors::ApiError;
    use crate::api::visibility::{self, Access};
    use crate::db::models::alarms::Alarm;
    use crate::db::models::enums::Metric;
    use crate::db::schema::nodes as nodes_schema;
    use crate::state::AppState;
    use crate::templates::{AlarmEntry, AlarmList, AlarmRow, Transition};

    /// Most alarms the feed draws on load.
    const FEED_LIMIT: i64 = 50;

    /// All fragment routes for alarms.
    pub fn routes() -> Router<AppState> {
        Router::new()
            .route("/fragments/alarms", get(feed))
            .route("/fragments/alarms/{id}", get(entry))
            .route("/fragments/alarms/{id}/transition", post(transition))
    }

    async fn feed(auth: AuthPage, State(state): State<AppState>) -> Result<Html<String>, ApiError> {
        let access = visibility::resolve(&state, auth.0.session.user_id).await?;
        let conn = state.pool.get().await?;
        let filter = access.visibility.node_filter();

        let rows: Vec<(Alarm, String)> = conn
            .interact(move |conn| {
                let mut query = alarms_schema::table
                    .inner_join(nodes_schema::table)
                    .select((Alarm::as_select(), nodes_schema::name))
                    .order(alarms_schema::raised_at.desc())
                    .limit(FEED_LIMIT)
                    .into_boxed();

                if let Some(allowed) = filter {
                    query = query.filter(alarms_schema::node_id.eq_any(allowed));
                }

                query.load(conn)
            })
            .await??;

        let alarms = rows
            .into_iter()
            .map(|(alarm, node_name)| row(&alarm, node_name, &access))
            .collect();

        let html = AlarmList { alarms }.render().map_err(ApiError::internal)?;

        Ok(Html(html))
    }

    async fn entry(
        auth: AuthPage,
        State(state): State<AppState>,
        Path(id): Path<i64>,
    ) -> Result<Html<String>, ApiError> {
        render(&state, auth.0.session.user_id, id).await
    }

    async fn transition(
        auth: AuthPage,
        State(state): State<AppState>,
        Path(id): Path<i64>,
        Form(request): Form<TransitionRequest>,
    ) -> Result<Html<String>, ApiError> {
        let user_id = auth.0.session.user_id;

        drop(super::transition_alarm(auth.0, State(state.clone()), Path(id), Json(request)).await?);

        render(&state, user_id, id).await
    }

    async fn render(state: &AppState, user_id: i64, id: i64) -> Result<Html<String>, ApiError> {
        let access = visibility::resolve(state, user_id).await?;
        let conn = state.pool.get().await?;

        let (alarm, node_name): (Alarm, String) = conn
            .interact(move |conn| {
                alarms_schema::table
                    .inner_join(nodes_schema::table)
                    .filter(alarms_schema::id.eq(id))
                    .select((Alarm::as_select(), nodes_schema::name))
                    .first(conn)
            })
            .await??;

        if !access.visibility.allows(alarm.node_id) {
            return Err(ApiError::Forbidden(
                "This alarm is on a node you are not dispatched to.".into(),
            ));
        }

        let alarm = row(&alarm, node_name, &access);
        let html = AlarmEntry { alarm }.render().map_err(ApiError::internal)?;

        Ok(Html(html))
    }

    /// Turn a stored alarm into what the template draws.
    pub fn row(alarm: &Alarm, node_name: String, access: &Access) -> AlarmRow {
        AlarmRow {
            id: alarm.id,
            node_id: alarm.node_id,
            node_name,
            metric: alarm.metric.map(Metric::label).unwrap_or_default(),
            state: alarm.state.label(),
            raised_at: crate::templates::stamp(alarm.raised_at),
            summary: summary(&alarm.explanation),
            actions: if access.may_act() {
                next_states(alarm.state)
                    .into_iter()
                    .map(|to| Transition {
                        to: to.label(),
                        label: danish(to),
                    })
                    .collect()
            } else {
                Vec::new()
            },
            may_dispatch: access.may_dispatch() && alarm.state != AlarmState::Closed,
        }
    }

    const fn danish(to: AlarmState) -> &'static str {
        match to {
            AlarmState::Open => "Genåbn",
            AlarmState::Acknowledged => "Kvittér",
            AlarmState::UnderVerification => "Send til verifikation",
            AlarmState::Closed => "Luk",
        }
    }

    fn summary(explanation: &serde_json::Value) -> String {
        let number = |pointer: &str| {
            explanation
                .pointer(pointer)
                .and_then(serde_json::Value::as_f64)
        };

        match explanation.get("kind").and_then(serde_json::Value::as_str) {
            Some("node_silence") => {
                let minutes = number("/threshold_minutes").unwrap_or_default();

                format!("Ingen målinger i over {minutes:.0} minutter.")
            }
            Some("baseline_deviation") => {
                let direction = match explanation
                    .get("direction")
                    .and_then(serde_json::Value::as_str)
                {
                    Some("below") => "under",
                    Some("above") => "over",
                    _ => "uden for",
                };

                match (number("/value"), number("/baseline/center")) {
                    (Some(value), Some(center)) => {
                        format!("Målt {value:.1}, {direction} et normalniveau på {center:.1}.")
                    }
                    _ => String::new(),
                }
            }
            _ => String::new(),
        }
    }
}
