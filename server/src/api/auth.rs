use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher as _, PasswordVerifier as _};
use askama::Template;
use axum::extract::{FromRequestParts, OptionalFromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Router};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use chrono::{Duration, Utc};
use diesel::{ExpressionMethods, OptionalExtension as _, QueryDsl, RunQueryDsl, SelectableHelper};
use rand::RngExt as _;
use serde::Deserialize;

use super::errors::ApiError;
use crate::db::models::sessions::NewSession;
use crate::db::models::users::NewUser;
use crate::db::schema::users as users_schema;
use crate::templates::LoginTemplate;
use crate::{
    db::{models::sessions::Session, schema::sessions as sessions_schema},
    state::AppState,
};

/// Name of the cookie a session travels in.
pub const COOKIE: &str = "session_token";

/// The authenticated user behind a request.
pub struct AuthUser {
    pub session: Session,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = extract_token(parts)?;

        let conn = state.pool.get().await?;

        let token_clone = token.clone();
        let (session, deactivated): (Session, bool) = conn
            .interact(move |conn| {
                sessions_schema::table
                    .inner_join(users_schema::table)
                    .filter(sessions_schema::token.eq(&token_clone))
                    .select((Session::as_select(), users_schema::deactivated))
                    .first(conn)
            })
            .await?
            .map_err(|e| match e {
                diesel::result::Error::NotFound => {
                    ApiError::Unauthorized("Invalid or expired session token.".into())
                }
                other => ApiError::internal(other),
            })?;

        if deactivated {
            return Err(ApiError::Unauthorized(
                "This account has been deactivated.".into(),
            ));
        }

        if session.expires_at < Utc::now().naive_utc() {
            return Err(ApiError::Unauthorized(
                "Session has expired. Please log in again.".into(),
            ));
        }

        Ok(Self { session })
    }
}

/// Same lookup as [`AuthUser`], without rejecting a missing session.
impl OptionalFromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Option<Self>, Self::Rejection> {
        match <Self as FromRequestParts<AppState>>::from_request_parts(parts, state).await {
            Ok(auth) => Ok(Some(auth)),
            Err(ApiError::Unauthorized(_)) => Ok(None),
            Err(other) => Err(other),
        }
    }
}

/// Whether a session token still resolves to a live account.
pub async fn session_is_live(state: &AppState, token: &str) -> bool {
    let Ok(conn) = state.pool.get().await else {
        return true;
    };

    let token = token.to_owned();
    let found = conn
        .interact(move |conn| {
            sessions_schema::table
                .inner_join(users_schema::table)
                .filter(sessions_schema::token.eq(&token))
                .select((sessions_schema::expires_at, users_schema::deactivated))
                .first::<(chrono::NaiveDateTime, bool)>(conn)
                .optional()
        })
        .await;

    match found {
        Ok(Ok(Some((expires_at, deactivated)))) => {
            !deactivated && expires_at >= Utc::now().naive_utc()
        }
        Ok(Ok(None)) => false,
        Ok(Err(_)) | Err(_) => true,
    }
}

fn extract_token(parts: &Parts) -> Result<String, ApiError> {
    let jar = CookieJar::from_headers(&parts.headers);
    if let Some(cookie) = jar.get(COOKIE) {
        let value = cookie.value().trim();
        if !value.is_empty() {
            return Ok(value.to_owned());
        }
    }

    if let Some(auth_header) = parts.headers.get("authorization") {
        let header_str = auth_header
            .to_str()
            .map_err(|_| ApiError::Unauthorized("Invalid Authorization header.".into()))?;

        if let Some(token) = header_str.strip_prefix("Bearer ") {
            let token = token.trim();
            if !token.is_empty() {
                return Ok(token.to_owned());
            }
        }
    }

    Err(ApiError::Unauthorized(
        "Missing session token. Provide a `session_token` cookie or `Authorization: Bearer` header.".into(),
    ))
}

/// The authenticated user behind a page request.
pub struct AuthPage(pub AuthUser);

impl FromRequestParts<AppState> for AuthPage {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        match <AuthUser as FromRequestParts<AppState>>::from_request_parts(parts, state).await {
            Ok(auth) => Ok(Self(auth)),
            Err(_) => Err(to_login(&parts.headers)),
        }
    }
}

const LOGIN_PATH: &str = "/login";

const HTMX_REQUEST: &str = "hx-request";
const HTMX_REDIRECT: &str = "hx-redirect";

fn to_login(headers: &HeaderMap) -> Response {
    if headers.contains_key(HTMX_REQUEST) {
        return (StatusCode::NO_CONTENT, [(HTMX_REDIRECT, LOGIN_PATH)]).into_response();
    }

    Redirect::to(LOGIN_PATH).into_response()
}

const SESSION_HOURS: i64 = 12;

const TOKEN_BYTES: usize = 32;

const ADMIN_EMAIL_ENV: &str = "SPEKTRA_ADMIN_EMAIL";
const ADMIN_PASSWORD_ENV: &str = "SPEKTRA_ADMIN_PASSWORD";

const ADMINISTRATOR_ROLE: i64 = 1;

/// All routes that create and destroy a session.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route(LOGIN_PATH, get(login_form).post(login))
        .route("/logout", post(logout))
}

/// What the login form posts.
#[derive(Debug, Deserialize)]
pub struct Credentials {
    pub email: String,
    pub password: String,
}

async fn login_form(auth: Option<AuthUser>, jar: CookieJar) -> Response {
    if auth.is_some() {
        return Redirect::to("/").into_response();
    }

    if jar.get(COOKIE).is_some() {
        return (jar.remove(Cookie::from(COOKIE)), render_login(None)).into_response();
    }

    render_login(None)
}

fn render_login(error: Option<&str>) -> Response {
    match (LoginTemplate {
        error: error.map(ToOwned::to_owned),
    })
    .render()
    {
        Ok(html) => Html(html).into_response(),
        Err(err) => ApiError::internal(err).into_response(),
    }
}

async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(credentials): Form<Credentials>,
) -> Result<Response, ApiError> {
    let email = credentials.email.trim().to_lowercase();
    let conn = state.pool.get().await?;

    let found: Option<(i64, String, bool)> = {
        let email = email.clone();

        conn.interact(move |conn| {
            users_schema::table
                .filter(users_schema::email.eq(email))
                .select((
                    users_schema::id,
                    users_schema::password_hash,
                    users_schema::deactivated,
                ))
                .first(conn)
                .optional()
        })
        .await??
    };

    let Some((user_id, hash, deactivated)) = found else {
        let _matched = verify(&credentials.password, &decoy_hash());

        tracing::warn!(%email, "login attempt for an unknown account");

        return Ok(render_login(Some(
            "Forkert e-mailadresse eller adgangskode.",
        )));
    };

    if !verify(&credentials.password, &hash) {
        tracing::warn!(user_id, "login attempt with a wrong password");

        return Ok(render_login(Some(
            "Forkert e-mailadresse eller adgangskode.",
        )));
    }

    if deactivated {
        tracing::warn!(user_id, "login attempt on a deactivated account");

        return Ok(render_login(Some(
            "Kontoen er deaktiveret. Kontakt en administrator.",
        )));
    }

    let token = generate_token();
    let session = NewSession {
        token: token.clone(),
        user_id,
        expires_at: Utc::now()
            .checked_add_signed(Duration::hours(SESSION_HOURS))
            .unwrap_or_else(Utc::now)
            .naive_utc(),
    };
    conn.interact(move |conn| {
        diesel::insert_into(sessions_schema::table)
            .values(&session)
            .execute(conn)
    })
    .await??;

    tracing::info!(user_id, "session opened");

    Ok((jar.add(session_cookie(token)), Redirect::to("/")).into_response())
}

async fn logout(State(state): State<AppState>, jar: CookieJar) -> Result<Response, ApiError> {
    if let Some(token) = jar.get(COOKIE).map(|cookie| cookie.value().to_owned()) {
        let conn = state.pool.get().await?;
        conn.interact(move |conn| {
            diesel::delete(sessions_schema::table.filter(sessions_schema::token.eq(token)))
                .execute(conn)
        })
        .await??;
    }

    Ok((jar.remove(Cookie::from(COOKIE)), Redirect::to(LOGIN_PATH)).into_response())
}

fn session_cookie(token: String) -> Cookie<'static> {
    let insecure = std::env::var("SPEKTRA_INSECURE_COOKIES").is_ok_and(|value| value == "1");

    Cookie::build((COOKIE, token))
        .path("/")
        .http_only(true)
        .secure(!insecure)
        .same_site(SameSite::Lax)
        .build()
}

/// Hash a password for storage.
///
/// # Errors
///
/// Returns a message if argon2 refuses.
pub fn hash_password(password: &str) -> Result<String, String> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(|err| err.to_string())
}

/// Whether a password matches a stored PHC string.
#[must_use]
pub fn verify(password: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(err) => {
            tracing::error!(error = %err, "a stored password hash does not parse");

            false
        }
    }
}

fn decoy_hash() -> String {
    hash_password("spektra-decoy").unwrap_or_default()
}

fn generate_token() -> String {
    let mut bytes = [0_u8; TOKEN_BYTES];
    rand::rng().fill(&mut bytes[..]);

    hex::encode(bytes)
}

/// Create the administrator named by the environment, if there are no users.
///
/// # Errors
///
/// Returns a message if the database refuses or the password cannot be hashed.
pub async fn bootstrap_admin(pool: &deadpool_diesel::postgres::Pool) -> Result<bool, String> {
    let (Ok(email), Ok(password)) = (
        std::env::var(ADMIN_EMAIL_ENV),
        std::env::var(ADMIN_PASSWORD_ENV),
    ) else {
        return Ok(false);
    };

    if email.trim().is_empty() || password.is_empty() {
        return Ok(false);
    }

    let hash = hash_password(&password)?;
    let conn = pool.get().await.map_err(|err| err.to_string())?;
    conn.interact(move |conn| {
        let existing: i64 = users_schema::table.count().get_result(conn)?;
        if existing > 0 {
            return Ok(false);
        }

        diesel::insert_into(users_schema::table)
            .values(&NewUser {
                email: email.trim().to_lowercase(),
                password_hash: hash,
                full_name: "Administrator".to_owned(),
                role_id: ADMINISTRATOR_ROLE,
            })
            .execute(conn)?;

        Ok::<_, diesel::result::Error>(true)
    })
    .await
    .map_err(|err| format!("the bootstrap task panicked: {err:?}"))?
    .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_verifies_against_its_own_hash() {
        let hash = hash_password("correct horse battery staple").expect("a hash");

        assert!(verify("correct horse battery staple", &hash));
    }

    #[test]
    fn a_wrong_password_does_not_verify() {
        let hash = hash_password("correct horse battery staple").expect("a hash");

        assert!(!verify("Correct horse battery staple", &hash));
        assert!(!verify("", &hash));
        assert!(!verify("correct horse battery stapl", &hash));
    }

    #[test]
    fn the_same_password_hashes_differently_every_time() {
        let first = hash_password("hunter2").expect("a hash");
        let second = hash_password("hunter2").expect("a hash");

        assert_ne!(first, second);
        assert!(verify("hunter2", &first));
        assert!(verify("hunter2", &second));
    }

    #[test]
    fn a_hash_is_a_phc_string() {
        let hash = hash_password("hunter2").expect("a hash");

        assert!(hash.starts_with("$argon2id$"), "got {hash}");
    }

    #[test]
    fn a_corrupt_hash_is_not_a_match() {
        assert!(!verify("hunter2", "not a hash at all"));
        assert!(!verify("hunter2", ""));
    }

    #[test]
    fn the_decoy_is_a_real_hash_that_matches_nothing_a_user_would_pick() {
        let decoy = decoy_hash();

        assert!(decoy.starts_with("$argon2id$"));
        assert!(!verify("hunter2", &decoy));
    }

    #[test]
    fn a_token_is_sixty_four_hex_characters() {
        let token = generate_token();

        assert_eq!(token.len(), TOKEN_BYTES * 2);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(token, generate_token());
    }

    #[test]
    fn the_session_cookie_is_not_reachable_from_script() {
        let cookie = session_cookie("deadbeef".to_owned());

        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(cookie.same_site(), Some(SameSite::Lax));
        assert_eq!(cookie.path(), Some("/"));
    }
}
