//! Typed API errors with HTTP mapping. No panics on the request path.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("not found")]
    NotFound,
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("validation: {0}")]
    Validation(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("internal error")]
    Internal(#[from] anyhow::Error),
}

impl From<core_db::DbError> for ApiError {
    fn from(e: core_db::DbError) -> Self {
        match &e {
            core_db::DbError::NotFound => ApiError::NotFound,
            core_db::DbError::Sql(sqlx::Error::RowNotFound) => ApiError::NotFound,
            core_db::DbError::Sql(sqlx::Error::Database(db)) if db.constraint().is_some() => {
                ApiError::Conflict(db.constraint().unwrap_or("").to_string())
            }
            _ => {
                tracing::error!(error = %e, "database error");
                ApiError::Internal(anyhow::anyhow!(e.to_string()))
            }
        }
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        Self::from(core_db::DbError::Sql(e))
    }
}

impl From<core_domain::DomainError> for ApiError {
    fn from(e: core_domain::DomainError) -> Self {
        use core_domain::DomainError as D;
        match e {
            D::ChecklistIncomplete(n) => ApiError::Validation(format!(
                "pre-close checklist incomplete: {n} unchecked item(s)"
            )),
            other => ApiError::Validation(other.to_string()),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            ApiError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, m.clone()),
            ApiError::Forbidden(m) => (StatusCode::FORBIDDEN, m.clone()),
            ApiError::Validation(m) => (StatusCode::UNPROCESSABLE_ENTITY, m.clone()),
            ApiError::Conflict(m) => (StatusCode::CONFLICT, m.clone()),
            ApiError::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal error".to_string(),
            ),
        };
        (
            status,
            Json(json!({ "error": { "code": status.as_u16(), "message": message } })),
        )
            .into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
