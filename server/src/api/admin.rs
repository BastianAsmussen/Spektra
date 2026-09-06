use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::auth::{AuthUser, hash_password};
use super::errors::{ApiError, ErrorBody};
use super::visibility::{self, Access};
use crate::db::models::node_credentials::NewNodeCredential;
use crate::db::models::nodes::{Node, PlannedNode};
use crate::db::schema::{
    node_credentials as credentials_schema, nodes as nodes_schema, roles as roles_schema,
    users as users_schema,
};
use crate::state::{AppState, NodeEvent};

const MAX_NAME_CHARS: usize = 100;

const MAX_EMAIL_CHARS: usize = 320;

const MIN_PASSWORD_CHARS: usize = 12;

/// All routes under `/api/users`, plus the node suspension the fleet needs.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/users", get(list_users).post(create_user))
        .route("/api/users/{id}", post(update_user))
        .route("/api/nodes", post(plan_node))
        .route("/api/nodes/{id}", post(update_node))
        .route("/api/nodes/{id}/credential", post(rotate_credential))
        .route("/api/nodes/{id}/suspension", post(set_suspension))
}

/// One account as the administration view lists it.
#[derive(Debug, Serialize, ToSchema)]
pub struct UserSummary {
    pub id: i64,
    pub email: String,
    pub full_name: String,
    /// The role's name, not its id: the id is a migration detail.
    pub role: String,
    pub deactivated: bool,
}

/// A new account.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NewUserRequest {
    pub email: String,
    pub full_name: String,
    /// The role's name, as `roles.name` spells it.
    pub role: String,
    pub password: String,
}

/// A change to an existing account. Both fields optional, applied together.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct UserUpdate {
    pub role: Option<String>,
    pub deactivated: Option<bool>,
}

/// Whether a node is in the fleet or out of it. Also the response.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct SuspensionRequest {
    pub suspended: bool,
}

/// Reject anybody who is not an administrator.
async fn require_admin(state: &AppState, user_id: i64) -> Result<Access, ApiError> {
    let access = visibility::resolve(state, user_id).await?;
    if !access.is_admin() {
        return Err(ApiError::Forbidden(
            "Only an administrator may administer users and nodes.".into(),
        ));
    }

    Ok(access)
}

/// Every account, oldest first.
///
/// # Errors
///
#[utoipa::path(
    get,
    path = "/api/users",
    responses(
        (status = 200, description = "Every account", body = Vec<UserSummary>),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 403, description = "Not an administrator", body = ErrorBody),
    ),
    security(("session_token" = [])),
    tag = "administration"
)]
pub async fn list_users(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<UserSummary>>, ApiError> {
    drop(require_admin(&state, auth.session.user_id).await?);

    Ok(Json(all_users(&state).await?))
}

async fn all_users(state: &AppState) -> Result<Vec<UserSummary>, ApiError> {
    let conn = state.pool.get().await?;
    let rows: Vec<(i64, String, String, String, bool)> = conn
        .interact(move |conn| {
            users_schema::table
                .inner_join(roles_schema::table)
                .select((
                    users_schema::id,
                    users_schema::email,
                    users_schema::full_name,
                    roles_schema::name,
                    users_schema::deactivated,
                ))
                .order(users_schema::id.asc())
                .load(conn)
        })
        .await??;

    Ok(rows
        .into_iter()
        .map(|(id, email, full_name, role, deactivated)| UserSummary {
            id,
            email,
            full_name,
            role,
            deactivated,
        })
        .collect())
}

/// Create an account.
///
/// # Errors
///
#[utoipa::path(
    post,
    path = "/api/users",
    request_body = NewUserRequest,
    responses(
        (status = 200, description = "The created account", body = UserSummary),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 403, description = "Not an administrator", body = ErrorBody),
        (status = 409, description = "The address is taken", body = ErrorBody),
        (status = 422, description = "The account is not valid", body = ErrorBody),
    ),
    security(("session_token" = [])),
    tag = "administration"
)]
pub async fn create_user(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<NewUserRequest>,
) -> Result<Json<UserSummary>, ApiError> {
    drop(require_admin(&state, auth.session.user_id).await?);

    let email = request.email.trim().to_lowercase();
    let full_name = request.full_name.trim().to_owned();
    let role = request.role.trim().to_owned();

    if email.is_empty() || !email.contains('@') {
        return Err(ApiError::UnprocessableEntity(
            "An account needs an email address.".into(),
        ));
    }

    if email.chars().count() > MAX_EMAIL_CHARS {
        return Err(ApiError::UnprocessableEntity(format!(
            "An address must not exceed {MAX_EMAIL_CHARS} characters."
        )));
    }

    if full_name.is_empty() || full_name.chars().count() > MAX_NAME_CHARS {
        return Err(ApiError::UnprocessableEntity(format!(
            "A name is required and must not exceed {MAX_NAME_CHARS} characters."
        )));
    }

    if request.password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(ApiError::UnprocessableEntity(format!(
            "A password must be at least {MIN_PASSWORD_CHARS} characters."
        )));
    }

    let hash = hash_password(&request.password).map_err(ApiError::Internal)?;

    let conn = state.pool.get().await?;
    let (id, role) = conn
        .interact(move |conn| {
            let role_id: i64 = roles_schema::table
                .filter(roles_schema::name.eq(&role))
                .select(roles_schema::id)
                .first(conn)?;

            let id: i64 = diesel::insert_into(users_schema::table)
                .values((
                    users_schema::email.eq(&email),
                    users_schema::password_hash.eq(&hash),
                    users_schema::full_name.eq(&full_name),
                    users_schema::role_id.eq(role_id),
                ))
                .returning(users_schema::id)
                .get_result(conn)?;

            Ok::<_, diesel::result::Error>((id, role))
        })
        .await?
        .map_err(|err| match err {
            diesel::result::Error::NotFound => {
                ApiError::UnprocessableEntity("No such role.".into())
            }
            other => ApiError::from(other),
        })?;

    tracing::info!(
        actor = auth.session.user_id,
        created = id,
        "account created"
    );

    Ok(Json(UserSummary {
        id,
        email: request.email.trim().to_lowercase(),
        full_name: request.full_name.trim().to_owned(),
        role,
        deactivated: false,
    }))
}

/// Change an account's role, its activation, or both.
///
/// # Errors
///
#[utoipa::path(
    post,
    path = "/api/users/{id}",
    params(("id" = i64, Path, description = "User id")),
    request_body = UserUpdate,
    responses(
        (status = 200, description = "The account after the change", body = UserSummary),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 403, description = "Not allowed", body = ErrorBody),
        (status = 404, description = "No such account", body = ErrorBody),
        (status = 422, description = "No such role", body = ErrorBody),
    ),
    security(("session_token" = [])),
    tag = "administration"
)]
pub async fn update_user(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(request): Json<UserUpdate>,
) -> Result<Json<UserSummary>, ApiError> {
    drop(require_admin(&state, auth.session.user_id).await?);

    let actor = auth.session.user_id;
    let losing_admin = request.deactivated == Some(true)
        || request
            .role
            .as_deref()
            .is_some_and(|role| role != visibility::ADMINISTRATOR);

    if id == actor && losing_admin {
        return Err(ApiError::Forbidden(
            "An administrator cannot remove their own access.".into(),
        ));
    }

    let conn = state.pool.get().await?;
    let role = request.role.map(|role| role.trim().to_owned());
    let deactivated = request.deactivated;

    conn.interact(move |conn| {
        conn.transaction(|conn| {
            if let Some(ref role) = role {
                let role_id: i64 = roles_schema::table
                    .filter(roles_schema::name.eq(role))
                    .select(roles_schema::id)
                    .first(conn)?;

                diesel::update(users_schema::table.filter(users_schema::id.eq(id)))
                    .set((
                        users_schema::role_id.eq(role_id),
                        users_schema::updated_at.eq(Utc::now().naive_utc()),
                    ))
                    .execute(conn)?;
            }

            if let Some(deactivated) = deactivated {
                diesel::update(users_schema::table.filter(users_schema::id.eq(id)))
                    .set((
                        users_schema::deactivated.eq(deactivated),
                        users_schema::updated_at.eq(Utc::now().naive_utc()),
                    ))
                    .execute(conn)?;
            }

            Ok::<_, diesel::result::Error>(())
        })
    })
    .await?
    .map_err(|err| match err {
        diesel::result::Error::NotFound => ApiError::UnprocessableEntity("No such role.".into()),
        other => ApiError::from(other),
    })?;

    tracing::info!(actor, changed = id, "account updated");

    one_user(&state, id).await.map(Json)
}

async fn one_user(state: &AppState, id: i64) -> Result<UserSummary, ApiError> {
    let conn = state.pool.get().await?;
    let (id, email, full_name, role, deactivated): (i64, String, String, String, bool) = conn
        .interact(move |conn| {
            users_schema::table
                .inner_join(roles_schema::table)
                .filter(users_schema::id.eq(id))
                .select((
                    users_schema::id,
                    users_schema::email,
                    users_schema::full_name,
                    roles_schema::name,
                    users_schema::deactivated,
                ))
                .first(conn)
        })
        .await??;

    Ok(UserSummary {
        id,
        email,
        full_name,
        role,
        deactivated,
    })
}

/// Take a node out of the fleet, or put it back.
///
///
/// # Errors
///
#[utoipa::path(
    post,
    path = "/api/nodes/{id}/suspension",
    params(("id" = i64, Path, description = "Node id")),
    request_body = SuspensionRequest,
    responses(
        (status = 200, description = "The node's suspension after the change", body = SuspensionRequest),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 403, description = "Not an administrator", body = ErrorBody),
        (status = 404, description = "No such node", body = ErrorBody),
    ),
    security(("session_token" = [])),
    tag = "administration"
)]
pub async fn set_suspension(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(request): Json<SuspensionRequest>,
) -> Result<Json<SuspensionRequest>, ApiError> {
    drop(require_admin(&state, auth.session.user_id).await?);

    let suspended = request.suspended;
    let conn = state.pool.get().await?;
    let updated: usize = conn
        .interact(move |conn| {
            diesel::update(nodes_schema::table.filter(nodes_schema::id.eq(id)))
                .set(nodes_schema::suspended.eq(suspended))
                .execute(conn)
        })
        .await??;

    if updated == 0 {
        return Err(ApiError::NotFound("No such node.".into()));
    }

    tracing::info!(
        actor = auth.session.user_id,
        node_id = id,
        suspended,
        "node suspension changed"
    );

    state.publish_node(NodeEvent::SuspensionChanged {
        node_id: id,
        suspended,
    });

    Ok(Json(SuspensionRequest { suspended }))
}

/// A node an administrator is planning, before any receiver exists for it.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PlannedNodeRequest {
    pub name: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub owner_id: Option<i64>,
}

/// A change to a node's record.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NodeUpdate {
    pub name: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    /// Administrators only, since owning a node grants the right to edit it.
    pub owner_id: Option<i64>,
}

/// A credential, shown once.
#[derive(Debug, Serialize, ToSchema)]
pub struct MintedCredential {
    pub node_id: i64,
    /// The only time this value is ever returned; it is not readable again.
    pub credential: String,
}

fn check_position(latitude: Option<f64>, longitude: Option<f64>) -> Result<(), ApiError> {
    match (latitude, longitude) {
        (None, None) => Ok(()),
        (Some(lat), Some(lon))
            if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) =>
        {
            Ok(())
        }
        (Some(_), Some(_)) => Err(ApiError::UnprocessableEntity(
            "A position must be a latitude in -90..90 and a longitude in -180..180.".into(),
        )),
        _ => Err(ApiError::UnprocessableEntity(
            "A position needs both a latitude and a longitude.".into(),
        )),
    }
}

/// Plan a node and mint the credential it will enrol with.
///
/// # Errors
///
#[utoipa::path(
    post,
    path = "/api/nodes",
    request_body = PlannedNodeRequest,
    responses(
        (status = 200, description = "The node and its enrollment credential", body = MintedCredential),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 403, description = "Not an administrator", body = ErrorBody),
        (status = 422, description = "The node is not valid", body = ErrorBody),
    ),
    security(("session_token" = [])),
    tag = "administration"
)]
pub async fn plan_node(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<PlannedNodeRequest>,
) -> Result<Json<MintedCredential>, ApiError> {
    drop(require_admin(&state, auth.session.user_id).await?);

    let name = request.name.trim().to_owned();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(ApiError::UnprocessableEntity(format!(
            "A name is required and must not exceed {MAX_NAME_CHARS} characters."
        )));
    }

    check_position(request.latitude, request.longitude)?;

    let credential = crate::grpc::generate_token();
    let token = credential.clone();
    let planned = PlannedNode {
        name,
        latitude: request.latitude,
        longitude: request.longitude,
        hardware: serde_json::json!({}),
        capabilities: serde_json::json!({}),
        owner_id: request.owner_id,
    };

    let conn = state.pool.get().await?;
    let node_id: i64 = conn
        .interact(move |conn| {
            conn.transaction(|conn| {
                let node_id: i64 = diesel::insert_into(nodes_schema::table)
                    .values(&planned)
                    .returning(nodes_schema::id)
                    .get_result(conn)?;

                diesel::insert_into(credentials_schema::table)
                    .values(&NewNodeCredential { node_id, token })
                    .execute(conn)?;

                Ok::<_, diesel::result::Error>(node_id)
            })
        })
        .await??;

    tracing::info!(actor = auth.session.user_id, node_id, "node planned");

    Ok(Json(MintedCredential {
        node_id,
        credential,
    }))
}

/// Revoke a node's live credential and issue a new one.
///
/// # Errors
///
#[utoipa::path(
    post,
    path = "/api/nodes/{id}/credential",
    params(("id" = i64, Path, description = "Node id")),
    responses(
        (status = 200, description = "The new credential", body = MintedCredential),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 403, description = "Not an administrator", body = ErrorBody),
        (status = 404, description = "No such node", body = ErrorBody),
    ),
    security(("session_token" = [])),
    tag = "administration"
)]
pub async fn rotate_credential(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<MintedCredential>, ApiError> {
    drop(require_admin(&state, auth.session.user_id).await?);

    let credential = crate::grpc::generate_token();
    let token = credential.clone();
    let now = Utc::now().naive_utc();
    let conn = state.pool.get().await?;

    conn.interact(move |conn| {
        conn.transaction(|conn| {
            let exists: i64 = nodes_schema::table
                .filter(nodes_schema::id.eq(id))
                .count()
                .get_result(conn)?;
            if exists == 0 {
                return Err(diesel::result::Error::NotFound);
            }

            diesel::update(
                credentials_schema::table
                    .filter(credentials_schema::node_id.eq(id))
                    .filter(credentials_schema::revoked_at.is_null()),
            )
            .set(credentials_schema::revoked_at.eq(now))
            .execute(conn)?;

            diesel::insert_into(credentials_schema::table)
                .values(&NewNodeCredential { node_id: id, token })
                .execute(conn)?;

            Ok(())
        })
    })
    .await?
    .map_err(|err: diesel::result::Error| match err {
        diesel::result::Error::NotFound => ApiError::NotFound("No such node.".into()),
        other => ApiError::from(other),
    })?;

    tracing::warn!(
        actor = auth.session.user_id,
        node_id = id,
        "node credential rotated"
    );

    Ok(Json(MintedCredential {
        node_id: id,
        credential,
    }))
}

/// Change a node's name, position or owner.
///
/// # Errors
///
#[utoipa::path(
    post,
    path = "/api/nodes/{id}",
    params(("id" = i64, Path, description = "Node id")),
    request_body = NodeUpdate,
    responses(
        (status = 200, description = "The node after the change", body = crate::db::models::nodes::Node),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 403, description = "Not allowed to edit this node", body = ErrorBody),
        (status = 404, description = "No such node", body = ErrorBody),
        (status = 422, description = "The change is not valid", body = ErrorBody),
    ),
    security(("session_token" = [])),
    tag = "administration"
)]
pub async fn update_node(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(request): Json<NodeUpdate>,
) -> Result<Json<Node>, ApiError> {
    let access = visibility::resolve(&state, auth.session.user_id).await?;
    if !visibility::may_edit_node(&state, &access, id).await? {
        return Err(ApiError::Forbidden(
            "You may only edit a node you own.".into(),
        ));
    }
    if request.owner_id.is_some() && !access.is_admin() {
        return Err(ApiError::Forbidden(
            "Only an administrator may reassign a node.".into(),
        ));
    }

    let name = match request.name {
        Some(name) => {
            let name = name.trim().to_owned();
            if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
                return Err(ApiError::UnprocessableEntity(format!(
                    "A name must not be empty or exceed {MAX_NAME_CHARS} characters."
                )));
            }
            Some(name)
        }
        None => None,
    };
    check_position(request.latitude, request.longitude)?;

    let owner_id = request.owner_id;
    let (latitude, longitude) = (request.latitude, request.longitude);
    let conn = state.pool.get().await?;
    let node: Node = conn
        .interact(move |conn| {
            conn.transaction(|conn| {
                if let Some(name) = name {
                    diesel::update(nodes_schema::table.filter(nodes_schema::id.eq(id)))
                        .set(nodes_schema::name.eq(name))
                        .execute(conn)?;
                }
                diesel::update(nodes_schema::table.filter(nodes_schema::id.eq(id)))
                    .set((
                        nodes_schema::latitude.eq(latitude),
                        nodes_schema::longitude.eq(longitude),
                    ))
                    .execute(conn)?;
                if let Some(owner_id) = owner_id {
                    diesel::update(nodes_schema::table.filter(nodes_schema::id.eq(id)))
                        .set(nodes_schema::owner_id.eq((owner_id != 0).then_some(owner_id)))
                        .execute(conn)?;
                }

                nodes_schema::table
                    .filter(nodes_schema::id.eq(id))
                    .select(Node::as_select())
                    .first(conn)
            })
        })
        .await??;

    tracing::info!(actor = auth.session.user_id, node_id = id, "node updated");

    Ok(Json(node))
}

/// The administration page and the fragments it swaps.
pub mod fragments {
    use askama::Template;
    use axum::Form;
    use axum::extract::{Path, State};
    use axum::response::Html;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use diesel::prelude::*;

    use super::{
        MintedCredential, NewUserRequest, PlannedNodeRequest, SuspensionRequest, UserSummary,
        UserUpdate, all_users, nodes_schema, require_admin,
    };
    use crate::api::auth::{AuthPage, AuthUser};
    use crate::api::errors::ApiError;
    use crate::api::pages;
    use crate::state::AppState;
    use crate::templates::{
        AdminNodeRow, AdminNodesFragment, AdminTemplate, AdminUserRow, AdminUsersFragment,
        CredentialReveal,
    };

    const ROLES: [(&str, &str); 4] = [
        ("administrator", "Administrator"),
        ("operator", "Operatør"),
        ("technician", "Tekniker"),
        ("reader", "Læser"),
    ];

    /// All administration routes: one page and the fragments it swaps.
    pub fn routes() -> Router<AppState> {
        Router::new()
            .route("/admin", get(page))
            .route("/fragments/admin/users", get(users).post(create))
            .route("/fragments/admin/users/{id}", post(update))
            .route("/fragments/admin/nodes", get(nodes).post(plan))
            .route("/fragments/admin/nodes/{id}/suspension", post(suspension))
            .route("/fragments/admin/nodes/{id}/credential", post(rotate))
    }

    ///
    async fn plan(
        auth: AuthUser,
        State(state): State<AppState>,
        Form(request): Form<PlannedNodeRequest>,
    ) -> Result<Html<String>, ApiError> {
        let name = request.name.trim().to_owned();
        let minted = super::plan_node(auth, State(state.clone()), Json(request))
            .await?
            .0;

        reveal(&state, minted, name).await
    }

    async fn rotate(
        auth: AuthUser,
        State(state): State<AppState>,
        Path(id): Path<i64>,
    ) -> Result<Html<String>, ApiError> {
        let minted = super::rotate_credential(auth, State(state.clone()), Path(id))
            .await?
            .0;

        let conn = state.pool.get().await?;
        let name: String = conn
            .interact(move |conn| {
                nodes_schema::table
                    .filter(nodes_schema::id.eq(id))
                    .select(nodes_schema::name)
                    .first(conn)
            })
            .await??;

        reveal(&state, minted, name).await
    }

    async fn reveal(
        state: &AppState,
        minted: MintedCredential,
        node_name: String,
    ) -> Result<Html<String>, ApiError> {
        let credential = CredentialReveal {
            node_id: minted.node_id,
            node_name,
            credential: minted.credential,
        }
        .render()
        .map_err(ApiError::internal)?;

        let list = AdminNodesFragment {
            nodes: node_rows(state).await?,
            list: true,
        }
        .render()
        .map_err(ApiError::internal)?;

        Ok(Html(format!(
            "{list}<div id=\"node-credential\" hx-swap-oob=\"innerHTML\">{credential}</div>"
        )))
    }

    async fn page(auth: AuthPage, State(state): State<AppState>) -> Result<Html<String>, ApiError> {
        let user_id = auth.0.session.user_id;
        let access = require_admin(&state, user_id).await?;
        let chrome = pages::chrome_for(&state, user_id).await?;

        let html = AdminTemplate {
            users: rows(all_users(&state).await?),
            nodes: node_rows(&state).await?,
            roles: ROLES.to_vec(),
            nodes_total: chrome.nodes_total,
            silent: chrome.silent,
            open_alarms: chrome.open_alarms,
            open_orders: chrome.open_orders,
            user_name: chrome.user_name,
            user_role: access.role,
            current_user_id: user_id,
            list: true,
            live: false,
        }
        .render()
        .map_err(ApiError::internal)?;

        Ok(Html(html))
    }

    async fn users(
        auth: AuthPage,
        State(state): State<AppState>,
    ) -> Result<Html<String>, ApiError> {
        let user_id = auth.0.session.user_id;
        drop(require_admin(&state, user_id).await?);

        render_users(&state, user_id).await
    }

    async fn create(
        auth: AuthUser,
        State(state): State<AppState>,
        Form(request): Form<NewUserRequest>,
    ) -> Result<Html<String>, ApiError> {
        let user_id = auth.session.user_id;

        drop(super::create_user(auth, State(state.clone()), Json(request)).await?);

        render_users(&state, user_id).await
    }

    async fn update(
        auth: AuthUser,
        State(state): State<AppState>,
        Path(id): Path<i64>,
        Form(request): Form<UserUpdate>,
    ) -> Result<Html<String>, ApiError> {
        let user_id = auth.session.user_id;

        let updated = super::update_user(auth, State(state.clone()), Path(id), Json(request))
            .await?
            .0;

        let html = AdminUsersFragment {
            users: rows(vec![updated]),
            roles: ROLES.to_vec(),
            current_user_id: user_id,
            list: false,
        }
        .render()
        .map_err(ApiError::internal)?;

        Ok(Html(html))
    }

    async fn nodes(
        auth: AuthPage,
        State(state): State<AppState>,
    ) -> Result<Html<String>, ApiError> {
        drop(require_admin(&state, auth.0.session.user_id).await?);

        let html = AdminNodesFragment {
            nodes: node_rows(&state).await?,
            list: true,
        }
        .render()
        .map_err(ApiError::internal)?;

        Ok(Html(html))
    }

    async fn suspension(
        auth: AuthUser,
        State(state): State<AppState>,
        Path(id): Path<i64>,
        Form(request): Form<SuspensionRequest>,
    ) -> Result<Html<String>, ApiError> {
        drop(super::set_suspension(auth, State(state.clone()), Path(id), Json(request)).await?);

        let conn = state.pool.get().await?;
        let (id, name, suspended, identity): (i64, String, bool, Option<String>) = conn
            .interact(move |conn| {
                nodes_schema::table
                    .filter(nodes_schema::id.eq(id))
                    .select((
                        nodes_schema::id,
                        nodes_schema::name,
                        nodes_schema::suspended,
                        nodes_schema::external_identity,
                    ))
                    .first(conn)
            })
            .await??;

        let html = AdminNodesFragment {
            nodes: vec![AdminNodeRow {
                id,
                name,
                suspended,
                enrolled: identity.is_some(),
            }],
            list: false,
        }
        .render()
        .map_err(ApiError::internal)?;

        Ok(Html(html))
    }

    async fn render_users(
        state: &AppState,
        current_user_id: i64,
    ) -> Result<Html<String>, ApiError> {
        let html = AdminUsersFragment {
            users: rows(all_users(state).await?),
            roles: ROLES.to_vec(),
            current_user_id,
            list: true,
        }
        .render()
        .map_err(ApiError::internal)?;

        Ok(Html(html))
    }

    fn rows(users: Vec<UserSummary>) -> Vec<AdminUserRow> {
        users
            .into_iter()
            .map(|user| AdminUserRow {
                id: user.id,
                email: user.email,
                full_name: user.full_name,
                role: user.role,
                deactivated: user.deactivated,
            })
            .collect()
    }

    async fn node_rows(state: &AppState) -> Result<Vec<AdminNodeRow>, ApiError> {
        let conn = state.pool.get().await?;
        let rows: Vec<(i64, String, bool, Option<String>)> = conn
            .interact(move |conn| {
                nodes_schema::table
                    .select((
                        nodes_schema::id,
                        nodes_schema::name,
                        nodes_schema::suspended,
                        nodes_schema::external_identity,
                    ))
                    .order(nodes_schema::name.asc())
                    .load(conn)
            })
            .await??;

        Ok(rows
            .into_iter()
            .map(|(id, name, suspended, identity)| AdminNodeRow {
                id,
                name,
                suspended,
                enrolled: identity.is_some(),
            })
            .collect())
    }
}
