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
use core_domain::entities::{Actor, AuditEvent, KbArticleStatus, Principal};
use core_domain::events::{
    types, ClientCreated, ClientUpdated, ContactCreated, ContactUpdated, ImportBatchFinished,
    ImportBatchStarted, SiteCreated, SiteUpdated, TicketCreated, TimeEntryCreated,
};
use core_domain::import_core::{
    apply_field_mapping, board_projection, build_specs, chunk_is_executable, enforce_limits,
    kb_article_projection, kb_category_projection, parse_csv_chunk, parse_json_chunk, plan_chunk,
    ticket_projection, time_entry_projection, validate_mapping, ChunkPlan, ChunkSummary,
    CommitAction, ConflictPolicy, EntityKind, ImportSpec, KeyMappingLookup, ParentLookup,
    ProjectionLookup, RawRow, RowFormat, RowReport, SpecResult,
};
use core_domain::policy::{Action, Module, Permission};
use core_domain::status::TicketStatusCategory;
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
    pub(crate) fn decode(&self) -> ApiResult<(Vec<RawRow>, usize)> {
        match self.format {
            RowFormat::Json => {
                let value = self.json_rows.as_ref().ok_or_else(|| {
                    ApiError::Validation("json_rows is required for format=json".into())
                })?;
                let byte_len = serde_json::to_vec(value)
                    .map_err(|_| ApiError::Validation("body could not be encoded".into()))?
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
    /// Optional saved field-mapping template: source headers are renamed to
    /// the template's canonical columns before validation (D2 remainder).
    pub template_id: Option<Uuid>,
    #[serde(flatten)]
    pub rows: RowsPayload,
}

/// POST /imports/batches/{id}/chunks request body.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ChunkRequest {
    /// Optional saved field-mapping template applied before validation.
    pub template_id: Option<Uuid>,
    #[serde(flatten)]
    pub rows: RowsPayload,
}

/// POST /api/imports/batches — opens a committed-run span. Chunks then land
/// through `/chunks` (synchronous) or `/stage` + `/run` (async, D2
/// remainder) until the batch finishes.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateBatchRequest {
    pub entity_kind: EntityKind,
    pub conflict_policy: ConflictPolicy,
    pub label: Option<String>,
    pub source_note: Option<String>,
}

// --- saved field-mapping templates (D2 remainder) ----------------------------

/// POST /api/imports/templates request.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateImportTemplateRequest {
    /// Operator-facing template name (unique per source system + kind).
    pub name: String,
    /// Source-system label this template belongs to (e.g. "connectwise").
    pub source_system: String,
    pub entity_kind: EntityKind,
    /// Source column → canonical Marula column. Targets are validated
    /// against the canonical field catalog for the entity kind.
    #[schema(value_type = Object)]
    pub mapping: BTreeMap<String, String>,
}

/// PUT /api/imports/templates/{id} request — all fields optional.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateTemplateRequest {
    pub name: Option<String>,
    pub source_system: Option<String>,
    pub entity_kind: EntityKind,
    #[schema(value_type = Object)]
    pub mapping: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TemplateViewResponse {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    pub name: String,
    pub source_system: String,
    pub entity_kind: EntityKind,
    #[schema(value_type = Object)]
    pub mapping: serde_json::Value,
    #[schema(value_type = uuid::Uuid)]
    pub created_by: Uuid,
    pub created_by_kind: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

fn template_view(t: core_db::imports::ImportMappingTemplate) -> TemplateViewResponse {
    let kind = t.entity_kind.parse().unwrap_or(EntityKind::Client);
    TemplateViewResponse {
        id: t.id,
        name: t.name,
        source_system: t.source_system,
        entity_kind: kind,
        mapping: t.mapping,
        created_by: t.created_by,
        created_by_kind: t.created_by_kind,
        created_at: t.created_at,
        updated_at: t.updated_at,
    }
}

// --- async giant batches (D2 remainder) -----------------------------------------

/// POST /imports/batches/{id}/stage request: uploads ONE chunk of rows to
/// the ObjectStore under a deterministic key without executing anything.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct StageChunkRequest {
    /// Optional saved field-mapping template applied at STAGE time, so the
    /// stored chunk is already in canonical columns.
    pub template_id: Option<Uuid>,
    #[serde(flatten)]
    pub rows: RowsPayload,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct StageChunkResponse {
    #[schema(value_type = uuid::Uuid)]
    pub batch_id: Uuid,
    /// Index of the chunk just staged (its deterministic key suffix).
    pub chunk_index: i32,
    /// Total staged chunks after this upload — the worker's runbook length.
    pub staged_chunks: i32,
}

/// POST /imports/batches/{id}/run response: the batch is queued for the
/// worker exactly once; poll the batch view for counters.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RunBatchResponse {
    #[schema(value_type = uuid::Uuid)]
    pub batch_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub job_id: Uuid,
    pub staged_chunks: i32,
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
    /// Async giant-batch bookkeeping (D2 remainder): staged chunk count and
    /// the once-only worker-enqueue stamp.
    pub staged_chunks: i32,
    pub queued_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<core_db::imports::ImportBatch> for BatchViewResponse {
    fn from(b: core_db::imports::ImportBatch) -> Self {
        // Entity-kind/policy strings round-trip through their engine enums;
        // unknown future values cannot exist while the CHECKs hold.
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
            staged_chunks: b.staged_chunks,
            queued_at: b.queued_at,
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

/// Maps the per-org unique slug constraint to the same typed 409 the KB
/// authoring surface returns — a bulk push must not degrade a slug
/// collision into an opaque storage error.
fn slug_conflict(e: core_db::DbError, what: &str) -> ApiError {
    match &e {
        core_db::DbError::Sql(sqlx::Error::Database(db))
            if db.constraint().unwrap_or("").contains("slug") =>
        {
            ApiError::Conflict(format!("slug already in use ({what})"))
        }
        _ => e.into(),
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
    // Existence IS part of parent resolution: a stale mapping pointing at a
    // deleted row is reported as a missing parent, never trusted. The
    // client/site/contact families have batch getters; the newer families
    // resolve per-id (chunks are bounded, so this stays comfortably cheap).
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
        EntityKind::Board | EntityKind::Ticket | EntityKind::TimeEntry => {
            let mut alive = Vec::with_capacity(ids.len());
            for id in &ids {
                let found = match kind {
                    EntityKind::Board => core_db::boards::get_board(&mut **tx, org_id, *id)
                        .await
                        .is_ok(),
                    EntityKind::Ticket => core_db::tickets::get_ticket(&mut **tx, org_id, *id)
                        .await
                        .is_ok(),
                    _ => core_db::timers::get_time_entry(&mut **tx, org_id, *id)
                        .await
                        .is_ok(),
                };
                if found {
                    alive.push(*id);
                }
            }
            alive
        }
        EntityKind::KbCategory | EntityKind::KbArticle => {
            let mut alive = Vec::with_capacity(ids.len());
            for id in &ids {
                let found = match kind {
                    EntityKind::KbCategory => core_db::kb::get_category(&mut **tx, org_id, *id)
                        .await
                        .is_ok(),
                    _ => core_db::kb::get_article(&mut **tx, org_id, *id)
                        .await
                        .is_ok(),
                };
                if found {
                    alive.push(*id);
                }
            }
            alive
        }
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
        EntityKind::Client | EntityKind::Board => {}
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
        EntityKind::Ticket => {
            let board_keys = collect_keys(specs, |spec| match spec {
                ImportSpec::Ticket(t) => Some(t.board_key.clone()),
                _ => None,
            });
            let client_keys = collect_keys(specs, |spec| match spec {
                ImportSpec::Ticket(t) => t.client_key.clone(),
                _ => None,
            });
            let site_keys = collect_keys(specs, |spec| match spec {
                ImportSpec::Ticket(t) => t.site_key.clone(),
                _ => None,
            });
            let contact_keys = collect_keys(specs, |spec| match spec {
                ImportSpec::Ticket(t) => t.contact_key.clone(),
                _ => None,
            });
            load_parents(
                tx,
                org_id,
                EntityKind::Board,
                &dedupe(board_keys),
                &mut snap,
            )
            .await?;
            load_parents(
                tx,
                org_id,
                EntityKind::Client,
                &dedupe(client_keys),
                &mut snap,
            )
            .await?;
            load_parents(tx, org_id, EntityKind::Site, &dedupe(site_keys), &mut snap).await?;
            load_parents(
                tx,
                org_id,
                EntityKind::Contact,
                &dedupe(contact_keys),
                &mut snap,
            )
            .await?;
        }
        EntityKind::TimeEntry => {
            let ticket_keys = collect_keys(specs, |spec| match spec {
                ImportSpec::TimeEntry(e) => Some(e.ticket_key.clone()),
                _ => None,
            });
            load_parents(
                tx,
                org_id,
                EntityKind::Ticket,
                &dedupe(ticket_keys),
                &mut snap,
            )
            .await?;
        }
        EntityKind::KbCategory => {}
        EntityKind::KbArticle => {
            let category_slugs = collect_keys(specs, |spec| match spec {
                ImportSpec::KbArticle(a) => a.category_slug.clone(),
                _ => None,
            });
            load_parents(
                tx,
                org_id,
                EntityKind::KbCategory,
                &dedupe(category_slugs),
                &mut snap,
            )
            .await?;
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
        EntityKind::Board => {
            let mut out = Vec::with_capacity(ids.len());
            for id in &ids {
                let board = core_db::boards::get_board(&mut **tx, org_id, *id).await?;
                out.push((board.id, board_projection(&board)));
            }
            out
        }
        EntityKind::Ticket => {
            let mut out = Vec::with_capacity(ids.len());
            for id in &ids {
                let ticket = core_db::tickets::get_ticket(&mut **tx, org_id, *id).await?;
                out.push((ticket.id, ticket_projection(&ticket)));
            }
            out
        }
        EntityKind::TimeEntry => {
            let mut out = Vec::with_capacity(ids.len());
            for id in &ids {
                let entry = core_db::timers::get_time_entry(&mut **tx, org_id, *id).await?;
                out.push((entry.id, time_entry_projection(&entry)));
            }
            out
        }
        EntityKind::KbCategory => {
            let mut out = Vec::with_capacity(ids.len());
            for id in &ids {
                let category = core_db::kb::get_category(&mut **tx, org_id, *id).await?;
                out.push((category.id, kb_category_projection(&category)));
            }
            out
        }
        EntityKind::KbArticle => {
            let mut out = Vec::with_capacity(ids.len());
            for id in &ids {
                let article = core_db::kb::get_article(&mut **tx, org_id, *id).await?;
                out.push((article.id, kb_article_projection(&article)));
            }
            out
        }
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

/// Applies a saved field-mapping template to decoded rows. The template
/// must exist and target the SAME entity kind as the batch — a mismatched
/// template is an operator mistake we refuse loudly, never a silent
/// remap. Runs inside the caller's org-scoped transaction so RLS applies.
async fn apply_template(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    org_id: Uuid,
    kind: EntityKind,
    template_id: Uuid,
    rows: Vec<RawRow>,
) -> ApiResult<Vec<RawRow>> {
    let template = core_db::imports::get_template(&mut **tx, org_id, template_id)
        .await
        .map_err(|_| ApiError::NotFound)?;
    if template.entity_kind != kind.as_str() {
        return Err(ApiError::Validation(format!(
            "template {} targets entity_kind {}, not {}",
            template.id,
            template.entity_kind,
            kind.as_str()
        )));
    }
    let Some(map_obj) = template.mapping.as_object() else {
        return Err(ApiError::Validation(
            "template mapping document is not an object".into(),
        ));
    };
    let mapping: BTreeMap<String, String> = map_obj
        .iter()
        .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
        .collect();
    apply_field_mapping(rows, &mapping).map_err(ApiError::from)
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
            // A saved template remaps source headers BEFORE validation, so
            // the plan reports canonical-name problems only.
            let raw_rows = match req.template_id {
                Some(template_id) => {
                    apply_template(&mut ctx.tx, org_id, kind, template_id, raw_rows).await?
                }
                None => raw_rows,
            };
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

    let result = commit_chunk(ChunkRun {
        state: &state,
        org_id,
        user_id: authed.principal.user.id,
        batch_id,
        descriptor: &descriptor,
        template_id: req.template_id,
        via_worker: false,
        raw_rows,
    })
    .await?;
    Ok(Json(result))
}

/// Everything one chunk commit needs. A struct because the parameter list
/// outgrew clippy's readability budget — the runner and the HTTP route
/// fill it from different worlds.
pub(crate) struct ChunkRun<'a> {
    pub state: &'a AppState,
    pub org_id: Uuid,
    /// The driving principal (drives the org_tx user GUC).
    pub user_id: Uuid,
    pub batch_id: Uuid,
    /// Audit descriptor for actor_kind "import".
    pub descriptor: &'a str,
    pub template_id: Option<Uuid>,
    /// Marks the worker's replay path: the sync route refuses chunks on
    /// QUEUED batches (they belong to the worker now), while the worker
    /// itself must run them.
    pub via_worker: bool,
    pub raw_rows: Vec<RawRow>,
}

/// The ONE plan→commit transaction shared by the synchronous HTTP chunk
/// path and the async worker runner (D2 remainder). Applies an optional
/// saved template, plans against a fresh org-scoped snapshot, executes the
/// decided actions, and bumps the batch counters — all atomically. A
/// fail-policy chunk with any conflict persists nothing and surfaces as a
/// typed 409 `import_chunk_rejected`.
pub(crate) async fn commit_chunk(run: ChunkRun<'_>) -> ApiResult<RunReportResponse> {
    let ChunkRun {
        state,
        org_id,
        user_id,
        batch_id,
        descriptor,
        template_id,
        via_worker,
        raw_rows,
    } = run;
    org_tx::with_org_tx(state, org_id, user_id, |mut ctx| async move {
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
        if batch.queued_at.is_some() && !via_worker {
            return Err(ApiError::Conflict(
                "batch already queued for async execution".into(),
            ));
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

        let raw_rows = match template_id {
            Some(template_id) => {
                apply_template(&mut ctx.tx, org_id, kind, template_id, raw_rows).await?
            }
            None => raw_rows,
        };
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
                descriptor,
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
        Ok((ctx, RunReportResponse::of(view, &plan)))
    })
    .await
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
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let batch = core_db::imports::get_batch(&mut *tx, authed.principal.user.org_id, batch_id)
        .await
        .map_err(|_| ApiError::NotFound)?;
    tx.commit().await.map_err(ApiError::from)?;
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
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let batches =
        core_db::imports::list_batches(&mut *tx, authed.principal.user.org_id, 100).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(batches.into_iter().map(Into::into).collect()))
}

// --- saved field-mapping templates (D2 remainder) ----------------------------------

/// Shared shape rule for template names and source-system labels: short,
/// visible text, no control characters.
fn validate_template_label(value: &str, field: &str) -> ApiResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::Validation(format!("{field} must not be empty")));
    }
    if trimmed.chars().count() > 120 {
        return Err(ApiError::Validation(format!(
            "{field} must be at most 120 characters"
        )));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(ApiError::Validation(format!(
            "{field} must not contain control characters"
        )));
    }
    Ok(trimmed.to_string())
}

/// POST /api/imports/templates — saves one reusable source→canonical column
/// mapping. Targets are validated against the canonical field catalog for
/// the entity kind; duplicates per (org, source system, kind, name) are a
/// typed 409.
#[utoipa::path(operation_id = "imports_create_template", post, path = "/api/imports/templates", tag = "imports",
    security(("bearer" = [])),
    request_body = CreateImportTemplateRequest,
    responses((status = 200, body = TemplateViewResponse)))]
pub async fn create_template(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateImportTemplateRequest>,
) -> ApiResult<Json<TemplateViewResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_import_gate(&authed.principal, authed.via_api_key)?;
    let org_id = authed.principal.user.org_id;

    validate_mapping(req.entity_kind, &req.mapping)
        .map_err(|errors| ApiError::Validation(errors.join("; ")))?;
    let name = validate_template_label(&req.name, "name")?;
    let source_system = validate_template_label(&req.source_system, "source_system")?;
    let mapping = serde_json::to_value(&req.mapping)
        .map_err(|_| ApiError::Validation("mapping could not be encoded".into()))?;

    let record = core_db::imports::ImportMappingTemplate {
        id: Uuid::now_v7(),
        org_id,
        name,
        source_system,
        entity_kind: req.entity_kind.as_str().to_string(),
        mapping,
        created_by: authed.principal.user.id,
        created_by_kind: if authed.via_api_key {
            "api_key"
        } else {
            "user"
        }
        .to_string(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    // RLS-scoped write (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    core_db::imports::insert_template(&mut *tx, &record).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(template_view(record)))
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct TemplateListQuery {
    /// Narrow the listing to one source system.
    pub source_system: Option<String>,
}

/// GET /api/imports/templates
#[utoipa::path(operation_id = "imports_list_templates", get, path = "/api/imports/templates", tag = "imports",
    security(("bearer" = [])),
    params(("source_system" = Option<String>, Query, description = "Narrow to one source system")),
    responses((status = 200, body = [TemplateViewResponse])))]
pub async fn list_templates(
    State(state): State<AppState>,
    extensions: Extensions,
    axum::extract::Query(query): axum::extract::Query<TemplateListQuery>,
) -> ApiResult<Json<Vec<TemplateViewResponse>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_import_gate(&authed.principal, authed.via_api_key)?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let templates = core_db::imports::list_templates(
        &mut *tx,
        authed.principal.user.org_id,
        query.source_system.as_deref(),
        100,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(templates.into_iter().map(template_view).collect()))
}

/// GET /api/imports/templates/{id}
#[utoipa::path(operation_id = "imports_get_template", get, path = "/api/imports/templates/{template_id}", tag = "imports",
    security(("bearer" = [])),
    params(("template_id" = Uuid, Path)),
    responses((status = 200, body = TemplateViewResponse)))]
pub async fn get_template(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(template_id): Path<Uuid>,
) -> ApiResult<Json<TemplateViewResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_import_gate(&authed.principal, authed.via_api_key)?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let template =
        core_db::imports::get_template(&mut *tx, authed.principal.user.org_id, template_id)
            .await
            .map_err(|_| ApiError::NotFound)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(template_view(template)))
}

/// PUT /api/imports/templates/{id} — content update; uniqueness conflicts
/// (renaming onto an existing sibling) surface as typed 409s.
#[utoipa::path(operation_id = "imports_update_template", put, path = "/api/imports/templates/{template_id}", tag = "imports",
    security(("bearer" = [])),
    params(("template_id" = Uuid, Path)),
    request_body = UpdateTemplateRequest,
    responses((status = 200, body = TemplateViewResponse)))]
pub async fn update_template(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(template_id): Path<Uuid>,
    Json(req): Json<UpdateTemplateRequest>,
) -> ApiResult<Json<TemplateViewResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_import_gate(&authed.principal, authed.via_api_key)?;
    let org_id = authed.principal.user.org_id;

    // RLS-scoped read + write (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let existing = core_db::imports::get_template(&mut *tx, org_id, template_id)
        .await
        .map_err(|_| ApiError::NotFound)?;
    let name = match &req.name {
        Some(name) => validate_template_label(name, "name")?,
        None => existing.name,
    };
    let source_system = match &req.source_system {
        Some(source) => validate_template_label(source, "source_system")?,
        None => existing.source_system,
    };
    let mapping = match &req.mapping {
        Some(mapping) => {
            let kind: EntityKind = existing.entity_kind.parse().map_err(|_| {
                ApiError::Validation("template entity_kind no longer parses".into())
            })?;
            validate_mapping(kind, mapping)
                .map_err(|errors| ApiError::Validation(errors.join("; ")))?;
            serde_json::to_value(mapping)
                .map_err(|_| ApiError::Validation("mapping could not be encoded".into()))?
        }
        None => existing.mapping,
    };
    let updated = core_db::imports::update_template(
        &mut *tx,
        org_id,
        template_id,
        &name,
        &source_system,
        &mapping,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(template_view(updated)))
}

/// DELETE /api/imports/templates/{id}
#[utoipa::path(operation_id = "imports_delete_template", delete, path = "/api/imports/templates/{template_id}", tag = "imports",
    security(("bearer" = [])),
    params(("template_id" = Uuid, Path)),
    responses((status = 200, body = TemplateViewResponse)))]
pub async fn delete_template(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(template_id): Path<Uuid>,
) -> ApiResult<Json<TemplateViewResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_import_gate(&authed.principal, authed.via_api_key)?;
    let org_id = authed.principal.user.org_id;
    // RLS-scoped read + write (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let existing = core_db::imports::get_template(&mut *tx, org_id, template_id)
        .await
        .map_err(|_| ApiError::NotFound)?;
    core_db::imports::delete_template(&mut *tx, org_id, template_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(template_view(existing)))
}

// --- async giant batches (D2 remainder) ---------------------------------------------

/// POST /api/imports/batches/{id}/stage — uploads ONE chunk of rows to the
/// ObjectStore under a deterministic key WITHOUT executing anything. An
/// optional saved template is applied at stage time so the stored chunk is
/// already canonical; the worker later replays the staged sequence.
#[utoipa::path(operation_id = "imports_stage_chunk", post, path = "/api/imports/batches/{batch_id}/stage", tag = "imports",
    security(("bearer" = [])),
    params(("batch_id" = Uuid, Path)),
    request_body = StageChunkRequest,
    responses((status = 200, body = StageChunkResponse)))]
pub async fn stage_chunk(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(batch_id): Path<Uuid>,
    Json(req): Json<StageChunkRequest>,
) -> ApiResult<Json<StageChunkResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_import_gate(&authed.principal, authed.via_api_key)?;
    let org_id = authed.principal.user.org_id;

    let (raw_rows, byte_len) = req.rows.decode()?;
    enforce_limits(raw_rows.len(), byte_len)?;
    let store = state.store.clone();

    let staged = org_tx::with_org_tx(
        &state,
        org_id,
        authed.principal.user.id,
        |mut ctx| async move {
            let batch = core_db::imports::get_batch(&mut *ctx.tx, org_id, batch_id)
                .await
                .map_err(|_| ApiError::NotFound)?;
            if batch.dry_run {
                return Err(ApiError::Validation(
                    "cannot stage into a dry-run probe".into(),
                ));
            }
            let kind: EntityKind = batch.entity_kind.parse().map_err(|_| {
                ApiError::Validation(format!(
                    "unsupported entity_kind {:?} recorded on this deployment",
                    batch.entity_kind
                ))
            })?;
            // Template remap happens at STAGE time so stored chunks are
            // canonical and the worker never needs template context.
            let raw_rows = match req.template_id {
                Some(template_id) => {
                    apply_template(&mut ctx.tx, org_id, kind, template_id, raw_rows).await?
                }
                None => raw_rows,
            };
            let chunk_index = core_db::imports::bump_staged_chunks(&mut *ctx.tx, org_id, batch_id)
                .await?
                .ok_or_else(|| {
                    ApiError::Conflict("batch already queued or finished; staging is closed".into())
                })?;
            let chunk_index = chunk_index - 1;
            let canonical: Vec<serde_json::Value> = raw_rows
                .iter()
                .map(|row| {
                    serde_json::Value::Object(
                        row.fields
                            .iter()
                            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                            .collect(),
                    )
                })
                .collect();
            let body = serde_json::to_vec(&serde_json::json!({
                "format": "json",
                "json_rows": canonical,
            }))
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("chunk serialize: {e}")))?;
            let key = core_db::imports::staged_chunk_key(org_id, batch_id, chunk_index);
            store
                .put(&key, "application/json", bytes::Bytes::from(body))
                .await
                .map_err(crate::routes::attachments::store_error)?;
            Ok((
                ctx,
                StageChunkResponse {
                    batch_id,
                    chunk_index,
                    staged_chunks: chunk_index + 1,
                },
            ))
        },
    )
    .await?;
    Ok(Json(staged))
}

/// POST /api/imports/batches/{id}/run — hands the staged batch to the
/// worker queue exactly once (generalized 2.7 job queue). Re-running a
/// finished or queued batch is a typed 409; the worker replays chunks
/// idempotently through the SAME commit path as synchronous pushes.
#[utoipa::path(operation_id = "imports_run_batch", post, path = "/api/imports/batches/{batch_id}/run", tag = "imports",
    security(("bearer" = [])),
    params(("batch_id" = Uuid, Path)),
    responses((status = 200, body = RunBatchResponse)))]
pub async fn run_batch(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(batch_id): Path<Uuid>,
) -> ApiResult<Json<RunBatchResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_import_gate(&authed.principal, authed.via_api_key)?;
    let org_id = authed.principal.user.org_id;
    let descriptor = actor_descriptor(&authed.principal, authed.via_api_key);

    let response = org_tx::with_org_tx(
        &state,
        org_id,
        authed.principal.user.id,
        |mut ctx| async move {
            let batch = core_db::imports::get_batch(&mut *ctx.tx, org_id, batch_id)
                .await
                .map_err(|_| ApiError::NotFound)?;
            if batch.dry_run {
                return Err(ApiError::Validation(
                    "dry-run probes are never executed".into(),
                ));
            }
            if batch.finished_at.is_some() {
                return Err(ApiError::Conflict("batch already finished".into()));
            }
            if batch.staged_chunks == 0 {
                return Err(ApiError::Validation(
                    "nothing staged; upload chunks through /stage first".into(),
                ));
            }
            let queued =
                core_db::imports::mark_batch_queued(&mut *ctx.tx, org_id, batch_id).await?;
            if !queued {
                return Err(ApiError::Conflict(
                    "batch already queued for async execution".into(),
                ));
            }
            let job_id = core_db::queue::enqueue(
                &mut *ctx.tx,
                &core_db::queue::NewJob {
                    org_id,
                    kind: crate::routes::internal::IMPORT_BATCH_JOB_KIND.to_string(),
                    payload: serde_json::json!({
                        "org_id": org_id,
                        "batch_id": batch_id,
                    }),
                    max_attempts: core_domain::retry::attempt_limit_i32(
                        core_domain::retry::Subsystem::Queue,
                    ),
                },
            )
            .await?;
            core_db::audit::insert(
                &mut *ctx.tx,
                &audit_event(
                    org_id,
                    &descriptor,
                    "import.batch_queued",
                    Some(&batch_id.to_string()),
                    serde_json::json!({
                        "job_id": job_id,
                        "staged_chunks": batch.staged_chunks,
                        "entity_kind": batch.entity_kind,
                    }),
                ),
            )
            .await?;
            Ok((
                ctx,
                RunBatchResponse {
                    batch_id,
                    job_id,
                    staged_chunks: batch.staged_chunks,
                },
            ))
        },
    )
    .await?;
    Ok(Json(response))
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
        staged_chunks: 0,
        queued_at: None,
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
        ImportSpec::Board(board) => {
            let id = Uuid::now_v7();
            let row = core_db::boards::create_board(
                &mut *ctx.tx,
                org_id,
                id,
                &board.name,
                board.description.as_deref(),
                board.is_default,
            )
            .await?;
            // The semantic-category status floor, seeded exactly like the
            // boards API surface — without it no ticket could ever land on
            // an imported board (defaults-first doctrine).
            const DEFAULT_STATUSES: [(&str, TicketStatusCategory, i32, bool); 4] = [
                ("New", TicketStatusCategory::Open, 0, true),
                ("In Progress", TicketStatusCategory::InProgress, 10, false),
                ("Waiting", TicketStatusCategory::Waiting, 20, false),
                ("Resolved", TicketStatusCategory::Closed, 30, false),
            ];
            for (name, category, sort, is_entry) in DEFAULT_STATUSES {
                core_db::boards::add_status(
                    &mut ctx.tx,
                    org_id,
                    Uuid::now_v7(),
                    id,
                    name,
                    category,
                    sort,
                    is_entry,
                    None,
                )
                .await?;
            }
            core_db::imports::upsert_mapping(
                &mut *ctx.tx,
                key_row_id,
                org_id,
                "board",
                &board.external_key,
                id,
                batch_id,
            )
            .await?;
            ctx.emit(
                types::BOARD_CREATED,
                "board",
                Some(row.id.to_string().as_str()),
                &serde_json::json!({ "board_id": row.id, "name": row.name }),
            )
            .await?;
        }
        ImportSpec::Ticket(ticket) => {
            let id = Uuid::now_v7();
            let board_id = resolve_parent(parents, EntityKind::Board, &ticket.board_key)?;
            let client_id = match &ticket.client_key {
                Some(key) => Some(resolve_parent(parents, EntityKind::Client, key)?),
                None => None,
            };
            let site_id = match &ticket.site_key {
                Some(key) => Some(resolve_parent(parents, EntityKind::Site, key)?),
                None => None,
            };
            let contact_id = match &ticket.contact_key {
                Some(key) => Some(resolve_parent(parents, EntityKind::Contact, key)?),
                None => None,
            };
            let entry_status = core_db::boards::default_status_for_board(&mut *ctx.tx, board_id)
                .await
                .map_err(|_| {
                    ApiError::Validation(format!(
                        "board {board_id} has no entry status; import boards first"
                    ))
                })?;
            let (prefix, seq) = core_db::orgs::next_ticket_number(&mut *ctx.tx, org_id).await?;
            let number = format!("{prefix}-{seq}");
            let row = core_db::tickets::insert_ticket(
                &mut ctx.tx,
                core_db::tickets::NewTicket {
                    id,
                    org_id,
                    number: &number,
                    board_id,
                    status_id: entry_status.id,
                    title: &ticket.title,
                    body: ticket.body.as_deref(),
                    priority: ticket.priority,
                    client_id,
                    site_id,
                    contact_id,
                    assignee_id: None,
                    requester_id: None,
                    due_at: ticket.due_at,
                },
            )
            .await?;
            core_db::imports::upsert_mapping(
                &mut *ctx.tx,
                key_row_id,
                org_id,
                "ticket",
                &ticket.external_key,
                id,
                batch_id,
            )
            .await?;
            ctx.emit(
                types::TICKET_CREATED,
                "ticket",
                Some(row.id.to_string().as_str()),
                &serde_json::to_value(TicketCreated {
                    ticket_id: row.id,
                    number: number.clone(),
                    board_id: row.board_id,
                    title: row.title.clone(),
                    priority: row.priority,
                    client_id: row.client_id,
                    assignee_id: row.assignee_id,
                    created_at: row.created_at,
                })
                .unwrap_or_else(|_| serde_json::json!({})),
            )
            .await?;
        }
        ImportSpec::TimeEntry(entry) => {
            let id = Uuid::now_v7();
            let ticket_id = resolve_parent(parents, EntityKind::Ticket, &entry.ticket_key)?;
            let user_id = match &entry.user_email {
                // Attributed to a live org user when the email resolves;
                // otherwise the driving principal is the documented
                // fallback (source systems rarely carry Marula ids).
                Some(email) => {
                    match core_db::users::find_by_email_in_org(&mut *ctx.tx, org_id, email).await? {
                        Some(user) => user.id,
                        None => {
                            return Err(ApiError::Validation(format!(
                                "user_email {email:?}: no active org user with this address"
                            )))
                        }
                    }
                }
                None => ctx.user_id,
            };
            let ended_at = entry
                .ended_at
                .unwrap_or_else(|| entry.started_at + chrono::Duration::minutes(entry.minutes));
            let row = core_domain::entities::TimeEntry {
                approval_status: Some(core_domain::entities::ApprovalStatus::Draft),
                approval_reason_code: None,
                unbillable_reason: None,
                id,
                org_id,
                ticket_id: Some(ticket_id),
                phase_id: None,
                user_id,
                started_at: entry.started_at,
                ended_at,
                minutes: entry.minutes,
                billable: entry.billable,
                notes: entry.notes.clone(),
                work_type: entry.work_type.clone(),
                timer_session_id: None,
                created_at: chrono::Utc::now(),
            };
            core_db::timers::insert_time_entry(&mut *ctx.tx, &row).await?;
            core_db::imports::upsert_mapping(
                &mut *ctx.tx,
                key_row_id,
                org_id,
                "time_entry",
                &entry.external_key,
                id,
                batch_id,
            )
            .await?;
            ctx.emit(
                types::TIME_ENTRY_CREATED,
                "time_entry",
                Some(row.id.to_string().as_str()),
                &serde_json::to_value(TimeEntryCreated { entry: row.clone() })
                    .unwrap_or_else(|_| serde_json::json!({})),
            )
            .await?;
        }
        ImportSpec::KbCategory(category) => {
            let id = Uuid::now_v7();
            core_db::kb::create_category(
                &mut *ctx.tx,
                org_id,
                id,
                &category.name,
                &category.slug,
                category.description.as_deref().unwrap_or_default(),
                category.position,
                category.client_visible,
            )
            .await
            .map_err(|e| slug_conflict(e, "kb category"))?;
            core_db::imports::upsert_mapping(
                &mut *ctx.tx,
                key_row_id,
                org_id,
                "kb_category",
                &category.external_key,
                id,
                batch_id,
            )
            .await?;
            // Categories are non-evented on the authoring surface too —
            // only articles carry lifecycle events (FTS rides those).
        }
        ImportSpec::KbArticle(article) => {
            let id = Uuid::now_v7();
            let category_id = match &article.category_slug {
                Some(key) => Some(resolve_parent(parents, EntityKind::KbCategory, key)?),
                None => None,
            };
            let row = core_db::kb::create_article(
                &mut *ctx.tx,
                org_id,
                id,
                category_id,
                &article.title,
                &article.slug,
                article.body.as_deref().unwrap_or_default(),
                None, // imports carry plain/markdown text; 0077 wrap-on-write applies only to editor writes
                article.status.unwrap_or(KbArticleStatus::Draft),
                article.client_visible,
                ctx.user_id,
            )
            .await
            .map_err(|e| slug_conflict(e, "kb article"))?;
            core_db::imports::upsert_mapping(
                &mut *ctx.tx,
                key_row_id,
                org_id,
                "kb_article",
                &article.external_key,
                id,
                batch_id,
            )
            .await?;
            // Same event + payload the authoring POST emits, so the
            // unified-search indexer treats an imported `published`
            // article exactly like one created by hand.
            ctx.emit(
                types::KB_ARTICLE_CREATED,
                "kb_article",
                Some(row.id.to_string().as_str()),
                &serde_json::json!({ "article_id": row.id, "status": row.status.as_str() }),
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
        ImportSpec::Board(board) => {
            let row = core_db::boards::update_board(
                &mut *ctx.tx,
                org_id,
                *internal_id,
                &board.name,
                board.description.as_deref(),
            )
            .await?;
            touch_last_seen(
                &mut *ctx.tx,
                org_id,
                "board",
                &board.external_key,
                *internal_id,
                batch_id,
            )
            .await?;
            ctx.emit(
                types::BOARD_UPDATED,
                "board",
                Some(row.id.to_string().as_str()),
                &serde_json::json!({ "board_id": row.id, "name": row.name }),
            )
            .await?;
        }
        ImportSpec::Ticket(ticket) => {
            core_db::tickets::update_ticket(
                &mut *ctx.tx,
                org_id,
                *internal_id,
                core_db::tickets::TicketPatch {
                    title: Some(&ticket.title),
                    body: ticket.body.as_deref().map(Some),
                    priority: Some(ticket.priority),
                    // Parent links are identity, never content drift.
                    assignee_id: None,
                    client_id: None,
                    site_id: None,
                    contact_id: None,
                    due_at: Some(ticket.due_at),

                    type_id: None,
                    category_id: None,
                },
            )
            .await?;
            touch_last_seen(
                &mut *ctx.tx,
                org_id,
                "ticket",
                &ticket.external_key,
                *internal_id,
                batch_id,
            )
            .await?;
            ctx.emit(
                types::TICKET_UPDATED,
                "ticket",
                Some(internal_id.to_string().as_str()),
                &serde_json::json!({ "ticket_id": internal_id }),
            )
            .await?;
        }
        ImportSpec::TimeEntry(entry) => {
            let ended_at = entry
                .ended_at
                .unwrap_or_else(|| entry.started_at + chrono::Duration::minutes(entry.minutes));
            core_db::timers::update_time_entry(
                &mut *ctx.tx,
                org_id,
                *internal_id,
                entry.started_at,
                ended_at,
                entry.minutes,
                entry.billable,
                entry.notes.as_deref(),
                entry.work_type.as_deref(),
                entry.unbillable_reason.as_deref(),
            )
            .await?;
            touch_last_seen(
                &mut *ctx.tx,
                org_id,
                "time_entry",
                &entry.external_key,
                *internal_id,
                batch_id,
            )
            .await?;
            ctx.emit(
                types::TIME_ENTRY_UPDATED,
                "time_entry",
                Some(internal_id.to_string().as_str()),
                &serde_json::json!({ "entry_id": internal_id }),
            )
            .await?;
        }
        ImportSpec::KbCategory(category) => {
            core_db::kb::update_category(
                &mut *ctx.tx,
                org_id,
                *internal_id,
                &category.name,
                category.description.as_deref().unwrap_or_default(),
                category.position,
                category.client_visible,
            )
            .await?;
            touch_last_seen(
                &mut *ctx.tx,
                org_id,
                "kb_category",
                &category.external_key,
                *internal_id,
                batch_id,
            )
            .await?;
        }
        ImportSpec::KbArticle(article) => {
            // The archived content lock is a core-kb rule, not a route
            // rule: a bulk overwrite must not become a side channel around
            // it. Status flips stay legal (that is how an import
            // unarchives), and content-identical repushes never trip the
            // guard — the planner only routes true drift here.
            let stored = core_db::kb::get_article(&mut *ctx.tx, org_id, *internal_id).await?;
            if stored.status == KbArticleStatus::Archived {
                let content_drift = stored.title != article.title
                    || stored.body != article.body.clone().unwrap_or_default();
                if content_drift {
                    return Err(ApiError::Validation(format!(
                        "article {} is archived and refuses title/body edits; \
                         re-publish it first",
                        internal_id
                    )));
                }
            }
            core_db::kb::update_article(
                &mut *ctx.tx,
                org_id,
                *internal_id,
                &core_db::kb::ArticlePatch {
                    title: Some(article.title.clone()),
                    body: Some(article.body.clone().unwrap_or_default()),
                    // Importer rows are plain text (D5); rich content
                    // stays NULL until the editor path writes it.
                    content_jsonb: None,
                    // Parent links are identity, never content drift.
                    category_id: None,
                    status: Some(article.status.unwrap_or(KbArticleStatus::Draft)),
                    client_visible: Some(article.client_visible),
                    updated_by: Some(ctx.user_id),
                },
            )
            .await?;
            touch_last_seen(
                &mut *ctx.tx,
                org_id,
                "kb_article",
                &article.external_key,
                *internal_id,
                batch_id,
            )
            .await?;
            ctx.emit(
                types::KB_ARTICLE_UPDATED,
                "kb_article",
                Some(internal_id.to_string().as_str()),
                &serde_json::json!({
                    "article_id": internal_id,
                    "status": article.status.unwrap_or(KbArticleStatus::Draft).as_str()
                }),
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
