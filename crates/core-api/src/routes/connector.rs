//! Migration connector API (TASKLIST D3, Wave 9 lane 6): the authenticated
//! bulk push/pull surface third-party migration tools integrate with.
//!
//! Purpose: a stable, machine-scoped HTTP contract over the D2 import
//! engine so a migration tool never hand-builds scripts against internals.
//!
//! **Push** (`/api/migration/push/...`): a session opens exactly one D2
//! import batch (1:1) and pushes chunks through
//! [`crate::routes::imports::commit_chunk`] — the ONE plan→commit
//! transaction the human and worker paths use, so provenance (audit
//! actor_kind "import"), row-level idempotency via stable external ids,
//! event emission, and the typed 409 `import_chunk_rejected` on
//! conflict_policy=fail are inherited, not reimplemented. What D3 adds is
//! POSITION: every committed chunk stamps a checkpoint row (migration
//! 0042) with the content fingerprint and the full per-row report, so a
//! multi-chunk run that dies mid-flight asks the session "where did I
//! stop?" (GET session) and resumes at `checkpoint.next_chunk`. An exact
//! re-send of a committed chunk replays the stored report for free (same
//! external ids ⇒ no double-apply); the same index with different content
//! is a typed 409 `migration_chunk_reused`; an out-of-position index is a
//! typed 409 `migration_cursor_mismatch` — chunks are sequential.
//!
//! **Pull** (`/migration/pull/{family}`): stateless cursor-paged exports
//! per entity family (the D2 `EntityKind` vocabulary). Rows come back as
//! the canonical import projections — the SAME field catalog the push side
//! diffs against — plus each row's stable `external_key` when one was
//! pushed, keyset-paged on the uuidv7 id so pages never skip or repeat
//! rows under concurrent writes.
//!
//! Gating: machine keys need the `migration:write` scope to push and
//! `migration:read` (or `write`) to pull — resolved into the permission
//! matrix by the x-api-key middleware and checked here like any scoped
//! surface. Humans must be org owner/admin. Every request is rate-limited
//! per principal (fixed one-minute window, `Retry-After` on 429).
//!
//! Non-purpose: no CSV templates (tools remap their own columns — saved
//! templates remain the operator surface), no async/worker queueing
//! (connector sessions are synchronous by design), no pull-side cursor
//! state (keyset cursors are stateless), and no checkpoint stamping inside
//! the commit transaction — the stamp happens AFTER a successful commit,
//! so a crash in between can only leave the checkpoint trailing the data,
//! which the engine's external-id idempotency makes safe to re-push.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::routes::imports::{ChunkRun, ChunkSummaryWire, RowsPayload};
use crate::routes::ApiGeneration;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    routing::{get, post},
    Json, Router,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine as _;
use core_domain::entities::{Actor, AuditEvent, Principal};
use core_domain::events::{
    types, ImportBatchFinished, ImportBatchStarted, MigrationPushSessionFinished,
    MigrationPushSessionStarted,
};
use core_domain::import_core::{
    board_projection, client_projection, contact_projection, enforce_limits, kb_article_projection,
    kb_category_projection, site_projection, ticket_projection, time_entry_projection,
    ConflictPolicy, EntityKind, RowReport,
};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Instant;
use utoipa::ToSchema;
use uuid::Uuid;

// --- rate limiting ----------------------------------------------------------------

/// Fixed rate-limit window: a generous per-principal budget resets every
/// minute (bulk tools push many chunks; interactive clients never get near
/// this). Counters are per-process — one API instance, one budget.
const RATE_WINDOW_SECS: u64 = 60;

struct RateWindow {
    started: Instant,
    count: u32,
}

static RATE_WINDOWS: OnceLock<std::sync::Mutex<HashMap<Uuid, RateWindow>>> = OnceLock::new();

/// Per-principal fixed-window limiter for the whole `/api/migration`
/// surface. `limit == 0` disables limiting (documented escape hatch); the
/// window resets every [`RATE_WINDOW_SECS`] and the breach carries the
/// seconds until it does (429 body + `Retry-After` header).
fn check_rate_limit(limit_per_min: u32, principal: Uuid) -> ApiResult<()> {
    if limit_per_min == 0 {
        return Ok(());
    }
    let buckets = RATE_WINDOWS.get_or_init(Default::default);
    let mut map = buckets
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let now = Instant::now();
    let window = map.entry(principal).or_insert(RateWindow {
        started: now,
        count: 0,
    });
    if now.duration_since(window.started).as_secs() >= RATE_WINDOW_SECS {
        window.started = now;
        window.count = 0;
    }
    window.count += 1;
    if window.count > limit_per_min {
        let elapsed = now.duration_since(window.started).as_secs();
        let retry_after = RATE_WINDOW_SECS.saturating_sub(elapsed).max(1);
        return Err(ApiError::RateLimited {
            retry_after_secs: i64::try_from(retry_after).unwrap_or(i64::MAX),
        });
    }
    Ok(())
}

// --- gating / provenance ---------------------------------------------------------

/// Humans must be org owner/admin; machine keys need the matrix grants
/// their `migration:*` scopes resolve onto (push = Clients create+edit —
/// the D2 machine contract; pull = view). The migration scopes are broad
/// data grants on purpose; creating a key with them is an explicit admin
/// act (ManageApiKeys), audited like every other key.
fn require_migration_gate(principal: &Principal, via_api_key: bool, write: bool) -> ApiResult<()> {
    if via_api_key {
        require_permission(
            principal,
            Permission::Matrix {
                module: Module::Clients,
                action: Action::View,
            },
        )?;
        if write {
            require_permission(
                principal,
                Permission::Matrix {
                    module: Module::Clients,
                    action: Action::Create,
                },
            )?;
            require_permission(
                principal,
                Permission::Matrix {
                    module: Module::Clients,
                    action: Action::Edit,
                },
            )?;
        }
        Ok(())
    } else if principal.is_owner_or_admin() {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "migration connector is limited to org admins".into(),
        ))
    }
}

/// Audit descriptor for actor_kind "import" (mirrors the D2 surface).
fn actor_descriptor(principal: &Principal, via_api_key: bool) -> String {
    if via_api_key {
        format!("api-key:{}", principal.user.id)
    } else {
        format!("user:{}", principal.user.id)
    }
}

/// Audit row with actor_kind "import": connector-driven bulk change keeps
/// the same provenance doctrine as the D2 surface.
fn audit_event(
    org_id: Uuid,
    descriptor: &str,
    action: &str,
    entity_id: Option<&str>,
    detail: serde_json::Value,
) -> AuditEvent {
    AuditEvent {
        id: Uuid::now_v7(),
        org_id,
        actor: Actor::Import(descriptor.to_string()),
        action: action.to_string(),
        entity_type: Some("migration_push_session".into()),
        entity_id: entity_id.map(str::to_string),
        detail,
        ip: None,
        occurred_at: chrono::Utc::now(),
    }
}

// --- wire types ---------------------------------------------------------------------

/// The checkpoint cursor of a push session: what the server has committed
/// and what it expects next. `last_committed_chunk` is `-1` on a fresh
/// session.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct PushCheckpoint {
    /// Highest committed chunk index; -1 before the first chunk.
    pub last_committed_chunk: i64,
    /// The chunk index the next POST must carry.
    pub next_chunk: i64,
}

fn checkpoint_of(committed: &[i32]) -> PushCheckpoint {
    let last = committed.iter().copied().max().map(i64::from).unwrap_or(-1);
    PushCheckpoint {
        last_committed_chunk: last,
        next_chunk: last + 1,
    }
}

/// POST /api/migration/push/sessions body.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreatePushSessionRequest {
    pub entity_kind: EntityKind,
    pub conflict_policy: ConflictPolicy,
    /// Operator-facing run label.
    #[serde(default)]
    pub label: Option<String>,
    /// Free-form note about the third-party source system.
    pub source_note: Option<String>,
}

/// 201 body: the session id, its D2 batch, and the starting cursor.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PushSessionCreated {
    pub session_id: Uuid,
    pub batch_id: Uuid,
    pub entity_kind: EntityKind,
    pub conflict_policy: ConflictPolicy,
    pub label: String,
    pub checkpoint: PushCheckpoint,
}

/// One committed chunk's position in the resume view.
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
pub struct CommittedChunkRef {
    pub chunk_index: i64,
    #[schema(value_type = String)]
    pub committed_at: chrono::DateTime<chrono::Utc>,
}

/// GET /api/migration/push/sessions/{id} — the resume view a tool asks
/// after a crash: where the checkpoint stands and which chunks committed.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PushSessionView {
    pub session_id: Uuid,
    pub batch_id: Uuid,
    pub entity_kind: EntityKind,
    pub conflict_policy: ConflictPolicy,
    pub label: String,
    pub source_note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
    #[schema(value_type = String)]
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub checkpoint: PushCheckpoint,
    pub committed_chunks: Vec<CommittedChunkRef>,
}

/// u32-friendly mirror of the per-chunk summary so the generated SDK gets
/// plain integers without overflow caveats (same shape as D2's
/// `ChunkSummaryWire`, but also deserialize for replay).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct PushChunkSummary {
    pub total: u32,
    pub ok: u32,
    pub would_update: u32,
    pub would_skip: u32,
    pub failed: u32,
}

impl From<crate::routes::imports::ChunkSummaryWire> for PushChunkSummary {
    fn from(s: ChunkSummaryWire) -> Self {
        Self {
            total: s.total,
            ok: s.ok,
            would_update: s.would_update,
            would_skip: s.would_skip,
            failed: s.failed,
        }
    }
}

/// The per-row report of one chunk — the D2 contract shape, kept
/// Serialize+Deserialize so a committed chunk's stored copy replays
/// faithfully on an exact re-send.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct PushChunkReport {
    pub batch_id: Uuid,
    pub dry_run: bool,
    pub entity_kind: EntityKind,
    pub conflict_policy: ConflictPolicy,
    pub summary: PushChunkSummary,
    pub rows: Vec<RowReport>,
}

impl From<crate::routes::imports::RunReportResponse> for PushChunkReport {
    fn from(r: crate::routes::imports::RunReportResponse) -> Self {
        Self {
            batch_id: r.batch_id,
            dry_run: r.dry_run,
            entity_kind: r.entity_kind,
            conflict_policy: r.conflict_policy,
            summary: r.summary.into(),
            rows: r.rows,
        }
    }
}

/// POST /api/migration/push/sessions/{id}/chunks body.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct PushChunkRequest {
    /// Position of this chunk in the run. Must be the checkpoint's
    /// `next_chunk`; a lower index replays (same payload) or 409s
    /// (`migration_chunk_reused`, different payload); a higher index 409s
    /// (`migration_cursor_mismatch`) — chunks are sequential.
    pub chunk_index: i64,
    #[serde(flatten)]
    pub rows: RowsPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PushChunkResponse {
    pub session_id: Uuid,
    pub chunk_index: i64,
    /// `true` when an exact re-send replayed the stored report without
    /// touching data (retry after a lost response is free).
    pub replay: bool,
    pub report: PushChunkReport,
    pub checkpoint: PushCheckpoint,
    /// Cumulative batch counters after this chunk.
    pub rows_total: i64,
    pub rows_created: i64,
    pub rows_updated: i64,
    pub rows_skipped: i64,
    pub rows_failed: i64,
}

/// POST /api/migration/push/sessions/{id}/finish — closes the session and
/// its D2 batch; returns the cumulative run totals.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PushSessionFinishedResponse {
    pub session_id: Uuid,
    pub batch_id: Uuid,
    pub entity_kind: EntityKind,
    pub conflict_policy: ConflictPolicy,
    pub chunks_committed: i64,
    pub rows_total: i32,
    pub rows_created: i32,
    pub rows_updated: i32,
    pub rows_skipped: i32,
    pub rows_failed: i32,
    pub checkpoint: PushCheckpoint,
}

/// GET /api/migration/pull/{family} query.
#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct PullQuery {
    /// Opaque keyset cursor from a previous page's `next_cursor` (the last
    /// row id). Absent = start of the family.
    pub cursor: Option<String>,
    /// Page size, clamped to [1, 200] (default 100).
    pub limit: Option<i64>,
}

/// One exported row: internal id, stable source key when the row was ever
/// pushed under one, and the canonical projection (the SAME field catalog
/// the push side diffs against).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PullItem {
    pub id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_key: Option<String>,
    #[schema(value_type = Object)]
    pub projection: serde_json::Value,
}

/// GET /api/migration/pull/{family} envelope.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PullPage {
    pub family: EntityKind,
    pub items: Vec<PullItem>,
    /// Pass back as `cursor`; absent when exhausted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

// --- router ---------------------------------------------------------------------

/// The migration connector slice of the `/api` surface, built per
/// generation so both mounts register identical routes through one merge.
pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(
            &format!("{base}/migration/push/sessions"),
            post(create_session),
        )
        .route(
            &format!("{base}/migration/push/sessions/{{session_id}}"),
            get(get_session),
        )
        .route(
            &format!("{base}/migration/push/sessions/{{session_id}}/chunks"),
            post(post_chunk).layer(axum::extract::DefaultBodyLimit::max(
                core_domain::import_core::MAX_CHUNK_BYTES,
            )),
        )
        .route(
            &format!("{base}/migration/push/sessions/{{session_id}}/finish"),
            post(finish_session),
        )
        .route(&format!("{base}/migration/pull/{{family}}"), get(pull))
}

// --- handlers ---------------------------------------------------------------------

/// POST /api/migration/push/sessions — opens a connector push session over
/// a fresh D2 import batch. One session targets exactly one entity kind;
/// cross-family ordering is the tool's run script's job (the D2 import
/// order: client → site → contact → board → ticket → time entry).
#[utoipa::path(operation_id = "migration_create_push_session", post, path = "/api/migration/push/sessions", tag = "migration",
    security(("bearer" = [])),
    request_body = CreatePushSessionRequest,
    responses((status = 201, body = PushSessionCreated)))]
pub async fn create_session(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreatePushSessionRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<PushSessionCreated>)> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_migration_gate(&authed.principal, authed.via_api_key, true)?;
    check_rate_limit(
        state.config.migration_rate_limit_per_min,
        authed.principal.user.id,
    )?;
    let org_id = authed.principal.user.org_id;
    let driver = authed.principal.user.id;
    let descriptor = actor_descriptor(&authed.principal, authed.via_api_key);

    let label = req.label.clone().unwrap_or_else(|| "unnamed".into());
    let created = org_tx::with_org_tx(
        &state,
        org_id,
        authed.principal.user.id,
        |mut ctx| async move {
            let batch_id = Uuid::now_v7();
            let session_id = Uuid::now_v7();
            let created_by_kind = if authed.via_api_key {
                "api_key"
            } else {
                "user"
            };

            core_db::imports::insert_batch(
                &mut *ctx.tx,
                &core_db::imports::ImportBatch {
                    id: batch_id,
                    org_id,
                    label: label.clone(),
                    source_note: None,
                    entity_kind: req.entity_kind.as_str().to_string(),
                    conflict_policy: req.conflict_policy.as_str().to_string(),
                    dry_run: false,
                    rows_total: 0,
                    rows_created: 0,
                    rows_updated: 0,
                    rows_skipped: 0,
                    rows_failed: 0,
                    created_by: driver,
                    created_by_kind: created_by_kind.to_string(),
                    started_at: chrono::Utc::now(),
                    finished_at: None,
                    staged_chunks: 0,
                    queued_at: None,
                },
            )
            .await?;
            core_db::connector::insert_push_session(
                &mut *ctx.tx,
                &core_db::connector::PushSession {
                    id: session_id,
                    org_id,
                    batch_id,
                    label: label.clone(),
                    source_note: req.source_note.clone(),
                    created_by: driver,
                    created_by_kind: created_by_kind.to_string(),
                    created_at: chrono::Utc::now(),
                    finished_at: None,
                },
            )
            .await?;

            ctx.emit(
                types::IMPORT_BATCH_STARTED,
                "import_batch",
                Some(batch_id.to_string().as_str()),
                &serde_json::to_value(ImportBatchStarted {
                    batch_id,
                    entity_kind: req.entity_kind.as_str().to_string(),
                    conflict_policy: req.conflict_policy.as_str().into(),
                    label: label.clone(),
                })
                .unwrap_or_else(|_| serde_json::json!({})),
            )
            .await?;
            ctx.emit(
                types::MIGRATION_PUSH_SESSION_STARTED,
                "migration_push_session",
                Some(session_id.to_string().as_str()),
                &serde_json::to_value(MigrationPushSessionStarted {
                    session_id,
                    batch_id,
                    entity_kind: req.entity_kind.as_str().to_string(),
                    conflict_policy: req.conflict_policy.as_str().into(),
                    label: label.clone(),
                })
                .unwrap_or_else(|_| serde_json::json!({})),
            )
            .await?;

            core_db::audit::insert(
                &mut *ctx.tx,
                &audit_event(
                    org_id,
                    &descriptor,
                    "migration.session_started",
                    Some(session_id.to_string().as_str()),
                    serde_json::json!({
                        "batch_id": batch_id,
                        "label": label,
                        "entity_kind": req.entity_kind.as_str(),
                        "conflict_policy": req.conflict_policy.as_str(),
                        "source_note": req.source_note,
                    }),
                ),
            )
            .await?;

            Ok((
                ctx,
                PushSessionCreated {
                    session_id,
                    batch_id,
                    entity_kind: req.entity_kind,
                    conflict_policy: req.conflict_policy,
                    label,
                    checkpoint: PushCheckpoint {
                        last_committed_chunk: -1,
                        next_chunk: 0,
                    },
                },
            ))
        },
    )
    .await?;
    Ok((axum::http::StatusCode::CREATED, Json(created)))
}

/// GET /api/migration/push/sessions/{id} — the resume view: where the
/// checkpoint stands and which chunks committed.
#[utoipa::path(operation_id = "migration_get_push_session", get, path = "/api/migration/push/sessions/{session_id}", tag = "migration",
    security(("bearer" = [])),
    params(("session_id" = Uuid, Path)),
    responses((status = 200, body = PushSessionView)))]
pub async fn get_session(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(session_id): Path<Uuid>,
) -> ApiResult<Json<PushSessionView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_migration_gate(&authed.principal, authed.via_api_key, true)?;
    let org_id = authed.principal.user.org_id;

    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let session = core_db::connector::get_push_session(&mut *tx, org_id, session_id)
        .await
        .map_err(|_| ApiError::NotFound)?;
    let batch = core_db::imports::get_batch(&mut *tx, org_id, session.batch_id)
        .await
        .map_err(|_| ApiError::NotFound)?;
    let chunks = core_db::connector::list_push_chunks(&mut *tx, org_id, session_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(PushSessionView {
        session_id: session.id,
        batch_id: session.batch_id,
        entity_kind: batch.entity_kind.parse().unwrap_or(EntityKind::Client),
        conflict_policy: batch
            .conflict_policy
            .parse()
            .unwrap_or(ConflictPolicy::Skip),
        label: session.label,
        source_note: session.source_note,
        finished_at: session.finished_at,
        created_at: session.created_at,
        checkpoint: checkpoint_of(&chunks.iter().map(|c| c.chunk_index).collect::<Vec<_>>()),
        committed_chunks: chunks
            .iter()
            .map(|c| CommittedChunkRef {
                chunk_index: i64::from(c.chunk_index),
                committed_at: c.created_at,
            })
            .collect(),
    }))
}

/// sha256 over the canonical chunk content (format + raw payload + index).
/// The fingerprint IS the replay contract: same index + same content is a
/// free replay; drifting content under a committed index is refused.
fn chunk_fingerprint(req: &PushChunkRequest) -> ApiResult<String> {
    let mut hasher = Sha256::new();
    hasher.update(b"meridian-d3-chunk");
    hasher.update(req.chunk_index.to_le_bytes());
    match req.rows.format {
        core_domain::import_core::RowFormat::Json => {
            let value = req.rows.json_rows.as_ref().ok_or_else(|| {
                ApiError::Validation("json_rows is required for format=json".into())
            })?;
            hasher.update(b"json");
            let bytes = serde_json::to_vec(value)
                .map_err(|e| ApiError::Validation(format!("unserializable body: {e}")))?;
            hasher.update(bytes);
        }
        core_domain::import_core::RowFormat::Csv => {
            let csv = req
                .rows
                .csv
                .as_ref()
                .ok_or_else(|| ApiError::Validation("csv is required for format=csv".into()))?;
            hasher.update(b"csv");
            hasher.update(csv.as_bytes());
        }
    }
    Ok(hex::encode(hasher.finalize()))
}

/// POST /api/migration/push/sessions/{id}/chunks — plans then executes ONE
/// chunk through the D2 commit path, then stamps the checkpoint. A
/// fail-policy chunk with any conflict persists nothing and returns the
/// typed 409 with the full per-row report (checkpoint untouched — fix the
/// rows and re-send the same index).
#[utoipa::path(operation_id = "migration_push_chunk", post, path = "/api/migration/push/sessions/{session_id}/chunks", tag = "migration",
    security(("bearer" = [])),
    params(("session_id" = Uuid, Path)),
    request_body = PushChunkRequest,
    responses((status = 200, body = PushChunkResponse)))]
pub async fn post_chunk(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(session_id): Path<Uuid>,
    Json(req): Json<PushChunkRequest>,
) -> ApiResult<Json<PushChunkResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_migration_gate(&authed.principal, authed.via_api_key, true)?;
    check_rate_limit(
        state.config.migration_rate_limit_per_min,
        authed.principal.user.id,
    )?;
    let org_id = authed.principal.user.org_id;
    let driver = authed.principal.user.id;
    let descriptor = actor_descriptor(&authed.principal, authed.via_api_key);

    if req.chunk_index < 0 || req.chunk_index > i64::from(i32::MAX) {
        return Err(ApiError::Validation(
            "chunk_index must be 0..=2147483647".into(),
        ));
    }
    let fingerprint = chunk_fingerprint(&req)?;

    // Cursor decision against the CURRENT checkpoint, outside the commit
    // transaction (reads only; the engine owns the write transaction).
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, driver)
        .await
        .map_err(ApiError::from)?;
    let session = core_db::connector::get_push_session(&mut *tx, org_id, session_id)
        .await
        .map_err(|_| ApiError::NotFound)?;
    if session.finished_at.is_some() {
        return Err(ApiError::Conflict("push session already finished".into()));
    }
    let chunks = core_db::connector::list_push_chunks(&mut *tx, org_id, session_id).await?;
    let checkpoint = checkpoint_of(&chunks.iter().map(|c| c.chunk_index).collect::<Vec<_>>());

    if req.chunk_index < checkpoint.next_chunk {
        // Already committed: replay the stored report on an exact match,
        // refuse drifting content — the index is a replay contract.
        let stored = chunks
            .iter()
            .find(|c| i64::from(c.chunk_index) == req.chunk_index)
            .expect("index below checkpoint exists in the chunk list");
        if stored.fingerprint == fingerprint {
            let report: PushChunkReport =
                serde_json::from_value(stored.summary.clone()).map_err(|e| {
                    ApiError::Internal(anyhow::anyhow!("stored checkpoint summary: {e}"))
                })?;
            let batch = core_db::imports::get_batch(&mut *tx, org_id, session.batch_id)
                .await
                .map_err(|_| ApiError::NotFound)?;
            tx.commit().await.map_err(ApiError::from)?;
            return Ok(Json(PushChunkResponse {
                session_id,
                chunk_index: req.chunk_index,
                replay: true,
                report,
                checkpoint,
                rows_total: i64::from(batch.rows_total),
                rows_created: i64::from(batch.rows_created),
                rows_updated: i64::from(batch.rows_updated),
                rows_skipped: i64::from(batch.rows_skipped),
                rows_failed: i64::from(batch.rows_failed),
            }));
        }
        return Err(ApiError::MigrationChunkReused {
            chunk_index: req.chunk_index,
        });
    }
    if req.chunk_index > checkpoint.next_chunk {
        return Err(ApiError::MigrationCursorMismatch {
            expected: checkpoint.next_chunk,
            provided: req.chunk_index,
        });
    }
    tx.commit().await.map_err(ApiError::from)?;

    // Fresh commit: decode + engine limits, then the ONE D2 commit path.
    let (raw_rows, byte_len) = req.rows.decode()?;
    enforce_limits(raw_rows.len(), byte_len)?;
    let report = crate::routes::imports::commit_chunk(ChunkRun {
        state: &state,
        org_id,
        user_id: driver,
        batch_id: session.batch_id,
        descriptor: &descriptor,
        template_id: None,
        via_worker: false,
        raw_rows,
    })
    .await?;

    // Stamp the checkpoint AFTER a successful commit. A crash between the
    // two can only leave the checkpoint trailing the data — the engine's
    // external-id idempotency makes that re-push safe.
    let report_view: PushChunkReport = report.into();
    let summary_json = serde_json::to_value(&report_view)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("summary encode: {e}")))?;
    org_tx::with_org_tx(&state, org_id, driver, |mut ctx| async move {
        core_db::connector::insert_push_chunk(
            &mut *ctx.tx,
            &core_db::connector::PushChunk {
                id: Uuid::now_v7(),
                org_id,
                session_id,
                chunk_index: i32::try_from(req.chunk_index)
                    .map_err(|_| ApiError::Validation("chunk_index out of range".into()))?,
                fingerprint: fingerprint.clone(),
                summary: summary_json.clone(),
                created_at: chrono::Utc::now(),
            },
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &audit_event(
                org_id,
                &descriptor,
                "migration.chunk_committed",
                Some(session_id.to_string().as_str()),
                serde_json::json!({
                    "batch_id": session.batch_id,
                    "chunk_index": req.chunk_index,
                    "rows_total": report_view.summary.total,
                    "rows_ok": report_view.summary.ok,
                    "rows_updated": report_view.summary.would_update,
                    "rows_skipped": report_view.summary.would_skip,
                    "rows_failed": report_view.summary.failed,
                }),
            ),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;

    // Cumulative counters ride the batch view.
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, driver)
        .await
        .map_err(ApiError::from)?;
    let batch = core_db::imports::get_batch(&mut *tx, org_id, session.batch_id)
        .await
        .map_err(|_| ApiError::NotFound)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(PushChunkResponse {
        session_id,
        chunk_index: req.chunk_index,
        replay: false,
        report: report_view,
        checkpoint: PushCheckpoint {
            last_committed_chunk: req.chunk_index,
            next_chunk: req.chunk_index + 1,
        },
        rows_total: i64::from(batch.rows_total),
        rows_created: i64::from(batch.rows_created),
        rows_updated: i64::from(batch.rows_updated),
        rows_skipped: i64::from(batch.rows_skipped),
        rows_failed: i64::from(batch.rows_failed),
    }))
}

/// POST /api/migration/push/sessions/{id}/finish — closes the session and
/// its D2 batch, emitting BOTH the engine lifecycle event
/// (`import.batch_finished`) and the connector-level
/// `migration.push_session_finished`.
#[utoipa::path(operation_id = "migration_finish_push_session", post, path = "/api/migration/push/sessions/{session_id}/finish", tag = "migration",
    security(("bearer" = [])),
    params(("session_id" = Uuid, Path)),
    responses((status = 200, body = PushSessionFinishedResponse)))]
pub async fn finish_session(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(session_id): Path<Uuid>,
) -> ApiResult<Json<PushSessionFinishedResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_migration_gate(&authed.principal, authed.via_api_key, true)?;
    check_rate_limit(
        state.config.migration_rate_limit_per_min,
        authed.principal.user.id,
    )?;
    let org_id = authed.principal.user.org_id;
    let descriptor = actor_descriptor(&authed.principal, authed.via_api_key);

    let finished = org_tx::with_org_tx(
        &state,
        org_id,
        authed.principal.user.id,
        |mut ctx| async move {
            let session = core_db::connector::get_push_session(&mut *ctx.tx, org_id, session_id)
                .await
                .map_err(|_| ApiError::NotFound)?;
            if session.finished_at.is_some() {
                return Err(ApiError::Conflict("push session already finished".into()));
            }
            if !core_db::connector::mark_push_session_finished(&mut *ctx.tx, org_id, session_id)
                .await?
            {
                return Err(ApiError::Conflict("push session already finished".into()));
            }
            // Close the paired D2 batch (1:1 by construction) so the commit
            // path refuses any further chunks.
            let batch =
                core_db::imports::mark_batch_finished(&mut *ctx.tx, org_id, session.batch_id)
                    .await
                    .map_err(|_| ApiError::NotFound)?;
            let chunks =
                core_db::connector::list_push_chunks(&mut *ctx.tx, org_id, session_id).await?;

            ctx.emit(
                types::IMPORT_BATCH_FINISHED,
                "import_batch",
                Some(batch.id.to_string().as_str()),
                &serde_json::to_value(ImportBatchFinished {
                    batch_id: session.batch_id,
                    entity_kind: batch.entity_kind.clone(),
                    rows_total: batch.rows_total,
                    rows_created: batch.rows_created,
                    rows_updated: batch.rows_updated,
                    rows_skipped: batch.rows_skipped,
                    rows_failed: batch.rows_failed,
                })
                .unwrap_or_else(|_| serde_json::json!({})),
            )
            .await?;
            ctx.emit(
                types::MIGRATION_PUSH_SESSION_FINISHED,
                "migration_push_session",
                Some(session.id.to_string().as_str()),
                &serde_json::to_value(MigrationPushSessionFinished {
                    session_id,
                    batch_id: session.batch_id,
                    entity_kind: batch.entity_kind.clone(),
                    chunks_committed: i32::try_from(chunks.len()).unwrap_or(i32::MAX),
                    rows_total: batch.rows_total,
                    rows_created: batch.rows_created,
                    rows_updated: batch.rows_updated,
                    rows_skipped: batch.rows_skipped,
                    rows_failed: batch.rows_failed,
                })
                .unwrap_or_else(|_| serde_json::json!({})),
            )
            .await?;

            core_db::audit::insert(
                &mut *ctx.tx,
                &audit_event(
                    org_id,
                    &descriptor,
                    "migration.session_finished",
                    Some(session_id.to_string().as_str()),
                    serde_json::json!({
                        "batch_id": session.batch_id,
                        "label": session.label,
                        "chunks_committed": chunks.len(),
                        "rows_total": batch.rows_total,
                        "rows_created": batch.rows_created,
                        "rows_updated": batch.rows_updated,
                        "rows_skipped": batch.rows_skipped,
                        "rows_failed": batch.rows_failed,
                    }),
                ),
            )
            .await?;

            let committed: Vec<i32> = chunks.iter().map(|c| c.chunk_index).collect();
            Ok((
                ctx,
                PushSessionFinishedResponse {
                    session_id,
                    batch_id: session.batch_id,
                    entity_kind: batch.entity_kind.parse().unwrap_or(EntityKind::Client),
                    conflict_policy: batch
                        .conflict_policy
                        .parse()
                        .unwrap_or(ConflictPolicy::Skip),
                    chunks_committed: i64::try_from(chunks.len()).unwrap_or(i64::MAX),
                    rows_total: batch.rows_total,
                    rows_created: batch.rows_created,
                    rows_updated: batch.rows_updated,
                    rows_skipped: batch.rows_skipped,
                    rows_failed: batch.rows_failed,
                    checkpoint: checkpoint_of(&committed),
                },
            ))
        },
    )
    .await?;
    Ok(Json(finished))
}

// --- bulk pull ---------------------------------------------------------------------

fn encode_cursor(id: Uuid) -> String {
    B64.encode(id.as_bytes())
}

fn decode_cursor(raw: &str) -> ApiResult<Uuid> {
    B64.decode(raw)
        .ok()
        .and_then(|bytes| Uuid::from_slice(&bytes).ok())
        .ok_or_else(|| ApiError::Validation("malformed cursor".into()))
}

/// GET /api/migration/pull/{family} — stateless cursor-paged export of one
/// entity family: internal id + stable external key + the canonical
/// import projection. Keyset-paged on the uuidv7 id (creation order), so
/// pages never skip or repeat rows under concurrent writes.
#[utoipa::path(operation_id = "migration_pull", get, path = "/api/migration/pull/{family}", tag = "migration",
    security(("bearer" = [])),
    params(("family" = String, Path, description = "Entity family: client, site, contact, board, ticket, time_entry, kb_category, kb_article"),
           PullQuery),
    responses((status = 200, body = PullPage)))]
pub async fn pull(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(family): Path<String>,
    Query(q): Query<PullQuery>,
) -> ApiResult<Json<PullPage>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_migration_gate(&authed.principal, authed.via_api_key, false)?;
    check_rate_limit(
        state.config.migration_rate_limit_per_min,
        authed.principal.user.id,
    )?;
    let org_id = authed.principal.user.org_id;

    let kind: EntityKind = family.parse().map_err(|_| {
        ApiError::Validation(format!(
            "unknown family {family:?}; expected one of client, site, contact, board, \
             ticket, time_entry, kb_category, kb_article"
        ))
    })?;
    let limit = q.limit.unwrap_or(100).clamp(1, 200);
    let after = match q.cursor.as_deref() {
        None => None,
        Some(raw) => Some(decode_cursor(raw)?),
    };

    // Read-model export under the caller's org scope: RLS applies to every
    // page read, exactly like any other read surface.
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let ids =
        core_db::connector::page_entity_ids(&mut *tx, kind.as_str(), org_id, after, limit + 1)
            .await?;
    let has_more = ids.len() as i64 > limit;
    let page_ids: Vec<Uuid> = ids.into_iter().take(limit as usize).collect();
    let keys: HashMap<Uuid, String> =
        core_db::connector::external_key_map(&mut *tx, kind.as_str(), org_id, &page_ids)
            .await?
            .into_iter()
            .collect();

    let mut items: Vec<PullItem> = Vec::with_capacity(page_ids.len());
    match kind {
        EntityKind::Client => {
            for client in core_db::clients::get_clients_by_ids(&mut *tx, org_id, &page_ids).await? {
                items.push(PullItem {
                    id: client.id,
                    external_key: keys.get(&client.id).cloned(),
                    projection: serde_json::to_value(client_projection(&client))
                        .unwrap_or_else(|_| serde_json::json!({})),
                });
            }
        }
        EntityKind::Site => {
            for site in core_db::clients::get_sites_by_ids(&mut *tx, org_id, &page_ids).await? {
                items.push(PullItem {
                    id: site.id,
                    external_key: keys.get(&site.id).cloned(),
                    projection: serde_json::to_value(site_projection(&site))
                        .unwrap_or_else(|_| serde_json::json!({})),
                });
            }
        }
        EntityKind::Contact => {
            for contact in
                core_db::clients::get_contacts_by_ids(&mut *tx, org_id, &page_ids).await?
            {
                items.push(PullItem {
                    id: contact.id,
                    external_key: keys.get(&contact.id).cloned(),
                    projection: serde_json::to_value(contact_projection(&contact))
                        .unwrap_or_else(|_| serde_json::json!({})),
                });
            }
        }
        EntityKind::Board => {
            for id in &page_ids {
                let board = core_db::boards::get_board(&mut *tx, org_id, *id).await?;
                items.push(PullItem {
                    id: board.id,
                    external_key: keys.get(&board.id).cloned(),
                    projection: serde_json::to_value(board_projection(&board))
                        .unwrap_or_else(|_| serde_json::json!({})),
                });
            }
        }
        EntityKind::Ticket => {
            for id in &page_ids {
                let ticket = core_db::tickets::get_ticket(&mut *tx, org_id, *id).await?;
                items.push(PullItem {
                    id: ticket.id,
                    external_key: keys.get(&ticket.id).cloned(),
                    projection: serde_json::to_value(ticket_projection(&ticket))
                        .unwrap_or_else(|_| serde_json::json!({})),
                });
            }
        }
        EntityKind::TimeEntry => {
            for id in &page_ids {
                let entry = core_db::timers::get_time_entry(&mut *tx, org_id, *id).await?;
                items.push(PullItem {
                    id: entry.id,
                    external_key: keys.get(&entry.id).cloned(),
                    projection: serde_json::to_value(time_entry_projection(&entry))
                        .unwrap_or_else(|_| serde_json::json!({})),
                });
            }
        }
        EntityKind::KbCategory => {
            for id in &page_ids {
                let category = core_db::kb::get_category(&mut *tx, org_id, *id).await?;
                items.push(PullItem {
                    id: category.id,
                    external_key: keys.get(&category.id).cloned(),
                    projection: serde_json::to_value(kb_category_projection(&category))
                        .unwrap_or_else(|_| serde_json::json!({})),
                });
            }
        }
        EntityKind::KbArticle => {
            for id in &page_ids {
                let article = core_db::kb::get_article(&mut *tx, org_id, *id).await?;
                items.push(PullItem {
                    id: article.id,
                    external_key: keys.get(&article.id).cloned(),
                    projection: serde_json::to_value(kb_article_projection(&article))
                        .unwrap_or_else(|_| serde_json::json!({})),
                });
            }
        }
    }
    // The id page is already ascending; per-id fetches must not disturb
    // that order or the keyset cursor would repeat/skip rows.
    items.sort_by_key(|item| item.id);
    tx.commit().await.map_err(ApiError::from)?;

    let next_cursor = if has_more {
        page_ids.last().map(|id| encode_cursor(*id))
    } else {
        None
    };
    Ok(Json(PullPage {
        family: kind,
        items,
        next_cursor,
    }))
}
