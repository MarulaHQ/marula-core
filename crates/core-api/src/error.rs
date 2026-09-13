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
//! `validation_failed`, `conflict`,
//! `org_slug_taken`, `edge_source_unknown`, `edge_assertion_invalid`, `edge_identity_unlinked`, `user_email_taken`, `role_name_taken`,
//! `sku_taken`, `quote_number_taken`, `stage_name_taken`,
//! `category_name_taken`, `settings_key_taken`, `alert_source_ref_taken`,
//! `already_seeded`, `version_conflict`, `idempotency_key_reused`,
//! `pre_close_checklist_blocked`, `dispatch_move_conflict`,
//! `import_chunk_rejected`,
//! `migration_cursor_mismatch`, `migration_chunk_reused`, `rate_limited`,
//! `task_dependencies_unsatisfied`, `undo_reverts_only`,
//! `service_unavailable`, `internal_error`.
//!
//! The vocabulary above IS the machine-readable reason-code registry
//! (Wave 13 lane 1): every error this API emits carries one of these
//! codes, and nothing else identifiable crosses the wire — no constraint
//! or index names, no SQL text, no serde/type names, no stack traces.
//! Unknown database constraint violations are logged server-side with
//! full detail and surface to clients as the generic `conflict` code;
//! framework rejections (JSON/query extractors, method-not-allowed) are
//! sanitized into the envelope by [`typed_rejections`].

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
    /// Typed 409 for a board move whose current status's SEMANTIC category
    /// has no counterpart on the target board (Wave 11 lane 3): carries
    /// every target-board status (id/name/category, deterministic order)
    /// so the client can render the pick list and retry with an explicit
    /// `status_id` — no second round-trip to discover the options.
    #[error("no status on the target board matches the current semantic category")]
    BoardMoveStatusConflict {
        /// Board the ticket was headed to.
        target_board_id: Uuid,
        /// The ticket's current status id (on the source board).
        from_status_id: Uuid,
        /// Semantic category that had no counterpart on the target board.
        from_category: String,
        /// Every status on the target board, sort order first.
        target_statuses: Vec<crate::routes::taxonomy::TargetStatusOption>,
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
    /// Typed 409 for a completion attempt whose finish-to-start
    /// dependencies are not all completed yet (TASKLIST Phase 3 projects
    /// line): names the task and the blocking dependency ids so the client
    /// can render exactly what is outstanding.
    #[error("task dependencies not satisfied")]
    TaskDependenciesUnsatisfied {
        /// The task that was denied completion.
        task_id: Uuid,
        /// Dependency ids still not completed.
        blocking: Vec<Uuid>,
    },
    /// Typed 409 for an IF7 whole-record revert against a CLOSED record:
    /// closing is the undo checkpoint, so only explicit, actioned
    /// per-field reverts (a named `fields` list) are accepted — never a
    /// silent rewrite of the whole record past the close.
    #[error("record is closed: only explicit per-field reverts are permitted")]
    UndoRevertsOnly,
    /// Typed 409 for an org-slug collision at registration (Wave 13 lane
    /// 1): carries a suggested available slug so the client can offer
    /// one-click accept instead of an opaque 409. The raw constraint name
    /// (`orgs_slug_key`) never crosses the wire.
    #[error("that workspace address is already taken")]
    SlugTaken {
        /// A currently-available alternative slug, when one could be
        /// derived cheaply at write time.
        suggestion: Option<String>,
    },
    /// Typed 409 for a unique-constraint collision with a KNOWN, typed
    /// meaning (the constraint registry below): `kind` is the machine
    /// reason code, `message` the fixed human copy. Unknown constraints
    /// never reach this variant — they fall through to a sanitized
    /// generic conflict with the detail logged server-side only.
    #[error("{message}")]
    UniqueConflict {
            /// Typed 401 for a federation (bundle) login refusal (Wave 13 lane
    /// 7): auth failures carry a registry reason code, not prose to
    /// parse. `reason` is the machine code clients branch on;
    /// `message` is fixed human copy. Verification internals (why a
    /// signature/issuer/audience check failed) are logged server-side
    /// and never cross the wire — all invalid assertions collapse to
    /// the single `edge_assertion_invalid` code so attackers cannot
    /// fingerprint which check tripped.
    #[error("{message}")]
    AuthDenied {
        /// Machine-readable reason code (e.g. `edge_assertion_invalid`).
        reason: &'static str,
        /// Fixed human-readable message for the client.
        message: &'static str,
    },
/// Machine-readable reason code (e.g. `user_email_taken`).
        kind: &'static str,
        /// Fixed human-readable message for the client.
        message: &'static str,
    },
    /// Typed 409 for a dev-seed attempt against an already-seeded demo
    /// org (Wave 13 lane 1): the reason code (not the prose) is what the
    /// ops page branches on for the "force reseed" second click.
    #[error("demo org already seeded")]
    AlreadySeeded,
    /// Typed 409 for a migration connector chunk pushed out of position
    /// (TASKLIST D3): the checkpoint cursor expects a different index. The
    /// tool resumes from `expected` (GET the session view) — nothing was
    /// written.
    #[error("migration chunk cursor mismatch: expected chunk {expected}, got {provided}")]
    MigrationCursorMismatch {
        /// The chunk index the checkpoint actually expects next.
        expected: i64,
        /// The index the caller presented.
        provided: i64,
    },
    /// Typed 409 when an already-committed migration chunk is re-sent with
    /// a DIFFERENT payload (TASKLIST D3). Same index + same payload is a
    /// free replay of the stored report; drifting content under one index
    /// is a client bug we refuse to guess at.
    #[error("migration chunk already committed with different content")]
    MigrationChunkReused {
        /// The offending chunk index.
        chunk_index: i64,
    },
    /// Typed 429: the per-key rate limit for the migration connector
    /// surface is exhausted for this window. Carries the seconds until the
    /// window resets (also emitted as the Retry-After header).
    #[error("rate limit exceeded; retry after {retry_after_secs}s")]
    RateLimited {
        /// Seconds until the current fixed window resets.
        retry_after_secs: i64,
    },
    #[error("internal error")]
    Internal(#[from] anyhow::Error),
}

/// Maps a Postgres unique-violation constraint name onto its typed
/// (reason code, human message) pair. This table is the ONLY place a
/// constraint name is interpreted, and the name itself never reaches a
/// response: unknown entries fall through to the sanitized generic
/// conflict so a new constraint degrades to `conflict`, never to a leak.
fn map_unique_constraint(constraint: &str) -> Option<(&'static str, &'static str)> {
    match constraint {
        "orgs_slug_key" => Some(("org_slug_taken", "that workspace address is already taken")),
        "users_org_id_email_key" => Some((
            "user_email_taken",
            "an account with this email already exists",
        )),
        "roles_org_id_name_key" => {
            Some(("role_name_taken", "a role with this name already exists"))
        }
        "uq_catalog_items_sku" => Some(("sku_taken", "an item with this SKU already exists")),
        "quotes_number_key" => Some((
            "quote_number_taken",
            "a quote with this number already exists",
        )),
        "pipeline_stages_name_key" => {
            Some(("stage_name_taken", "a stage with this name already exists"))
        }
        "uq_ticket_categories_root_name" | "uq_ticket_categories_child_name" => Some((
            "category_name_taken",
            "a category with this name already exists",
        )),
        "settings_key_unique" => Some((
            "settings_key_taken",
            "a setting with this key already exists",
        )),
        "idx_alert_sources_ref" => Some((
            "alert_source_ref_taken",
            "an alert source with this ref already exists",
        )),
        // Wave 13 lane 7 (identity federation).
        "oidc_providers_org_id_name_key" => Some((
            "oidc_provider_name_taken",
            "an OIDC provider with this name already exists",
        )),
        "edge_assertion_configs_org_id_name_key" => Some((
            "edge_source_name_taken",
            "an edge-assertion source with this name already exists",
        )),
        "federation_links_source_kind_source_id_subject_key" => Some((
            "federation_link_taken",
            "this federated identity is already linked to an account",
        )),
        "kb_embed_providers_org_id_slug_key" => Some((
            "kb_embed_provider_slug_taken",
            "an embed provider with this slug already exists",
        )),
        "kb_embeds_org_id_article_id_url_key" => Some((
            "kb_embed_url_taken",
            "this embed url is already attached to the article",
        )),
        _ => None,
    }
}

/// Decides how a unique violation surfaces: known constraints produce a
/// fully typed error, unknown ones emit the sanitized generic conflict
/// (the name is logged server-side only).
fn unique_violation(e: &sqlx::Error) -> Option<ApiError> {
    let db = e.as_database_error()?;
    let constraint = db.constraint()?;
    Some(match map_unique_constraint(constraint) {
        Some((kind, message)) => {
            tracing::warn!(
                constraint,
                reason = kind,
                "unique violation mapped to typed reason"
            );
            ApiError::UniqueConflict { kind, message }
        }
        None => {
            tracing::error!(
                constraint,
                "unique violation on unmapped constraint; sending sanitized generic conflict"
            );
            ApiError::Conflict(
                "this value conflicts with an existing record; adjust it and retry".into(),
            )
        }
    })
}

impl From<core_db::DbError> for ApiError {
    fn from(e: core_db::DbError) -> Self {
        match &e {
            core_db::DbError::NotFound => ApiError::NotFound,
            core_db::DbError::Conflict(msg) => ApiError::Conflict(msg.clone()),
            core_db::DbError::Sql(sqlx::Error::RowNotFound) => ApiError::NotFound,
            core_db::DbError::Sql(err @ sqlx::Error::Database(_)) => unique_violation(err)
                .unwrap_or_else(|| {
                    tracing::error!(error = %e, "database error");
                    ApiError::Internal(anyhow::anyhow!(e.to_string()))
                }),
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
            ApiError::BoardMoveStatusConflict {
                target_board_id,
                from_status_id,
                from_category,
                target_statuses,
            } => (
                StatusCode::CONFLICT,
                "board_move_status_conflict",
                "no status on the target board matches the current semantic category".to_string(),
                Some(json!({
                    "target_board_id": target_board_id,
                    "from_status_id": from_status_id,
                    "from_category": from_category,
                    "target_statuses": target_statuses,
                })),
            ),
            ApiError::ImportChunkRejected { report } => (
                StatusCode::CONFLICT,
                "import_chunk_rejected",
                "import chunk rejected by conflict_policy=fail".to_string(),
                Some(json!({ "report": report })),
            ),
            ApiError::TaskDependenciesUnsatisfied { task_id, blocking } => (
                StatusCode::CONFLICT,
                "task_dependencies_unsatisfied",
                "task dependencies not satisfied".to_string(),
                Some(json!({ "task_id": task_id, "blocking": blocking })),
            ),
            ApiError::UniqueConflict { kind, message } => {
                (StatusCode::CONFLICT, *kind, message.to_string(), None)
            }
            ApiError::SlugTaken { suggestion } => (
                StatusCode::CONFLICT,
                "org_slug_taken",
                "that workspace address is already taken".to_string(),
                Some(json!({ "suggestion": suggestion })),
            ),
            ApiError::AlreadySeeded => (
                StatusCode::CONFLICT,
                "already_seeded",
                "demo org already seeded; POST /api/dev/seed?force=1 to wipe and reseed"
                    .to_string(),
                None,
            ),
            ApiError::UndoRevertsOnly => (
                StatusCode::CONFLICT,
                "undo_reverts_only",
                "record is closed: only explicit per-field reverts are permitted".to_string(),
                None,
            ),
            ApiError::MigrationCursorMismatch { expected, provided } => (
                StatusCode::CONFLICT,
                "migration_cursor_mismatch",
                format!(
                    "migration chunk cursor mismatch: expected chunk {expected}, got {provided}"
                ),
                Some(json!({ "expected": expected, "provided": provided })),
            ),
            ApiError::MigrationChunkReused { chunk_index } => (
                StatusCode::CONFLICT,
                "migration_chunk_reused",
                "migration chunk already committed with different content".to_string(),
                Some(json!({ "chunk_index": chunk_index })),
            ),
            ApiError::RateLimited { retry_after_secs } => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                format!("rate limit exceeded; retry after {retry_after_secs}s"),
                Some(json!({ "retry_after_secs": retry_after_secs })),
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
        let mut response = (status, Json(json!({ "error": error }))).into_response();
        if let ApiError::RateLimited { retry_after_secs } = &self {
            if let Ok(value) = retry_after_secs.to_string().parse() {
                response
                    .headers_mut()
                    .insert(axum::http::header::RETRY_AFTER, value);
            }
        }
        response
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

/// Middleware: rewrites framework rejection bodies into the IF3 envelope.
///
/// axum's extractors and method router reject with `text/plain` bodies
/// carrying raw serde diagnostics (type names, serde paths, line/column)
/// — an internal-leak path across all 200+ handler argument sites (Wave
/// 13 lane 1). This layer, mounted over the whole router in
/// `build_router`, maps every such response onto the typed envelope and
/// keeps the raw text server-side. A deliberate no-op for everything
/// else: JSON envelope responses pass through untouched, and the
/// empty-body unmatched-404 sentinel (api_versioning/dev_surface
/// existence checks) has no content-type and also passes through.
pub async fn typed_rejections(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::header::CONTENT_TYPE;

    let res = next.run(req).await;
    let status = res.status();
    if !status.is_client_error() && !status.is_server_error() {
        return res;
    }
    let is_text_rejection = res
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("text/plain"));
    if !is_text_rejection {
        return res;
    }

    let (_parts, body) = res.into_parts();
    let raw = axum::body::to_bytes(body, 64 * 1024)
        .await
        .unwrap_or_default();
    // Server-side only: full framework/serde diagnostics for debugging.
    tracing::debug!(
        status = status.as_u16(),
        raw = %String::from_utf8_lossy(&raw),
        "text/plain rejection sanitized into the IF3 envelope"
    );
    let (reason, message): (&str, &str) = match status {
        StatusCode::NOT_FOUND => ("not_found", "not found"),
        StatusCode::UNAUTHORIZED => ("unauthorized", "unauthorized"),
        StatusCode::FORBIDDEN => ("forbidden", "forbidden"),
        StatusCode::BAD_REQUEST => (
            "validation_failed",
            "request is malformed or has the wrong shape",
        ),
        _ => (
            "validation_failed",
            "request is malformed or has the wrong shape",
        ),
    };
    (
        status,
        Json(json!({
            "error": { "code": status.as_u16(), "reason": reason, "message": message }
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;
    use serde_json::Value;

    async fn body_bytes(response: axum::response::Response) -> Vec<u8> {
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec()
    }

    /// The machine-readable reason-code registry: every variant must map
    /// to a distinct reason in the documented vocabulary. A new variant
    /// that forgets its code fails here, not in production. (The
    /// `import_chunk_rejected` variant is exercised by imports_flow tests.)
    #[tokio::test(flavor = "multi_thread")]
    async fn every_variant_carries_a_registry_reason() {
        let cases: Vec<(ApiError, &str)> = vec![
            (ApiError::NotFound, "not_found"),
            (ApiError::RlsSparse, "rls_sparse"),
            (ApiError::Unauthorized("x".into()), "unauthorized"),
            (ApiError::Forbidden("x".into()), "forbidden"),
            (ApiError::Validation("x".into()), "validation_failed"),
            (ApiError::Conflict("x".into()), "conflict"),
            (
                ApiError::VersionConflict {
                    expected: 2,
                    provided: 1,
                },
                "version_conflict",
            ),
            (ApiError::IdempotencyKeyReused, "idempotency_key_reused"),
            (
                ApiError::ChecklistBlocked { missing: vec![] },
                "pre_close_checklist_blocked",
            ),
            (
                ApiError::DispatchMoveConflict {
                    index: 0,
                    appointment_id: Uuid::now_v7(),
                    blocker_title: String::new(),
                },
                "dispatch_move_conflict",
            ),
            (
                ApiError::BoardMoveStatusConflict {
                    target_board_id: Uuid::now_v7(),
                    from_status_id: Uuid::now_v7(),
                    from_category: "work".into(),
                    target_statuses: vec![],
                },
                "board_move_status_conflict",
            ),
            (
                ApiError::TaskDependenciesUnsatisfied {
                    task_id: Uuid::now_v7(),
                    blocking: vec![],
                },
                "task_dependencies_unsatisfied",
            ),
            (ApiError::UndoRevertsOnly, "undo_reverts_only"),
            (ApiError::SlugTaken { suggestion: None }, "org_slug_taken"),
            (
                ApiError::UniqueConflict {
                    kind: "user_email_taken",
                    message: "x",
                },
                "user_email_taken",
            ),
            (ApiError::AlreadySeeded, "already_seeded"),
            (
                ApiError::MigrationCursorMismatch {
                    expected: 1,
                    provided: 2,
                },
                "migration_cursor_mismatch",
            ),
            (
                ApiError::MigrationChunkReused { chunk_index: 1 },
                "migration_chunk_reused",
            ),
            (
                ApiError::RateLimited {
                    retry_after_secs: 1,
                },
                "rate_limited",
            ),
            (ApiError::Unavailable("x".into()), "service_unavailable"),
            (
                ApiError::Internal(anyhow::anyhow!("boom")),
                "internal_error",
            ),
        ];
        let mut seen = std::collections::HashSet::new();
        for (err, expected) in cases {
            let bytes = body_bytes(err.into_response()).await;
            let v: Value = serde_json::from_slice(&bytes).expect("envelope is JSON");
            let reason = v["error"]["reason"].as_str().expect("reason key");
            assert_eq!(reason, expected, "wrong reason for {expected}");
            assert!(
                seen.insert(reason.to_string()),
                "duplicate reason code: {reason}"
            );
            assert!(v["error"]["message"].is_string(), "message key missing");
        }
    }

    /// The constraint registry: every known constraint maps to a distinct
    /// typed reason code, and none of the codes collide with each other
    /// or with the generic `conflict`.
    #[test]
    fn constraint_registry_is_typed_and_disjoint() {
        let known = [
            "orgs_slug_key",
            "users_org_id_email_key",
            "roles_org_id_name_key",
            "uq_catalog_items_sku",
            "quotes_number_key",
            "pipeline_stages_name_key",
            "uq_ticket_categories_root_name",
            "uq_ticket_categories_child_name",
            "settings_key_unique",
            "idx_alert_sources_ref",
        ];
        let mut seen = std::collections::HashSet::new();
        for constraint in known {
            let (kind, message) = map_unique_constraint(constraint)
                .unwrap_or_else(|| panic!("{constraint} unmapped"));
            assert_ne!(kind, "conflict");
            assert!(!message.is_empty());
            seen.insert(kind);
        }
        // The two taxonomy constraints deliberately SHARE one reason code
        // (`category_name_taken` — root-name and child-name collisions are
        // the same UX problem), so 10 constraints map to 9 distinct codes.
        assert_eq!(seen.len(), 9, "unexpected reason-code fan-out: {seen:?}");
        assert_eq!(
            map_unique_constraint("uq_ticket_categories_root_name").map(|(k, _)| k),
            map_unique_constraint("uq_ticket_categories_child_name").map(|(k, _)| k)
        );
        // Unknown constraints degrade, never map.
        assert_eq!(map_unique_constraint("some_future_constraint_key"), None);
        assert_eq!(map_unique_constraint(""), None);
    }

    /// The slug-taken envelope carries the `suggestion` extra key.
    #[tokio::test(flavor = "multi_thread")]
    async fn slug_taken_carries_suggestion() {
        let bytes = body_bytes(
            ApiError::SlugTaken {
                suggestion: Some("acme-2".into()),
            }
            .into_response(),
        )
        .await;
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["error"]["reason"], "org_slug_taken");
        assert_eq!(v["error"]["suggestion"], "acme-2");
    }

    /// A text/plain framework rejection is rewritten into the envelope
    /// with the raw serde/framework text kept server-side only.
    #[tokio::test(flavor = "multi_thread")]
    async fn typed_rejections_sanitize_text_plain_bodies() {
        use tower::ServiceExt;
        let app: axum::Router = axum::Router::new()
            .route(
                "/probe",
                axum::routing::post(|axum::Json(_req): axum::Json<serde_json::Value>| async {
                    "ok"
                }),
            )
            .layer(axum::middleware::from_fn(typed_rejections));
        let res = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/probe")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from("{\"not\":\"closed"))
                    .unwrap(),
            )
            .await
            .unwrap();
        // axum 0.8: malformed JSON (JsonSyntaxError) is 400 BAD_REQUEST;
        // a well-formed body with the wrong shape (JsonDataError) is 422.
        assert_eq!(res.status(), axum::http::StatusCode::BAD_REQUEST);
        let bytes = body_bytes(res).await;
        let v: Value = serde_json::from_slice(&bytes).expect("rejection is envelope JSON");
        assert_eq!(v["error"]["reason"], "validation_failed");
        assert_eq!(v["error"]["code"], 400);
        let text = String::from_utf8(bytes).unwrap();
        assert!(
            !text.contains("serde") && !text.contains("line ") && !text.contains("expected"),
            "serde diagnostics leaked into the response: {text}"
        );
    }

    /// Method-not-allowed rejections are EMPTY in axum (status-only, no
    /// body, no content-type): the middleware leaves them untouched —
    /// nothing to sanitize, and the empty-body sentinel stays intact.
    #[tokio::test(flavor = "multi_thread")]
    async fn method_not_allowed_stays_empty() {
        use tower::ServiceExt;
        let app: axum::Router = axum::Router::new()
            .route("/probe", axum::routing::post(|| async { "ok" }))
            .layer(axum::middleware::from_fn(typed_rejections));
        let res = app
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/probe")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), axum::http::StatusCode::METHOD_NOT_ALLOWED);
        assert!(
            res.headers()
                .get(axum::http::header::CONTENT_TYPE)
                .is_none(),
            "405 must stay content-type-free (no leak, sentinel intact)"
        );
    }
}
