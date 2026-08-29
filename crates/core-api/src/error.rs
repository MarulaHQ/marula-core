//! Typed API errors with HTTP mapping. No panics on the request path.
//!
//! Instant-first contract (docs/instant-first.md IF3): every error body
//! carries a machine-readable `reason` code clients branch on — never prose
//! to parse. The legacy `code` (HTTP status) and `message` keys stay so the
//! change is purely additive for existing consumers:
//!
//! ```json
//! { "error": { "code": 409, "reason": "version_conflict", "message": "...", ...extra } }
//! ```
//!
//! Route handlers added by parallel lanes converge on this envelope for
//! free: returning `ApiError` from any handler is all it takes. Reason
//! vocabulary: `not_found`, `rls_sparse`, `unauthorized`, `forbidden`,
//! `validation_failed`, `conflict`, `version_conflict`,
//! `idempotency_key_reused`, `pre_close_checklist_blocked`,
//! `dispatch_move_conflict`, `import_chunk_rejected`, `service_unavailable`,
//! `internal_error`.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("not found")]
    NotFound,
    /// The mutation target is not visible under the caller's row-level
    /// scope (RLS filtered it, it was deleted, or it never existed — the
    /// three are deliberately indistinguishable, which is the RLS security
    /// property). Mutations get this instead of `not_found` so clients can
    /// branch: a write that did not land because the target left the
    /// visible scope reconciles by resyncing, not by re-POSTing.
    #[error("mutation target not visible in scope")]
    RlsSparse,
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("validation: {0}")]
    Validation(String),
    #[error("conflict: {0}")]
    Conflict(String),
    /// Typed 409 for an If-Match-style version mismatch (instant-first
    /// IF1): the caller's `version` no longer matches the entity's current
    /// monotonic seq, so the write was rejected before anything changed.
    #[error("version conflict: entity moved from version {provided} to {expected}")]
    VersionConflict {
        /// The entity's current version (what the caller must re-read).
        expected: i64,
        /// The stale version the caller presented.
        provided: i64,
    },
    /// Typed 409 when an idempotency key is reused with a different
    /// request payload (or against a different endpoint family). The key
    /// is a replay contract, not a dedupe hint: same key + different body
    /// is a client bug we refuse to guess at.
    #[error("idempotency key reused with a different request")]
    IdempotencyKeyReused,
    /// Typed 409 for a close attempt blocked by the pre-close checklist:
    /// carries the unchecked REQUIRED item texts so clients can render the
    /// exact blockers without a second round-trip.
    #[error("pre-close checklist incomplete")]
    ChecklistBlocked { missing: Vec<String> },
    /// Typed 409 for a rejected dispatch bulk-move (TASKLIST 2.19): names
    /// the offending move index and the blocking booking so the board can
    /// flag the exact drop target and surface the conflict inline.
    #[error("dispatch move conflicts with existing booking")]
    DispatchMoveConflict {
        /// Index into the request's `moves` array that was rejected.
        index: usize,
        appointment_id: Uuid,
        /// Title of the blocking booking (empty when the rejection is a
        /// non-overlap failure such as an invalid span).
        blocker_title: String,
    },
    /// Capability present but disabled by configuration (fail-closed
    /// surfaces such as the internal effect surface without its token).
    #[error("service unavailable: {0}")]
    Unavailable(String),
    /// A fail-policy import chunk was rejected transactionally (TASKLIST
    /// D2): nothing from that chunk was written; the full per-row report
    /// rides the 409 so clients can correct and retry without re-upload.
    #[error("import chunk rejected by conflict_policy=fail")]
    ImportChunkRejected {
        report: crate::routes::imports::RunReportResponse,
    },
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
        ApiError::Validation(e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, reason, message, extra) = match &self {
            ApiError::NotFound => (
                StatusCode::NOT_FOUND,
                "not_found",
                "not found".to_string(),
                None,
            ),
            ApiError::RlsSparse => (
                StatusCode::NOT_FOUND,
                "rls_sparse",
                "mutation target not visible in scope".to_string(),
                None,
            ),
            ApiError::Unauthorized(m) => {
                (StatusCode::UNAUTHORIZED, "unauthorized", m.clone(), None)
            }
            ApiError::Forbidden(m) => (StatusCode::FORBIDDEN, "forbidden", m.clone(), None),
            ApiError::Validation(m) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation_failed",
                m.clone(),
                None,
            ),
            ApiError::Conflict(m) => (StatusCode::CONFLICT, "conflict", m.clone(), None),
            ApiError::VersionConflict { expected, provided } => (
                StatusCode::CONFLICT,
                "version_conflict",
                format!("version conflict: entity moved from version {provided} to {expected}"),
                Some(json!({ "expected": expected, "provided": provided })),
            ),
            ApiError::IdempotencyKeyReused => (
                StatusCode::CONFLICT,
                "idempotency_key_reused",
                "idempotency key reused with a different request".to_string(),
                None,
            ),
            ApiError::ChecklistBlocked { missing } => (
                StatusCode::CONFLICT,
                "pre_close_checklist_blocked",
                "pre-close checklist incomplete".to_string(),
                Some(json!({ "missing": missing })),
            ),
            ApiError::DispatchMoveConflict {
                index,
                appointment_id,
                blocker_title,
            } => (
                StatusCode::CONFLICT,
                "dispatch_move_conflict",
                "dispatch move conflicts with existing booking".to_string(),
                Some(json!({
                    "index": index,
                    "appointment_id": appointment_id,
                    "blocker_title": blocker_title,
                })),
            ),
            ApiError::ImportChunkRejected { report } => (
                StatusCode::CONFLICT,
                "import_chunk_rejected",
                "import chunk rejected by conflict_policy=fail".to_string(),
                Some(json!({ "report": report })),
            ),
            ApiError::Unavailable(m) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "service_unavailable",
                m.clone(),
                None,
            ),
            ApiError::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "internal error".to_string(),
                None,
            ),
        };
        let mut error = json!({ "code": status.as_u16(), "reason": reason, "message": message });
        if let Some(extra) = extra {
            if let (Some(base), Some(ext)) = (error.as_object_mut(), extra.as_object()) {
                for (k, v) in ext {
                    base.insert(k.clone(), v.clone());
                }
            }
        }
        (status, Json(json!({ "error": error }))).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
