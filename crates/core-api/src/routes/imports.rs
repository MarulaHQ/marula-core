//! Import framework endpoints (TASKLIST D2): dry-run validation reports,
//! batch lifecycle, and chunked commits for the client/site/contact family.
//!
//! Purpose: the HTTP face of `core_domain::import_core`. Planning is pure;
//! this layer persists plans transactionally per chunk, resolves external
//! keys through `external_ids`, emits the SAME domain events ordinary
//! mutations would (imports never bypass event emission — every insert or
//! update goes through the canonical core-db mutators inside an outbox-bound
//! org_tx), and stamps batch-lifecycle provenance as audit actor_kind
//! "import" with the driving identity embedded.
//!
//! Gating doctrine: humans must be org owner/admin; machine `x-api-key`
//! principals need BOTH `clients:create` and `clients:edit` scopes on top of
//! the middleware prefix allowance — bulk pushes are the documented D3
//! machine flow. One batch targets exactly one entity kind; cross-family
//! ordering (clients → sites → contacts across successive batches) is the
//! caller's run script's job.
//!
//! Non-purpose: no CSV template downloads, no saved field-mapping templates
//! per source system, no worker-based async giant batches (deferred D2
//! follow-ups); not the D3 conformance harness.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx::{self, OrgTx};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use core_domain::entities::{Actor, AuditEvent, Principal};
use core_domain::events::{
    types, ClientCreated, ClientUpdated, ContactCreated, ContactUpdated, ImportBatchFinished,
    ImportBatchStarted, SiteCreated, SiteUpdated,
};
use core_domain::import_core::{
    build_specs, chunk_is_executable, enforce_limits, parse_csv_chunk, parse_json_chunk,
    plan_chunk, ChunkPlan, ChunkSummary, CommitAction, ConflictPolicy, EntityKind, ImportSpec,
    KeyMappingLookup, ParentLookup, ProjectionLookup, RawRow, RowFormat, RowReport, SpecResult,
};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use utoipa::ToSchema;
use uuid::Uuid;

// --- wire types ---------------------------------------------------------------

/// The encoded row source shared by dry-run and chunk posts.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct RowsPayload {
    pub format: RowFormat,
    /// Row objects as a JSON array of flat objects (`format=json`). Nulls
    /// behave exactly like absent keys.
    #[schema(value_type = Object)]
    pub json_rows: Option<serde_json::Value>,
    /// Full CSV text including header row (`format=csv`). UTF-8, BOM
    /// tolerated; RFC 4180 quoting supported.
    pub csv: Option<String>,
}

impl RowsPayload {
    fn decode(&self) -> ApiResult<(Vec<RawRow>, usize)> {
        match self.format {
            RowFormat::Json => {
                let value = self.json_rows.as_ref().ok_or_else(|| {
                    ApiError::Validation("json_rows is required for format=json".into())
                })?;
                let byte_len = serde_json::to_vec(value)
                    .map_err(|e| ApiError::Validation(format!("unserializable body: {e}")))?
                    .len();
                let rows = parse_json_chunk(value).map_err(ApiError::from)?;
                Ok((rows, byte_len))
            }
            RowFormat::Csv => {
                let csv = self
                    .csv
                    .as_ref()
                    .ok_or_else(|| ApiError::Validation("csv is required for format=csv".into()))?;
                let byte_len = csv.len();
                let rows = parse_csv_chunk(csv).map_err(ApiError::from)?;
                Ok((rows, byte_len))
            }
        }
    }
}

impl From<core_domain::import_core::ImportError> for ApiError {
    fn from(e: core_domain::import_core::ImportError) -> Self {
        use core_domain::import_core::{MAX_CHUNK_BYTES, MAX_ROWS_PER_CHUNK};
        match e {
            core_domain::import_core::ImportError::TooManyRows(n) => {
                ApiError::Validation(format!("row limit exceeded: {n} > {MAX_ROWS_PER_CHUNK}"))
            }
            core_domain::import_core::ImportError::TooManyBytes(n) => ApiError::Validation(
                format!("payload limit exceeded: {n} bytes > {MAX_CHUNK_BYTES}"),
            ),
            other => ApiError::Validation(other.to_string()),
        }
    }
}

/// POST /api/imports/dry-run request: validate a full payload and report
/// what each row WOULD do before any data write. The probe is recorded as a
/// finished dry-run batch for operator history.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct DryRunRequest {
    pub entity_kind: EntityKind,
    pub conflict_policy: ConflictPolicy,
    /// Free-form run label recorded with the dry-run history entry.
    pub label: Option<String>,
    #[serde(flatten)]
    pub rows: RowsPayload,
}

/// POST /api/imports/batches — opens a committed-run span. Chunks then land
/// through `/chunks` until `/finish` closes the batch.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateBatchRequest {
    pub entity_kind: EntityKind,
    pub conflict_policy: ConflictPolicy,
    pub label: Option<String>,
    pub source_note: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ChunkRequest {
    #[serde(flatten)]
    pub rows: RowsPayload,
}

/// Per-row fate report — the D2 contract shape
/// (`row_index → errors[] | ok | would_skip | would_update`).
#[derive(Debug, Clone, Serialize, PartialEq, ToSchema)]
pub struct RunReportResponse {
    #[schema(value_type = uuid::Uuid)]
    pub batch_id: Uuid,
    pub dry_run: bool,
    pub entity_kind: EntityKind,
    pub conflict_policy: ConflictPolicy,
    pub summary: ChunkSummaryWire,
    pub rows: Vec<RowReport>,
}

impl RunReportResponse {
    fn of(batch: RunBatchRef, plan: &ChunkPlan) -> Self {
        RunReportResponse {
            batch_id: batch.id,
            dry_run: batch.dry_run,
            entity_kind: batch.kind,
            conflict_policy: batch.policy,
            summary: plan.summary.into(),
            rows: plan.reports.clone(),
        }
    }
}

struct RunBatchRef {
    id: Uuid,
    dry_run: bool,
    kind: EntityKind,
    policy: ConflictPolicy,
}

/// u32-friendly mirror of [`core_domain::import_core::ChunkSummary`] so the
/// generated SDK gets plain integers without overflow caveats.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, ToSchema)]
pub struct ChunkSummaryWire {
    pub total: u32,
    pub ok: u32,
    pub would_update: u32,
    pub would_skip: u32,
    pub failed: u32,
}

impl From<ChunkSummary> for ChunkSummaryWire {
    fn from(s: ChunkSummary) -> Self {
        Self {
            total: s.total,
            ok: s.ok,
            would_update: s.would_update,
            would_skip: s.would_skip,
            failed: s.failed,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BatchCreatedResponse {
    #[schema(value_type = uuid::Uuid)]
    pub batch_id: Uuid,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BatchViewResponse {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub org_id: Uuid,
    pub label: String,
    pub source_note: Option<String>,
    pub entity_kind: EntityKind,
    pub conflict_policy: ConflictPolicy,
    pub dry_run: bool,
    pub rows_total: i32,
    pub rows_created: i32,
    pub rows_updated: i32,
    pub rows_skipped: i32,
    pub rows_failed: i32,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<core_db::imports::ImportBatch> for BatchViewResponse {
    fn from(b: core_db::imports::ImportBatch) -> Self {
        // Entity-kind/policy strings round-trip through their engine enums;
        // unknown future values cannot exist while 0027's CHECKs hold.
        let kind = b.entity_kind.parse().unwrap_or(EntityKind::Client);
        let policy = b.conflict_policy.parse().unwrap_or(ConflictPolicy::Skip);
        BatchViewResponse {
            id: b.id,
            org_id: b.org_id,
            label: b.label,
            source_note: b.source_note,
            entity_kind: kind,
            conflict_policy: policy,
            dry_run: b.dry_run,
            rows_total: b.rows_total,
            rows_created: b.rows_created,
            rows_updated: b.rows_updated,
            rows_skipped: b.rows_skipped,
            rows_failed: b.rows_failed,
            started_at: b.started_at,
            finished_at: b.finished_at,
        }
    }
}

// --- gating / provenance --------------------------------------------------------

fn require_import_gate(principal: &Principal, via_api_key: bool) -> ApiResult<()> {
    if via_api_key {
        // Scoped machine push: BOTH mutate-scoped grants are required.
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
        )
    } else if principal.is_owner_or_admin() {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "imports are limited to org admins".into(),
        ))
    }
}

/// Audit descriptor for actor_kind "import": names WHO drove the run even
/// though the import actor deliberately sits one level above row edits.
fn actor_descriptor(principal: &Principal, via_api_key: bool) -> String {
    if via_api_key {
        format!("api-key:{}", principal.user.id)
    } else {
        format!("user:{}", principal.user.id)
    }
}

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
        entity_type: Some("import_batch".into()),
        entity_id: entity_id.map(str::to_string),
        detail,
        ip: None,
        occurred_at: chrono::Utc::now(),
    }
}

// --- planning snapshot -------------------------------------------------------------

/// Pre-fetched, chunk-local resolution state handed to the pure engine as
/// sync closures. Built INSIDE the caller's org-scoped transaction so RLS
/// applies to every read.
struct Snapshot {
    /// Resolved-and-verified parents: mappings whose backing rows STILL exist
    /// (a stale mapping for a deleted client is reported, never trusted).
    parents: HashMap<(EntityKind, String), Uuid>,
    /// Self-kind key → internal id.
    mapping: HashMap<String, Uuid>,
    /// Internal id → canonical projection of the stored row.
    projections: HashMap<Uuid, BTreeMap<String, String>>,
}

fn collect_keys(specs: &[SpecResult], pick: impl Fn(&ImportSpec) -> Option<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for spec in specs {
        if let Ok(parsed) = &spec.spec {
            if let Some(k) = pick(parsed) {
                if !k.is_empty() && seen.insert(k.clone()) {
                    out.push(k);
                }
            }
        }
    }
    out
}

/// Loads mappings for `kind` keeping only keys whose backing entities still
/// exist — existence IS part of parent resolution, not an afterthought.
async fn load_parents(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    org_id: Uuid,
    kind: EntityKind,
    keys: &[String],
    snap: &mut Snapshot,
) -> ApiResult<()> {
    let owned = core_db::imports::find_mappings(&mut **tx, org_id, kind.as_str(), keys).await?;
    let ids: Vec<Uuid> = owned.iter().map(|(_, id)| *id).collect();
    let alive: Vec<Uuid> = match kind {
        EntityKind::Client => core_db::clients::get_clients_by_ids(&mut **tx, org_id, &ids)
            .await?
            .into_iter()
            .map(|c| c.id)
            .collect(),
        EntityKind::Site => core_db::clients::get_sites_by_ids(&mut **tx, org_id, &ids)
            .await?
            .into_iter()
            .map(|s| s.id)
            .collect(),
        EntityKind::Contact => core_db::clients::get_contacts_by_ids(&mut **tx, org_id, &ids)
            .await?
            .into_iter()
            .map(|c| c.id)
            .collect(),
    };
    for (key, id) in owned {
        if alive.contains(&id) {
            snap.parents.insert((kind, key), id);
        }
    }
    Ok(())
}

fn dedupe(keys: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    keys.into_iter()
        .filter(|k| !k.is_empty() && seen.insert(k.clone()))
        .collect()
}

async fn build_snapshot(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    org_id: Uuid,
    kind: EntityKind,
    specs: &[SpecResult],
) -> ApiResult<Snapshot> {
    use core_domain::import_core::{client_projection, contact_projection, site_projection};

    let mut snap = Snapshot {
        parents: HashMap::new(),
        mapping: HashMap::new(),
        projections: HashMap::new(),
    };

    match kind {
        EntityKind::Client => {}
        EntityKind::Site => {
            let client_keys = collect_keys(specs, |spec| match spec {
                ImportSpec::Site(site) => Some(site.client_key.clone()),
                _ => None,
            });
            load_parents(
                tx,
                org_id,
                EntityKind::Client,
                &dedupe(client_keys),
                &mut snap,
            )
            .await?;
        }
        EntityKind::Contact => {
            let client_keys = collect_keys(specs, |spec| match spec {
                ImportSpec::Contact(contact) => Some(contact.client_key.clone()),
                _ => None,
            });
            let site_keys = collect_keys(specs, |spec| match spec {
                ImportSpec::Contact(contact) => contact.site_key.clone(),
                _ => None,
            });
            load_parents(
                tx,
                org_id,
                EntityKind::Client,
                &dedupe(client_keys),
                &mut snap,
            )
            .await?;
            load_parents(tx, org_id, EntityKind::Site, &dedupe(site_keys), &mut snap).await?;
        }
    }

    let self_keys = collect_keys(specs, |spec| Some(spec.external_key().to_string()));
    for (key, id) in
        core_db::imports::find_mappings(&mut **tx, org_id, kind.as_str(), &self_keys).await?
    {
        snap.mapping.insert(key, id);
    }

    let ids: Vec<Uuid> = snap.mapping.values().copied().collect();
    let stored: Vec<(Uuid, BTreeMap<String, String>)> = match kind {
        EntityKind::Client => core_db::clients::get_clients_by_ids(&mut **tx, org_id, &ids)
            .await?
            .into_iter()
            .map(|c| (c.id, client_projection(&c)))
            .collect(),
        EntityKind::Site => core_db::clients::get_sites_by_ids(&mut **tx, org_id, &ids)
            .await?
            .into_iter()
            .map(|s| (s.id, site_projection(&s)))
            .collect(),
        EntityKind::Contact => core_db::clients::get_contacts_by_ids(&mut **tx, org_id, &ids)
            .await?
            .into_iter()
            .map(|c| (c.id, contact_projection(&c)))
            .collect(),
    };
    for (id, projection) in stored {
        snap.projections.insert(id, projection);
    }
    Ok(snap)
}

/// Builds the three sync lookups the pure planner consumes, borrowing the
/// pre-fetched snapshot. Named bindings so the coercion to `&dyn Fn` aliases
/// is explicit.
macro_rules! lookups {
    ($snap:expr, $p:ident, $m:ident, $j:ident) => {
        #[allow(clippy::redundant_closure)]
        let $p: ParentLookup<'_> =
            &|kind, key| $snap.parents.get(&(kind, key.to_string())).copied();
        let $m: KeyMappingLookup<'_> = &|key| $snap.mapping.get(key).copied();
        let $j: ProjectionLookup<'_> = &|id| $snap.projections.get(&id).cloned();
    };
}

fn summarize_counters(summary: ChunkSummary) -> (i32, i32, i32, i32, i32) {
    (
        summary.total as i32,
        summary.ok as i32,
        summary.would_update as i32,
        summary.would_skip as i32,
        summary.failed as i32,
    )
}

// --- handlers --------------------------------------------------------------------

/// POST /api/imports/dry-run
#[utoipa::path(operation_id = "imports_dry_run", post, path = "/api/imports/dry-run", tag = "imports",
    security(("bearer" = [])),
    request_body = DryRunRequest,
    responses((status = 200, body = RunReportResponse)))]
pub async fn dry_run(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<DryRunRequest>,
) -> ApiResult<Json<RunReportResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_import_gate(&authed.principal, authed.via_api_key)?;
    let org_id = authed.principal.user.org_id;
    let descriptor = actor_descriptor(&authed.principal, authed.via_api_key);

    let (raw_rows, byte_len) = req.rows.decode()?;
    enforce_limits(raw_rows.len(), byte_len)?;
    let kind = req.entity_kind;

    let response = org_tx::with_org_tx(
        &state,
        org_id,
        authed.principal.user.id,
        |mut ctx| async move {
            let specs = build_specs(kind, &raw_rows);
            let snap = build_snapshot(&mut ctx.tx, org_id, kind, &specs).await?;
            lookups!(snap, parent_lookup, mapping_lookup, projection_lookup);
            // Dry-run NEVER fails fast: operators deserve every finding at once.
            let plan = plan_chunk(
                &specs,
                req.conflict_policy,
                parent_lookup,
                mapping_lookup,
                projection_lookup,
                false,
            );

            // Record the probe (finished immediately; counters mirror the
            // report). No outbox event: a dry run performs no data mutation for
            // downstream consumers to react to; audit keeps the provenance.
            let batch = fresh_batch(
                org_id,
                req.label.clone().unwrap_or_else(|| "dry-run".into()),
                kind,
                req.conflict_policy,
                true,
                ctx.user_id,
                authed.via_api_key,
            );
            core_db::imports::insert_batch(&mut *ctx.tx, &batch).await?;
            let (total, created, updated, skipped, failed) = summarize_counters(plan.summary);
            core_db::imports::add_chunk_counters(
                &mut *ctx.tx,
                org_id,
                batch.id,
                total,
                created,
                updated,
                skipped,
                failed,
            )
            .await?;
            // A probe closes itself immediately.
            core_db::imports::mark_batch_finished(&mut *ctx.tx, org_id, batch.id).await?;
            core_db::audit::insert(
                &mut *ctx.tx,
                &audit_event(
                    org_id,
                    &descriptor,
                    "import.dry_run",
                    Some(&batch.id.to_string()),
                    serde_json::json!({
                        "label": batch.label,
                        "entity_kind": kind.as_str(),
                        "conflict_policy": req.conflict_policy.as_str(),
                        "summary": plan.summary,
                    }),
                ),
            )
            .await?;

            let view = RunBatchRef {
                id: batch.id,
                dry_run: true,
                kind,
                policy: req.conflict_policy,
            };
            Ok((ctx, Json(RunReportResponse::of(view, &plan))))
        },
    )
    .await?;
    Ok(response)
}

/// POST /api/imports/batches
#[utoipa::path(operation_id = "imports_create_batch", post, path = "/api/imports/batches", tag = "imports",
    security(("bearer" = [])),
    request_body = CreateBatchRequest,
    responses((status = 200, body = BatchCreatedResponse)))]
pub async fn create_batch(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateBatchRequest>,
) -> ApiResult<Json<BatchCreatedResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_import_gate(&authed.principal, authed.via_api_key)?;
    let org_id = authed.principal.user.org_id;
    let descriptor = actor_descriptor(&authed.principal, authed.via_api_key);

    let label = req.label.clone().unwrap_or_else(|| "unnamed".into());
    let batch = org_tx::with_org_tx(
        &state,
        org_id,
        authed.principal.user.id,
        |mut ctx| async move {
            let record = fresh_batch(
                org_id,
                label.clone(),
                req.entity_kind,
                req.conflict_policy,
                false,
                ctx.user_id,
                authed.via_api_key,
            );
            core_db::imports::insert_batch(&mut *ctx.tx, &record).await?;

            ctx.emit(
                types::IMPORT_BATCH_STARTED,
                "import_batch",
                Some(record.id.to_string().as_str()),
                &serde_json::to_value(ImportBatchStarted {
                    batch_id: record.id,
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
                    "import.batch_started",
                    Some(&record.id.to_string()),
                    serde_json::json!({
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
                BatchCreatedResponse {
                    batch_id: record.id,
                    label: record.label,
                },
            ))
        },
    )
    .await?;
    Ok(Json(batch))
}

/// POST /api/imports/batches/{id}/chunks — plans then executes ONE chunk
/// atomically. A fail-policy chunk with any conflict persists nothing and
/// returns 409 with the full per-row report.
#[utoipa::path(operation_id = "imports_post_chunk", post, path = "/api/imports/batches/{batch_id}/chunks", tag = "imports",
    security(("bearer" = [])),
    params(("batch_id" = Uuid, Path)),
    request_body = ChunkRequest,
    responses((status = 200, body = RunReportResponse)))]
pub async fn post_chunk(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(batch_id): Path<Uuid>,
    Json(req): Json<ChunkRequest>,
) -> ApiResult<Json<RunReportResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_import_gate(&authed.principal, authed.via_api_key)?;
    let org_id = authed.principal.user.org_id;
    let descriptor = actor_descriptor(&authed.principal, authed.via_api_key);

    let (raw_rows, byte_len) = req.rows.decode()?;
    enforce_limits(raw_rows.len(), byte_len)?;

    let result = org_tx::with_org_tx(
        &state,
        org_id,
        authed.principal.user.id,
        |mut ctx| async move {
            let batch = core_db::imports::get_batch(&mut *ctx.tx, org_id, batch_id)
                .await
                .map_err(|_| ApiError::NotFound)?;
            if batch.dry_run {
                return Err(ApiError::Validation(
                    "cannot push chunks into a dry-run probe".into(),
                ));
            }
            if batch.finished_at.is_some() {
                return Err(ApiError::Conflict("batch already finished".into()));
            }
            let kind: EntityKind = batch.entity_kind.parse().map_err(|_| {
                ApiError::Validation(format!(
                    "unsupported entity_kind {:?} recorded on this deployment",
                    batch.entity_kind
                ))
            })?;
            let policy: ConflictPolicy = batch.conflict_policy.parse().map_err(|_| {
                ApiError::Validation(format!(
                    "unsupported conflict_policy {:?}",
                    batch.conflict_policy
                ))
            })?;

            let specs = build_specs(kind, &raw_rows);
            let snap = build_snapshot(&mut ctx.tx, org_id, kind, &specs).await?;
            lookups!(snap, parent_lookup, mapping_lookup, projection_lookup);
            let plan = plan_chunk(
                &specs,
                policy,
                parent_lookup,
                mapping_lookup,
                projection_lookup,
                policy == ConflictPolicy::Fail,
            );

            if policy == ConflictPolicy::Fail && !chunk_is_executable(&plan) {
                // Nothing executed yet; dropping ctx rolls back even the reads'
                // locks. The report rides a typed 409 so callers correct and
                // retry without re-uploading anything else.
                let view = RunBatchRef {
                    id: batch.id,
                    dry_run: false,
                    kind,
                    policy,
                };
                return Err(ApiError::ImportChunkRejected {
                    report: RunReportResponse::of(view, &plan),
                });
            }

            let mut created: i64 = 0;
            let mut updated: i64 = 0;
            for action in &plan.actions {
                match &action.action {
                    CommitAction::Insert(spec) => {
                        commit_insert(&mut ctx, org_id, batch.id, spec, &snap.parents).await?;
                        created += 1;
                    }
                    CommitAction::Update { internal_id, spec } => {
                        commit_update(&mut ctx, org_id, batch.id, internal_id, spec).await?;
                        updated += 1;
                    }
                }
            }
            let failed = plan.summary.failed as i64;
            let skipped = plan.summary.would_skip as i64;
            let total = plan.summary.total as i64;
            let bumped = core_db::imports::add_chunk_counters(
                &mut *ctx.tx,
                org_id,
                batch.id,
                total as i32,
                created as i32,
                updated as i32,
                skipped as i32,
                failed as i32,
            )
            .await?;
            if !bumped {
                return Err(ApiError::Conflict(
                    "batch finished while the chunk was in flight".into(),
                ));
            }
            core_db::audit::insert(
                &mut *ctx.tx,
                &audit_event(
                    org_id,
                    &descriptor,
                    "import.chunk_committed",
                    Some(&batch.id.to_string()),
                    serde_json::json!({
                        "rows_total": total,
                        "rows_created": created,
                        "rows_updated": updated,
                        "rows_skipped": skipped,
                        "rows_failed": failed,
                    }),
                ),
            )
            .await?;

            let view = RunBatchRef {
                id: batch.id,
                dry_run: false,
                kind,
                policy,
            };
            Ok((ctx, Json(RunReportResponse::of(view, &plan))))
        },
    )
    .await?;
    Ok(result)
}

/// POST /api/imports/batches/{id}/finish — closes the span, emits
/// `import.batch_finished`, and records cumulative counters in audit.
#[utoipa::path(operation_id = "imports_finish_batch", post, path = "/api/imports/batches/{batch_id}/finish", tag = "imports",
    security(("bearer" = [])),
    params(("batch_id" = Uuid, Path)),
    responses((status = 200, body = BatchViewResponse)))]
pub async fn finish_batch(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(batch_id): Path<Uuid>,
) -> ApiResult<Json<BatchViewResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_import_gate(&authed.principal, authed.via_api_key)?;
    let org_id = authed.principal.user.org_id;
    let descriptor = actor_descriptor(&authed.principal, authed.via_api_key);

    let batch = org_tx::with_org_tx(
        &state,
        org_id,
        authed.principal.user.id,
        |mut ctx| async move {
            let closed = core_db::imports::mark_batch_finished(&mut *ctx.tx, org_id, batch_id)
                .await
                .map_err(|_| ApiError::NotFound)?;

            ctx.emit(
                types::IMPORT_BATCH_FINISHED,
                "import_batch",
                Some(closed.id.to_string().as_str()),
                &serde_json::to_value(ImportBatchFinished {
                    batch_id: closed.id,
                    entity_kind: closed.entity_kind.clone(),
                    rows_total: closed.rows_total,
                    rows_created: closed.rows_created,
                    rows_updated: closed.rows_updated,
                    rows_skipped: closed.rows_skipped,
                    rows_failed: closed.rows_failed,
                })
                .unwrap_or_else(|_| serde_json::json!({})),
            )
            .await?;

            core_db::audit::insert(
                &mut *ctx.tx,
                &audit_event(
                    org_id,
                    &descriptor,
                    "import.batch_finished",
                    Some(&closed.id.to_string()),
                    serde_json::json!({
                        "label": closed.label,
                        "entity_kind": closed.entity_kind,
                        "rows_total": closed.rows_total,
                        "rows_created": closed.rows_created,
                        "rows_updated": closed.rows_updated,
                        "rows_skipped": closed.rows_skipped,
                        "rows_failed": closed.rows_failed,
                    }),
                ),
            )
            .await?;
            Ok((ctx, closed))
        },
    )
    .await?;
    Ok(Json(batch.into()))
}

/// GET /api/imports/batches/{id}
#[utoipa::path(operation_id = "imports_get_batch", get, path = "/api/imports/batches/{batch_id}", tag = "imports",
    security(("bearer" = [])),
    params(("batch_id" = Uuid, Path)),
    responses((status = 200, body = BatchViewResponse)))]
pub async fn get_batch(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(batch_id): Path<Uuid>,
) -> ApiResult<Json<BatchViewResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_import_gate(&authed.principal, authed.via_api_key)?;
    let batch = core_db::imports::get_batch(&state.pool, authed.principal.user.org_id, batch_id)
        .await
        .map_err(|_| ApiError::NotFound)?;
    Ok(Json(batch.into()))
}

/// GET /api/imports/batches
#[utoipa::path(operation_id = "imports_list_batches", get, path = "/api/imports/batches", tag = "imports",
    security(("bearer" = [])),
    responses((status = 200, body = [BatchViewResponse])))]
pub async fn list_batches(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<BatchViewResponse>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_import_gate(&authed.principal, authed.via_api_key)?;
    let batches =
        core_db::imports::list_batches(&state.pool, authed.principal.user.org_id, 100).await?;
    Ok(Json(batches.into_iter().map(Into::into).collect()))
}

// --- execution ----------------------------------------------------------------------

fn fresh_batch(
    org_id: Uuid,
    label: String,
    kind: EntityKind,
    policy: ConflictPolicy,
    dry_run: bool,
    driver_id: Uuid,
    via_api_key: bool,
) -> core_db::imports::ImportBatch {
    core_db::imports::ImportBatch {
        id: Uuid::now_v7(),
        org_id,
        label,
        source_note: None,
        entity_kind: kind.as_str().to_string(),
        conflict_policy: policy.as_str().to_string(),
        dry_run,
        rows_total: 0,
        rows_created: 0,
        rows_updated: 0,
        rows_skipped: 0,
        rows_failed: 0,
        created_by: driver_id,
        created_by_kind: if via_api_key { "api_key" } else { "user" }.to_string(),
        started_at: chrono::Utc::now(),
        finished_at: None,
    }
}

/// Inserts through the canonical core-db mutators, wires the external-id
/// mapping, and emits the SAME domain event any staff mutation emits.
#[allow(clippy::too_many_lines)]
async fn commit_insert(
    ctx: &mut OrgTx<'_>,
    org_id: Uuid,
    batch_id: Uuid,
    spec: &ImportSpec,
    parents: &HashMap<(EntityKind, String), Uuid>,
) -> ApiResult<()> {
    let key_row_id = Uuid::now_v7();
    match spec {
        ImportSpec::Client(client) => {
            let id = Uuid::now_v7();
            let row = core_db::clients::create_client(
                &mut *ctx.tx,
                org_id,
                id,
                &client.name,
                client.notes.as_deref(),
            )
            .await?;
            core_db::imports::upsert_mapping(
                &mut *ctx.tx,
                key_row_id,
                org_id,
                "client",
                &client.external_key,
                id,
                batch_id,
            )
            .await?;
            ctx.emit(
                types::CLIENT_CREATED,
                "client",
                Some(row.id.to_string().as_str()),
                &serde_json::to_value(ClientCreated {
                    client_id: row.id,
                    name: row.name.clone(),
                })
                .unwrap_or_else(|_| serde_json::json!({})),
            )
            .await?;
        }
        ImportSpec::Site(site) => {
            let id = Uuid::now_v7();
            let client_id = resolve_parent(parents, EntityKind::Client, &site.client_key)?;
            let row = core_db::clients::create_site(
                &mut *ctx.tx,
                org_id,
                id,
                client_id,
                &site.name,
                site.address.as_deref(),
                site.city.as_deref(),
                site.region.as_deref(),
                site.postal_code.as_deref(),
                site.country.as_deref(),
                site.latitude,
                site.longitude,
                site.timezone.as_deref(),
            )
            .await?;
            core_db::imports::upsert_mapping(
                &mut *ctx.tx,
                key_row_id,
                org_id,
                "site",
                &site.external_key,
                id,
                batch_id,
            )
            .await?;
            ctx.emit(
                types::SITE_CREATED,
                "site",
                Some(row.id.to_string().as_str()),
                &serde_json::to_value(SiteCreated {
                    site_id: row.id,
                    client_id: row.client_id,
                    name: row.name.clone(),
                })
                .unwrap_or_else(|_| serde_json::json!({})),
            )
            .await?;
        }
        ImportSpec::Contact(contact) => {
            let id = Uuid::now_v7();
            let client_id = resolve_parent(parents, EntityKind::Client, &contact.client_key)?;
            let site_id = match &contact.site_key {
                Some(key) => Some(resolve_parent(parents, EntityKind::Site, key)?),
                None => None,
            };
            let row = core_db::clients::create_contact(
                &mut *ctx.tx,
                org_id,
                id,
                client_id,
                site_id,
                &contact.name,
                contact.email.as_deref(),
                contact.phone.as_deref(),
                contact.title.as_deref(),
                contact.is_primary,
            )
            .await?;
            core_db::imports::upsert_mapping(
                &mut *ctx.tx,
                key_row_id,
                org_id,
                "contact",
                &contact.external_key,
                id,
                batch_id,
            )
            .await?;
            ctx.emit(
                types::CONTACT_CREATED,
                "contact",
                Some(row.id.to_string().as_str()),
                &serde_json::to_value(ContactCreated {
                    contact_id: row.id,
                    client_id: row.client_id,
                    name: row.name.clone(),
                })
                .unwrap_or_else(|_| serde_json::json!({})),
            )
            .await?;
        }
    }
    Ok(())
}

fn resolve_parent(
    parents: &HashMap<(EntityKind, String), Uuid>,
    kind: EntityKind,
    key: &str,
) -> ApiResult<Uuid> {
    parents
        .get(&(kind, key.to_string()))
        .copied()
        .ok_or_else(|| {
            ApiError::Validation(format!(
                "parent {key:?} vanished between planning and commit; retry"
            ))
        })
}

/// Updates through the canonical core-db mutators. Parent links (client /
/// site ownership) are identity-set at creation and deliberately immutable
/// here — moving a child under a different parent is a re-parent operation,
/// not content drift, and never part of a bulk overwrite.
async fn commit_update(
    ctx: &mut OrgTx<'_>,
    org_id: Uuid,
    batch_id: Uuid,
    internal_id: &Uuid,
    spec: &ImportSpec,
) -> ApiResult<()> {
    match spec {
        ImportSpec::Client(client) => {
            let row = core_db::clients::update_client(
                &mut *ctx.tx,
                org_id,
                *internal_id,
                &client.name,
                client.notes.as_deref(),
            )
            .await?;
            touch_last_seen(
                &mut *ctx.tx,
                org_id,
                "client",
                &client.external_key,
                *internal_id,
                batch_id,
            )
            .await?;
            ctx.emit(
                types::CLIENT_UPDATED,
                "client",
                Some(row.id.to_string().as_str()),
                &serde_json::to_value(ClientUpdated {
                    client_id: row.id,
                    name: row.name.clone(),
                })
                .unwrap_or_else(|_| serde_json::json!({})),
            )
            .await?;
        }
        ImportSpec::Site(site) => {
            let row = core_db::clients::update_site(
                &mut *ctx.tx,
                org_id,
                *internal_id,
                &site.name,
                site.address.as_deref(),
                site.city.as_deref(),
                site.region.as_deref(),
                site.postal_code.as_deref(),
                site.country.as_deref(),
                site.latitude,
                site.longitude,
                site.timezone.as_deref(),
            )
            .await?;
            touch_last_seen(
                &mut *ctx.tx,
                org_id,
                "site",
                &site.external_key,
                *internal_id,
                batch_id,
            )
            .await?;
            ctx.emit(
                types::SITE_UPDATED,
                "site",
                Some(row.id.to_string().as_str()),
                &serde_json::to_value(SiteUpdated {
                    site_id: row.id,
                    client_id: row.client_id,
                    name: row.name.clone(),
                })
                .unwrap_or_else(|_| serde_json::json!({})),
            )
            .await?;
        }
        ImportSpec::Contact(contact) => {
            let row = core_db::clients::update_contact(
                &mut *ctx.tx,
                org_id,
                *internal_id,
                &contact.name,
                contact.email.as_deref(),
                contact.phone.as_deref(),
                contact.title.as_deref(),
                contact.is_primary,
            )
            .await?;
            touch_last_seen(
                &mut *ctx.tx,
                org_id,
                "contact",
                &contact.external_key,
                *internal_id,
                batch_id,
            )
            .await?;
            ctx.emit(
                types::CONTACT_UPDATED,
                "contact",
                Some(row.id.to_string().as_str()),
                &serde_json::to_value(ContactUpdated {
                    contact_id: row.id,
                    client_id: row.client_id,
                    name: row.name.clone(),
                })
                .unwrap_or_else(|_| serde_json::json!({})),
            )
            .await?;
        }
    }
    Ok(())
}

async fn touch_last_seen<'e, E>(
    executor: E,
    org_id: Uuid,
    kind: &str,
    external_key: &str,
    internal_id: Uuid,
    batch_id: Uuid,
) -> ApiResult<()>
where
    E: sqlx::PgExecutor<'e>,
{
    core_db::imports::upsert_mapping(
        executor,
        Uuid::now_v7(),
        org_id,
        kind,
        external_key,
        internal_id,
        batch_id,
    )
    .await?;
    Ok(())
}
