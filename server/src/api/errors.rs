use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use utoipa::ToSchema;

/// A structured JSON error body returned to clients.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    /// Machine-readable code, e.g. `"not_found"`.
    pub error: &'static str,
    /// Human-readable description of what went wrong.
    pub message: String,
}

/// Unified API error type.
#[derive(Debug)]
pub enum ApiError {
    /// 401 - missing or invalid credentials.
    Unauthorized(String),
    /// 403 - authenticated but not allowed to perform this action.
    Forbidden(String),
    /// 404 - the requested resource does not exist.
    NotFound(String),
    /// 409 - a uniqueness constraint was violated.
    Conflict(String),
    /// 422 - the request failed validation.
    UnprocessableEntity(String),
    /// 500 - an unexpected internal error.
    Internal(String),
}

impl ApiError {
    /// Wrap any `std::error::Error` as [`ApiError::Internal`].
    pub fn internal<E: std::error::Error>(err: E) -> Self {
        Self::Internal(err.to_string())
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unauthorized(msg) => write!(f, "Unauthorized: {msg}"),
            Self::Forbidden(msg) => write!(f, "Forbidden: {msg}"),
            Self::NotFound(msg) => write!(f, "Not found: {msg}"),
            Self::Conflict(msg) => write!(f, "Conflict: {msg}"),
            Self::UnprocessableEntity(msg) => write!(f, "Unprocessable entity: {msg}"),
            Self::Internal(msg) => write!(f, "Internal error: {msg}"),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error_code, message) = match &self {
            Self::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, "unauthorized", msg.clone()),
            Self::Forbidden(msg) => (StatusCode::FORBIDDEN, "forbidden", msg.clone()),
            Self::NotFound(msg) => (StatusCode::NOT_FOUND, "not_found", msg.clone()),
            Self::Conflict(msg) => (StatusCode::CONFLICT, "conflict", msg.clone()),
            Self::UnprocessableEntity(msg) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "unprocessable_entity",
                msg.clone(),
            ),
            Self::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "An unexpected error occurred.".to_owned(),
            ),
        };

        if let Self::Internal(detail) = &self {
            tracing::error!(%status, detail, "internal server error");
        } else {
            tracing::warn!(%status, %message, "client error");
        }

        (
            status,
            Json(ErrorBody {
                error: error_code,
                message,
            }),
        )
            .into_response()
    }
}

impl From<deadpool_diesel::InteractError> for ApiError {
    fn from(err: deadpool_diesel::InteractError) -> Self {
        Self::internal(err)
    }
}

impl From<deadpool_diesel::PoolError> for ApiError {
    fn from(err: deadpool_diesel::PoolError) -> Self {
        Self::internal(err)
    }
}

impl From<diesel::result::Error> for ApiError {
    fn from(err: diesel::result::Error) -> Self {
        match &err {
            diesel::result::Error::RollbackTransaction => {
                Self::Forbidden("Action fails to meet structural requirements.".into())
            }
            diesel::result::Error::NotFound => {
                Self::NotFound("The requested resource was not found.".into())
            }
            diesel::result::Error::DatabaseError(
                diesel::result::DatabaseErrorKind::UniqueViolation,
                info,
            ) => Self::Conflict(info.message().to_owned()),
            diesel::result::Error::DatabaseError(
                diesel::result::DatabaseErrorKind::NotNullViolation
                | diesel::result::DatabaseErrorKind::CheckViolation,
                info,
            ) => Self::UnprocessableEntity(info.message().to_owned()),
            _ => Self::internal(err),
        }
    }
}
