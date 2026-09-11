use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::auth::AuthUser;
use super::errors::{ApiError, ErrorBody};
use super::visibility;
use crate::db::models::alarm_events::NewAlarmEvent;
use crate::db::models::enums::{AlarmState, WorkOrderStatus};
use crate::db::models::work_orders::{NewWorkOrder, WorkOrder};
use crate::db::schema::{
    alarm_events as events_schema, alarms as alarms_schema, work_orders as orders_schema,
};
use crate::state::{AlarmEvent, AppState};

/// Longest station name accepted.
const MAX_STATION_CHARS: usize = 200;

/// All routes under `/api/work-orders`, plus the dispatch that creates one.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/work-orders", get(list_work_orders))
        .route("/api/alarms/{id}/dispatch", post(dispatch))
        .route("/api/work-orders/{id}/complete", post(complete))
}

/// Who to send, and where.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DispatchRequest {
    /// The technician to assign.
    pub technician_user_id: i64,
    /// The station to visit, as an operator would name it on the phone.
    pub station_name: String,
}

/// What the technician found.
#[derive(Debug, Deserialize, ToSchema)]
pub struct FieldReport {
    /// Whether the fault was still there when they arrived.
    pub fault_present: bool,
    /// What caused it, in the technician's words.
    pub cause: Option<String>,
    /// What they did about it.
    pub action_taken: Option<String>,
}

/// A completed order and the alarm state it left behind.
#[derive(Debug, Serialize, ToSchema)]
pub struct CompletionResult {
    pub work_order: WorkOrder,
    /// The alarm's state after the report was filed.
    pub alarm_state: AlarmState,
}

/// List work orders this user may see.
///
/// # Errors
///
/// Returns [`ApiError`] for a missing session or a database failure.
#[utoipa::path(
    get,
    path = "/api/work-orders",
    responses(
        (status = 200, description = "Work orders this user may see", body = Vec<WorkOrder>),
        (status = 401, description = "Not authenticated", body = ErrorBody),
    ),
    security(("session_token" = [])),
    tag = "work orders"
)]
pub async fn list_work_orders(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<WorkOrder>>, ApiError> {
    let access = visibility::resolve(&state, auth.session.user_id).await?;
    let user_id = auth.session.user_id;
    let conn = state.pool.get().await?;

    let orders = conn
        .interact(move |conn| {
            let mut query = orders_schema::table
                .select(WorkOrder::as_select())
                .order(orders_schema::created_at.desc())
                .into_boxed();

            if access.sees_only_own_orders() {
                query = query.filter(orders_schema::technician_user_id.eq(user_id));
            }

            query.load(conn)
        })
        .await??;

    Ok(Json(orders))
}

/// Dispatch an alarm to a technician.
///
/// # Errors
///
/// Returns [`ApiError`] if the caller cannot dispatch, the alarm is missing, or the station name is invalid.
#[utoipa::path(
    post,
    path = "/api/alarms/{id}/dispatch",
    params(("id" = i64, Path, description = "Alarm id")),
    request_body = DispatchRequest,
    responses(
        (status = 200, description = "The created work order", body = WorkOrder),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 403, description = "Not allowed to dispatch", body = ErrorBody),
        (status = 404, description = "No such alarm", body = ErrorBody),
        (status = 422, description = "The alarm cannot be dispatched", body = ErrorBody),
    ),
    security(("session_token" = [])),
    tag = "work orders"
)]
pub async fn dispatch(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(alarm_id): Path<i64>,
    Json(request): Json<DispatchRequest>,
) -> Result<Json<WorkOrder>, ApiError> {
    let station = request.station_name.trim().to_owned();
    if station.is_empty() {
        return Err(ApiError::UnprocessableEntity(
            "A work order must name the station to visit.".into(),
        ));
    }
    if station.chars().count() > MAX_STATION_CHARS {
        return Err(ApiError::UnprocessableEntity(format!(
            "A station name must not exceed {MAX_STATION_CHARS} characters."
        )));
    }

    let access = visibility::resolve(&state, auth.session.user_id).await?;
    if !access.may_dispatch() {
        return Err(ApiError::Forbidden(
            "Only an operator or an administrator may dispatch a work order.".into(),
        ));
    }

    let user_id = auth.session.user_id;
    let technician_user_id = request.technician_user_id;
    let conn = state.pool.get().await?;
    let (order, node_id, from) = conn
        .interact(move |conn| {
            conn.transaction(|conn| {
                let (node_id, from): (i64, AlarmState) = alarms_schema::table
                    .filter(alarms_schema::id.eq(alarm_id))
                    .select((alarms_schema::node_id, alarms_schema::state))
                    .for_update()
                    .first(conn)?;

                if from == AlarmState::Closed {
                    return Err(diesel::result::Error::RollbackTransaction);
                }

                let order: WorkOrder = diesel::insert_into(orders_schema::table)
                    .values(&NewWorkOrder {
                        alarm_id,
                        technician_user_id,
                        station_name: station,
                    })
                    .returning(WorkOrder::as_returning())
                    .get_result(conn)?;

                if from != AlarmState::UnderVerification {
                    diesel::update(alarms_schema::table.filter(alarms_schema::id.eq(alarm_id)))
                        .set((
                            alarms_schema::state.eq(AlarmState::UnderVerification),
                            alarms_schema::updated_at.eq(Utc::now().naive_utc()),
                        ))
                        .execute(conn)?;

                    diesel::insert_into(events_schema::table)
                        .values(&NewAlarmEvent {
                            alarm_id,
                            from_state: Some(from),
                            to_state: AlarmState::UnderVerification,
                            changed_by_user_id: Some(user_id),
                            reason: format!(
                                "dispatched to technician {technician_user_id} at {}",
                                order.station_name
                            ),
                        })
                        .execute(conn)?;
                }

                Ok((order, node_id, from))
            })
        })
        .await?
        .map_err(|err| match err {
            diesel::result::Error::RollbackTransaction => {
                ApiError::UnprocessableEntity("A closed alarm cannot be dispatched.".into())
            }
            other => ApiError::from(other),
        })?;

    if from != AlarmState::UnderVerification {
        state.publish_alarm(AlarmEvent::StateChanged {
            alarm_id,
            node_id,
            from,
            to: AlarmState::UnderVerification,
        });
    }

    Ok(Json(order))
}

/// File a field report and close the alarm behind it.
///
/// # Errors
///
/// Returns [`ApiError`] if the caller cannot complete this order or it is already completed.
#[utoipa::path(
    post,
    path = "/api/work-orders/{id}/complete",
    params(("id" = i64, Path, description = "Work order id")),
    request_body = FieldReport,
    responses(
        (status = 200, description = "The completed order and the alarm's state", body = CompletionResult),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 403, description = "Not this technician's order", body = ErrorBody),
        (status = 404, description = "No such work order", body = ErrorBody),
        (status = 422, description = "Already completed", body = ErrorBody),
    ),
    security(("session_token" = [])),
    tag = "work orders"
)]
pub async fn complete(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(order_id): Path<i64>,
    Json(report): Json<FieldReport>,
) -> Result<Json<CompletionResult>, ApiError> {
    let access = visibility::resolve(&state, auth.session.user_id).await?;
    if !access.may_act() {
        return Err(ApiError::Forbidden(
            "Your role may read work orders but not complete them.".into(),
        ));
    }

    let user_id = auth.session.user_id;
    let conn = state.pool.get().await?;

    let assignee: i64 = conn
        .interact(move |conn| {
            orders_schema::table
                .filter(orders_schema::id.eq(order_id))
                .select(orders_schema::technician_user_id)
                .first(conn)
        })
        .await??;

    if !access.may_complete(assignee) {
        return Err(ApiError::Forbidden(
            "This work order is assigned to another technician.".into(),
        ));
    }

    let reason = describe(&report);
    let (order, alarm_id, node_id, from, alarm_state) = conn
        .interact(move |conn| {
            conn.transaction(|conn| {
                let (alarm_id, status): (i64, WorkOrderStatus) = orders_schema::table
                    .filter(orders_schema::id.eq(order_id))
                    .select((orders_schema::alarm_id, orders_schema::status))
                    .for_update()
                    .first(conn)?;

                if status == WorkOrderStatus::Completed {
                    return Err(diesel::result::Error::RollbackTransaction);
                }

                let order: WorkOrder =
                    diesel::update(orders_schema::table.filter(orders_schema::id.eq(order_id)))
                        .set((
                            orders_schema::status.eq(WorkOrderStatus::Completed),
                            orders_schema::fault_present.eq(Some(report.fault_present)),
                            orders_schema::cause.eq(report.cause),
                            orders_schema::action_taken.eq(report.action_taken),
                            orders_schema::completed_at.eq(Some(Utc::now().naive_utc())),
                        ))
                        .returning(WorkOrder::as_returning())
                        .get_result(conn)?;

                let (node_id, from): (i64, AlarmState) = alarms_schema::table
                    .filter(alarms_schema::id.eq(alarm_id))
                    .select((alarms_schema::node_id, alarms_schema::state))
                    .for_update()
                    .first(conn)?;

                let alarm_state = if from == AlarmState::Closed {
                    from
                } else {
                    diesel::update(alarms_schema::table.filter(alarms_schema::id.eq(alarm_id)))
                        .set((
                            alarms_schema::state.eq(AlarmState::Closed),
                            alarms_schema::updated_at.eq(Utc::now().naive_utc()),
                            alarms_schema::closed_at.eq(Some(Utc::now().naive_utc())),
                        ))
                        .execute(conn)?;

                    diesel::insert_into(events_schema::table)
                        .values(&NewAlarmEvent {
                            alarm_id,
                            from_state: Some(from),
                            to_state: AlarmState::Closed,
                            changed_by_user_id: Some(user_id),
                            reason,
                        })
                        .execute(conn)?;

                    AlarmState::Closed
                };

                Ok((order, alarm_id, node_id, from, alarm_state))
            })
        })
        .await?
        .map_err(|err| match err {
            diesel::result::Error::RollbackTransaction => {
                ApiError::UnprocessableEntity("This work order is already completed.".into())
            }
            other => ApiError::from(other),
        })?;

    if from != AlarmState::Closed {
        state.publish_alarm(AlarmEvent::StateChanged {
            alarm_id,
            node_id,
            from,
            to: AlarmState::Closed,
        });
    }

    Ok(Json(CompletionResult {
        work_order: order,
        alarm_state,
    }))
}

fn describe(report: &FieldReport) -> String {
    let verdict = if report.fault_present {
        "fault confirmed on site"
    } else {
        "no fault found on site"
    };

    match (report.cause.as_deref(), report.action_taken.as_deref()) {
        (Some(cause), Some(action)) => format!("{verdict}: {cause}; {action}"),
        (Some(cause), None) => format!("{verdict}: {cause}"),
        (None, Some(action)) => format!("{verdict}; {action}"),
        (None, None) => verdict.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(fault: bool, cause: Option<&str>, action: Option<&str>) -> FieldReport {
        FieldReport {
            fault_present: fault,
            cause: cause.map(ToOwned::to_owned),
            action_taken: action.map(ToOwned::to_owned),
        }
    }

    #[test]
    fn a_confirmed_fault_says_so() {
        assert_eq!(
            describe(&report(true, None, None)),
            "fault confirmed on site"
        );
    }

    #[test]
    fn a_clear_visit_says_so() {
        assert_eq!(
            describe(&report(false, None, None)),
            "no fault found on site"
        );
    }

    #[test]
    fn the_cause_and_the_action_both_reach_the_event() {
        assert_eq!(
            describe(&report(
                true,
                Some("water in the feeder"),
                Some("replaced it")
            )),
            "fault confirmed on site: water in the feeder; replaced it"
        );
        assert_eq!(
            describe(&report(true, Some("water in the feeder"), None)),
            "fault confirmed on site: water in the feeder"
        );
        assert_eq!(
            describe(&report(false, None, Some("retightened the connector"))),
            "no fault found on site; retightened the connector"
        );
    }
}

/// Dashboard fragments for dispatch and field reporting.
pub mod fragments {
    use askama::Template;
    use axum::Form;
    use axum::extract::{Path, State};
    use axum::response::{Html, IntoResponse as _, Response};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use diesel::prelude::*;

    use super::{DispatchRequest, FieldReport, WorkOrderStatus, orders_schema};
    use crate::api::auth::AuthPage;
    use crate::api::errors::ApiError;
    use crate::api::visibility::{self, Access, TECHNICIAN};
    use crate::db::models::work_orders::WorkOrder;
    use crate::db::schema::{
        alarms as alarms_schema, nodes as nodes_schema, roles as roles_schema,
        users as users_schema,
    };
    use crate::state::AppState;
    use crate::templates::{DispatchForm, Technician, WorkOrderEntry, WorkOrderList, WorkOrderRow};

    /// All fragment routes for work orders.
    pub fn routes() -> Router<AppState> {
        Router::new()
            .route("/fragments/work-orders", get(list))
            .route("/fragments/alarms/{id}/dispatch", get(form).post(send))
            .route("/fragments/work-orders/{id}/complete", post(complete))
    }

    type OrderRow = (WorkOrder, String, String);

    async fn list(auth: AuthPage, State(state): State<AppState>) -> Result<Html<String>, ApiError> {
        let user_id = auth.0.session.user_id;
        let orders = load(&state, user_id, None).await?;
        let html = WorkOrderList { orders }
            .render()
            .map_err(ApiError::internal)?;

        Ok(Html(html))
    }

    async fn form(
        auth: AuthPage,
        State(state): State<AppState>,
        Path(alarm_id): Path<i64>,
    ) -> Result<Html<String>, ApiError> {
        let access = visibility::resolve(&state, auth.0.session.user_id).await?;
        if !access.may_dispatch() {
            return Err(ApiError::Forbidden(
                "Only an operator or an administrator may dispatch a work order.".into(),
            ));
        }

        let conn = state.pool.get().await?;

        let station_name: String = conn
            .interact(move |conn| {
                alarms_schema::table
                    .inner_join(nodes_schema::table)
                    .filter(alarms_schema::id.eq(alarm_id))
                    .select(nodes_schema::name)
                    .first(conn)
            })
            .await??;

        let technicians: Vec<Technician> = conn
            .interact(|conn| {
                users_schema::table
                    .inner_join(roles_schema::table)
                    .filter(roles_schema::name.eq(TECHNICIAN))
                    .order(users_schema::full_name.asc())
                    .select((users_schema::id, users_schema::full_name))
                    .load::<(i64, String)>(conn)
            })
            .await??
            .into_iter()
            .map(|(id, name)| Technician { id, name })
            .collect();

        let html = DispatchForm {
            alarm_id,
            technicians,
            station_name,
        }
        .render()
        .map_err(ApiError::internal)?;

        Ok(Html(html))
    }

    const ORDERS_CHANGED: &str = "spektra:orders";

    async fn send(
        auth: AuthPage,
        State(state): State<AppState>,
        Path(alarm_id): Path<i64>,
        Form(request): Form<DispatchRequest>,
    ) -> Result<Response, ApiError> {
        let user_id = auth.0.session.user_id;

        drop(super::dispatch(auth.0, State(state.clone()), Path(alarm_id), Json(request)).await?);

        let access = visibility::resolve(&state, user_id).await?;
        let conn = state.pool.get().await?;
        let (alarm, node_name) = conn
            .interact(move |conn| {
                alarms_schema::table
                    .inner_join(nodes_schema::table)
                    .filter(alarms_schema::id.eq(alarm_id))
                    .select((
                        crate::db::models::alarms::Alarm::as_select(),
                        nodes_schema::name,
                    ))
                    .first::<(crate::db::models::alarms::Alarm, String)>(conn)
            })
            .await??;

        let alarm = crate::api::alarms::fragments::row(&alarm, node_name, &access);
        let html = crate::templates::AlarmEntry { alarm }
            .render()
            .map_err(ApiError::internal)?;

        Ok(([("HX-Trigger", ORDERS_CHANGED)], Html(html)).into_response())
    }

    async fn complete(
        auth: AuthPage,
        State(state): State<AppState>,
        Path(order_id): Path<i64>,
        Form(report): Form<FieldReport>,
    ) -> Result<Html<String>, ApiError> {
        let user_id = auth.0.session.user_id;

        drop(super::complete(auth.0, State(state.clone()), Path(order_id), Json(report)).await?);

        let mut orders = load(&state, user_id, Some(order_id)).await?;
        let Some(order) = orders.pop() else {
            return Err(ApiError::NotFound("No such work order.".into()));
        };

        let html = WorkOrderEntry { order }
            .render()
            .map_err(ApiError::internal)?;

        Ok(Html(html))
    }

    async fn load(
        state: &AppState,
        user_id: i64,
        only: Option<i64>,
    ) -> Result<Vec<WorkOrderRow>, ApiError> {
        let access = visibility::resolve(state, user_id).await?;
        let conn = state.pool.get().await?;
        let scoped = access.sees_only_own_orders();

        let rows: Vec<OrderRow> = conn
            .interact(move |conn| {
                let mut query = orders_schema::table
                    .inner_join(alarms_schema::table)
                    .inner_join(nodes_schema::table.on(nodes_schema::id.eq(alarms_schema::node_id)))
                    .inner_join(
                        users_schema::table
                            .on(users_schema::id.eq(orders_schema::technician_user_id)),
                    )
                    .select((
                        WorkOrder::as_select(),
                        nodes_schema::name,
                        users_schema::full_name,
                    ))
                    .order(orders_schema::created_at.desc())
                    .into_boxed();

                if scoped {
                    query = query.filter(orders_schema::technician_user_id.eq(user_id));
                }
                if let Some(id) = only {
                    query = query.filter(orders_schema::id.eq(id));
                }

                query.load(conn)
            })
            .await??;

        Ok(rows
            .into_iter()
            .map(|(order, node_name, technician)| row(order, node_name, technician, &access))
            .collect())
    }

    fn row(
        order: WorkOrder,
        node_name: String,
        technician: String,
        access: &Access,
    ) -> WorkOrderRow {
        WorkOrderRow {
            id: order.id,
            alarm_id: order.alarm_id,
            node_name,
            station_name: order.station_name,
            technician,
            status: order.status.label(),
            dispatched_at: crate::templates::stamp(order.created_at),
            completed_at: crate::templates::stamp_or_empty(order.completed_at),
            fault_present: order.fault_present,
            cause: order.cause.unwrap_or_default(),
            action_taken: order.action_taken.unwrap_or_default(),
            may_complete: order.status != WorkOrderStatus::Completed
                && access.may_complete(order.technician_user_id),
        }
    }
}
