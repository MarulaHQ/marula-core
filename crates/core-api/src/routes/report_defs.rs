//! Report definitions CRUD (the private build tracker R3; ADR 0001 §Decision).
//!
//! Purpose: the admin-gated HTTP surface for STORED report specs —
//! registry-validated definitions over the `ops.v1_*` read-model views,
//! kept as data so the U5 customization manifest covers them and the U6
//! upgrade preflight can flag deprecated-construct usage (policy §5).
//!
//! RBAC (the "manager sees org, user sees own" line from R1, applied to
//! definitions):
//! * reads require `reporting:view`; owner/admin list every org
//!   definition, everyone else sees org-published rows plus their own;
//! * writes require `reporting:create`/`edit`/`delete` (admin-gated —
//!   builtin roles grant those to owner/admin only); a non-admin may
//!   touch ONLY their own `self`-scope definitions and can never publish
//!   to `org` scope.
//!
//! Instant-first contract (IF1–IF3): every mutation routes through the
//! shared idempotency gate + version gate + write-trail layer and returns
//! read-your-write `mutation` metadata; every failure is a typed envelope
//! reason (`validation_failed`, `forbidden`, `conflict`, `rls_sparse`),
//! never prose to parse.
//!
//! Execution: deliberately absent. Definitions are templates; running them
//! (R6 builder / R4 delivery) compiles on demand via
//! `core_reports::ReportSpec::compile` against the reporting pool. This
//! module never executes SQL against report data. A definition MAY carry a
//! card (migration 0180); the card's window seeds the run default when the
//! caller supplies no bounds, and nothing else about a run changes.
//!
//! Non-purpose: no dashboards UI (R2), no scheduling (R4), no share-by-
//! link (R6), no manifest assembly (U5 reads core-db directly).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome, MutationParams};
use crate::org_tx;
use crate::routes::ApiGeneration;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use core_reports::{CardSpec, ReportSpec, RunWindow, ViewId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

/// Visibility scope of a stored definition: `org` (published org-wide,
/// admins only) or `self` (private to the creator).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DefinitionScope {
    Org,
    #[serde(rename = "self")]
    Self_,
}

impl DefinitionScope {
    fn as_str(self) -> &'static str {
        match self {
            DefinitionScope::Org => "org",
            DefinitionScope::Self_ => "self",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "org" => Some(DefinitionScope::Org),
            "self" => Some(DefinitionScope::Self_),
            _ => None,
        }
    }
}

/// A stored report definition with its registry-validated spec
/// reassembled (`spec.view` == the stored `kind` column).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReportDefinitionView {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub spec: ReportSpec,
    /// The card stored with the definition, when it has one: its size, the
    /// window it displays and how it refreshes. Absent is legitimate.
    pub card: Option<CardSpec>,
    pub scope: DefinitionScope,
    pub created_by: Uuid,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String)]
    pub updated_at: DateTime<Utc>,
}

/// POST /api/report-definitions body.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateReportDefinitionRequest {
    /// Org-unique display name (1–200 chars after trimming).
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// The registry-validated report spec.
    pub spec: ReportSpec,
    /// Optional card contract stored with the definition. Omitted means the
    /// definition carries no card; a present card must pass
    /// `CardSpec::validate`, and the refusal is the run path's own typed 422.
    #[serde(default)]
    pub card: Option<CardSpec>,
    /// Requested visibility; `org` requires admin and non-admins are
    /// forced onto `self` regardless.
    #[serde(default)]
    pub scope: Option<DefinitionScope>,
    #[serde(flatten)]
    pub instant: MutationParams,
}

/// PUT /api/report-definitions/{id} body — full-replace semantics: every
/// member is applied; `description: null` clears it and `card: null` (or an
/// omitted card) clears the card.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UpdateReportDefinitionRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub spec: ReportSpec,
    /// Full replace: `null` clears the stored card, a present card replaces
    /// it after `CardSpec::validate`.
    #[serde(default)]
    pub card: Option<CardSpec>,
    /// New visibility; `org` requires admin and non-admins keep `self`.
    #[serde(default)]
    pub scope: Option<DefinitionScope>,
    #[serde(flatten)]
    pub instant: MutationParams,
}

/// GET /api/report-definitions envelope.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReportDefinitionList {
    pub definitions: Vec<ReportDefinitionView>,
}

/// Mutation envelope: applied definition (flattened) + read-your-write
/// metadata (IF2).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReportDefinitionMutationResponse {
    #[serde(flatten)]
    pub definition: ReportDefinitionView,
    pub mutation: instant::MutationMeta,
}

// --- router ---------------------------------------------------------------------

/// The report-definitions slice of the `/api` surface, built per
/// generation so both mounts (`/api/v1` canonical + `/api` legacy alias)
/// register identical routes through one merge line in `routes::mod`.
pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(
            &format!("{base}/report-definitions"),
            get(list).post(create),
        )
        .route(
            &format!("{base}/report-definitions/{{id}}"),
            get(get_one).put(update).delete(remove),
        )
        .route(&format!("{base}/report-definitions/{{id}}/run"), get(run))
        .route(&format!("{base}/report-catalog"), get(catalog))
        .route(&format!("{base}/reports/run-spec"), post(run_spec))
}

// --- handlers -------------------------------------------------------------------

/// GET /api/report-definitions — visible definitions, deterministically
/// ordered `(name, id)`.
#[utoipa::path(
    operation_id = "report_definitions_list",
    get,
    path = "/api/report-definitions",
    tag = "reports",
    security(("bearer" = [])),
    responses(
        (status = 200, body = ReportDefinitionList)
    )
)]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<ReportDefinitionList>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Reporting,
            action: Action::View,
        },
    )?;
    let org_id = authed.principal.user.org_id;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = if authed.principal.is_owner_or_admin() {
        core_db::report_defs::list_all(&mut *tx, org_id).await?
    } else {
        core_db::report_defs::list_visible(&mut *tx, org_id, authed.principal.user.id).await?
    };
    tx.commit().await.map_err(ApiError::from)?;
    let mut definitions = Vec::with_capacity(rows.len());
    for row in rows {
        definitions.push(view_of(row)?);
    }
    Ok(Json(ReportDefinitionList { definitions }))
}

/// GET /api/report-definitions/{id} — one visible definition; anything not
/// visible (deleted, other org, private of another user) is a bare 404.
#[utoipa::path(
    operation_id = "report_definitions_get",
    get,
    path = "/api/report-definitions/{id}",
    tag = "reports",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = ReportDefinitionView),
        (status = 404, description = "not visible in scope")
    )
)]
pub async fn get_one(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ReportDefinitionView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Reporting,
            action: Action::View,
        },
    )?;
    let org_id = authed.principal.user.org_id;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let row = core_db::report_defs::get(&mut *tx, org_id, id)
        .await
        .map_err(|e| match e {
            core_db::DbError::NotFound => ApiError::NotFound,
            other => ApiError::from(other),
        })?;
    tx.commit().await.map_err(ApiError::from)?;
    if !visible(&authed.principal, &row) {
        return Err(ApiError::NotFound);
    }
    Ok(Json(view_of(row)?))
}

/// POST /api/report-definitions — create a stored definition through the
/// instant-first write path (idempotency key honored; response carries
/// `mutation` metadata).
#[utoipa::path(
    operation_id = "report_definitions_create",
    post,
    path = "/api/report-definitions",
    tag = "reports",
    security(("bearer" = [])),
    request_body = CreateReportDefinitionRequest,
    responses(
        (status = 200, body = ReportDefinitionMutationResponse),
        (status = 403, description = "missing reporting:create"),
        (status = 409, description = "name conflict or idempotency key reused"),
        (status = 422, description = "spec failed registry validation")
    )
)]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateReportDefinitionRequest>,
) -> ApiResult<Json<ReportDefinitionMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Reporting,
            action: Action::Create,
        },
    )?;
    let name = validate_name(&req.name)?;
    validate_description(req.description.as_deref())?;
    // Registry gate BEFORE anything persists.
    req.spec
        .validate()
        .map_err(|e| ApiError::Validation(e.to_string()))?;
    if let Some(card) = req.card.as_ref() {
        card.validate().map_err(card_failure)?;
    }
    let card = card_document(req.card.as_ref())?;
    let admin = authed.principal.is_owner_or_admin();
    let scope = match req.scope {
        // Publishing org-wide is an admin act; everyone else is silently
        // kept private (their intent cannot exceed their grant).
        Some(DefinitionScope::Org) if admin => DefinitionScope::Org,
        _ => DefinitionScope::Self_,
    };
    let (kind, body, viz) = req.spec.to_parts();

    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: ENTITY,
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let id = Uuid::now_v7();
        let fields = core_db::report_defs::DefinitionFields {
            name,
            description: req.description.clone(),
            kind: kind.to_string(),
            spec: serde_json::to_value(&body).unwrap_or(Value::Null),
            card,
            viz: viz.to_string(),
            scope: scope.as_str().to_string(),
        };
        let row =
            core_db::report_defs::insert(&mut ctx.tx, id, ctx.org_id, user_id, &fields).await?;
        // Fresh entity: a create presenting a non-zero version is a
        // client bug (only 0 can ever have been observed) — rejected
        // before the response is built; the tx rolls the insert back.
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let view = view_of(row)?;
        ctx.emit(
            types::REPORT_DEFINITION_CREATED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::json!({
                "definition_id": id,
                "name": view.name,
                "kind": kind,
                "scope": scope.as_str(),
            }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: view.updated_at,
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ReportDefinitionMutationResponse {
                definition: view,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PUT /api/report-definitions/{id} — full-replace update through the
/// instant-first write path (version gate enforced; a miss inside a
/// mutation is the typed `rls_sparse` reason, not `not_found`).
#[utoipa::path(
    operation_id = "report_definitions_update",
    put,
    path = "/api/report-definitions/{id}",
    tag = "reports",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = UpdateReportDefinitionRequest,
    responses(
        (status = 200, body = ReportDefinitionMutationResponse),
        (status = 403, description = "definition outside the caller's writable scope"),
        (status = 404, description = "rls_sparse: target not visible"),
        (status = 409, description = "version conflict or idempotency key reused"),
        (status = 422, description = "spec failed registry validation")
    )
)]
pub async fn update(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateReportDefinitionRequest>,
) -> ApiResult<Json<ReportDefinitionMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Reporting,
            action: Action::Edit,
        },
    )?;
    let name = validate_name(&req.name)?;
    validate_description(req.description.as_deref())?;
    req.spec
        .validate()
        .map_err(|e| ApiError::Validation(e.to_string()))?;
    if let Some(card) = req.card.as_ref() {
        card.validate().map_err(card_failure)?;
    }
    let card = card_document(req.card.as_ref())?;
    let admin = authed.principal.is_owner_or_admin();

    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let current = core_db::report_defs::get(&mut *ctx.tx, ctx.org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        if !writable(&authed.principal, &current) {
            // Visible but not theirs to change — an authz boundary, not a
            // miss. Non-admins may only rewrite their own private rows.
            return Err(ApiError::Forbidden(
                "report definitions are writable by their creator or org admins".into(),
            ));
        }
        // Scope can only RISE to `org` under an admin pen; everyone else
        // keeps the row's current visibility.
        let scope = match req.scope {
            Some(DefinitionScope::Org) if admin => DefinitionScope::Org,
            _ => DefinitionScope::parse(&current.scope).unwrap_or(DefinitionScope::Self_),
        };
        let (kind, body, viz) = req.spec.to_parts();
        let fields = core_db::report_defs::DefinitionFields {
            name,
            description: req.description.clone(),
            kind: kind.to_string(),
            spec: serde_json::to_value(&body).unwrap_or(Value::Null),
            card,
            viz: viz.to_string(),
            scope: scope.as_str().to_string(),
        };
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let row = core_db::report_defs::update(&mut ctx.tx, ctx.org_id, id, &fields)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        let before = view_of(current)?;
        let view = view_of(row)?;
        ctx.emit(
            types::REPORT_DEFINITION_UPDATED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::json!({
                "definition_id": id,
                "name": view.name,
                "kind": kind,
                "scope": scope.as_str(),
            }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: view.updated_at,
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ReportDefinitionMutationResponse {
                definition: view,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/report-definitions/{id} — removal through the same gated
/// path; the write trail keeps the deleted snapshot (undo substrate).
#[utoipa::path(
    operation_id = "report_definitions_delete",
    delete,
    path = "/api/report-definitions/{id}",
    tag = "reports",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = ReportDefinitionMutationResponse),
        (status = 403, description = "definition outside the caller's writable scope"),
        (status = 404, description = "rls_sparse: target not visible"),
        (status = 409, description = "version conflict")
    )
)]
pub async fn remove(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ReportDefinitionMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Reporting,
            action: Action::Delete,
        },
    )?;

    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let current = core_db::report_defs::get(&mut *ctx.tx, ctx.org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        if !writable(&authed.principal, &current) {
            return Err(ApiError::Forbidden(
                "report definitions are deletable by their creator or org admins".into(),
            ));
        }
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: id,
                version: None,
            },
        )
        .await?;
        core_db::report_defs::delete(&mut ctx.tx, ctx.org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        let before = view_of(current)?;
        ctx.emit(
            types::REPORT_DEFINITION_DELETED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::json!({
                "definition_id": id,
                "name": before.name,
            }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: id,
                op: "delete",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: Value::Null,
                updated_at: before.updated_at,
                key: None,
                payload: &before,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ReportDefinitionMutationResponse {
                definition: before,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- helpers ----------------------------------------------------------------------

const ENTITY: &str = "report_definition";

/// READ visibility: admins see the whole org; everyone else sees
/// org-published definitions plus their own.
fn visible(
    principal: &core_domain::entities::Principal,
    row: &core_db::report_defs::ReportDefinitionRow,
) -> bool {
    principal.is_owner_or_admin() || row.scope == "org" || row.created_by == principal.user.id
}

/// WRITE visibility (narrower than read): non-admins may only mutate
/// their own PRIVATE definitions — org-published rows are admin-managed
/// surface by construction.
fn writable(
    principal: &core_domain::entities::Principal,
    row: &core_db::report_defs::ReportDefinitionRow,
) -> bool {
    principal.is_owner_or_admin() || (row.scope == "self" && row.created_by == principal.user.id)
}

/// Reassembles the typed, registry-validated spec from the stored
/// representation. The ONLY place stored rows turn back into specs; a
/// stored row failing validation is a server-side integrity fault
/// (writes gate on the same registry), surfaced as internal error.
fn view_of(row: core_db::report_defs::ReportDefinitionRow) -> ApiResult<ReportDefinitionView> {
    let card = card_of(&row)?;
    let spec = ReportSpec::from_parts(&row.kind, &row.spec, &row.viz).map_err(|e| {
        ApiError::Internal(anyhow::anyhow!("stored definition failed registry: {e}"))
    })?;
    Ok(ReportDefinitionView {
        id: row.id,
        name: row.name,
        description: row.description,
        spec,
        card,
        scope: DefinitionScope::parse(&row.scope).unwrap_or(DefinitionScope::Self_),
        created_by: row.created_by,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// Reassembles a stored card and re-runs the same bounds and
/// window-versus-refresh gate every write applies. A stored card that fails
/// is an integrity fault like a stored spec that fails, never caller input.
fn card_of(row: &core_db::report_defs::ReportDefinitionRow) -> ApiResult<Option<CardSpec>> {
    let Some(value) = row.card.as_ref() else {
        return Ok(None);
    };
    let card: CardSpec = serde_json::from_value(value.clone()).map_err(|e| {
        ApiError::Internal(anyhow::anyhow!("stored definition card unreadable: {e}"))
    })?;
    card.validate().map_err(|e| {
        ApiError::Internal(anyhow::anyhow!(
            "stored definition card failed validation: {e}"
        ))
    })?;
    Ok(Some(card))
}

/// The column's representation of a card: `None` stays SQL NULL, a card is
/// its JSON object (the migration's CHECK pins that shape).
fn card_document(card: Option<&CardSpec>) -> ApiResult<Option<Value>> {
    match card {
        None => Ok(None),
        Some(card) => serde_json::to_value(card)
            .map(Some)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("card did not serialize: {e}"))),
    }
}

/// The bounds a stored-definition run resolves. Explicit caller bounds always
/// win, including the one-bound case `resolve_window` collapses to a single
/// day. With no bounds at all, a card's `window_days` becomes the span; a
/// definition without a card keeps the registry's trailing-30-day default.
fn run_window_bounds(
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    card: Option<&CardSpec>,
    now: DateTime<Utc>,
) -> (Option<NaiveDate>, Option<NaiveDate>) {
    match (from, to, card) {
        (None, None, Some(card)) => {
            let today = now.date_naive();
            let days = i64::from(card.window_days);
            (Some(today - Duration::days(days - 1)), Some(today))
        }
        bounds => (bounds.0, bounds.1),
    }
}

fn validate_name(raw: &str) -> ApiResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ApiError::Validation("name is required".into()));
    }
    if trimmed.len() > 200 {
        return Err(ApiError::Validation(
            "name must be at most 200 characters".into(),
        ));
    }
    Ok(trimmed.to_string())
}

fn validate_description(raw: Option<&str>) -> ApiResult<()> {
    if let Some(d) = raw {
        if d.chars().count() > 2_000 {
            return Err(ApiError::Validation(
                "description must be at most 2000 characters".into(),
            ));
        }
    }
    Ok(())
}

// --- R6 builder surface: field catalog + spec runs ------------------------------

/// One catalog column: the static identifier plus its value class slug
/// (`uuid | text | timestamp | date | int | bigint | bool`). The slug is
/// what the builder UI keys operand typing on.
#[derive(Debug, Serialize, ToSchema)]
pub struct CatalogColumn {
    pub name: String,
    pub value_type: String,
}

/// The field catalog of ONE versioned view: every projected column plus the
/// narrower filterable inventory. Identifiers here are the ONLY ones a
/// builder may reference — the server re-validates every spec against the
/// same Rust catalog at save and run time.
#[derive(Debug, Serialize, ToSchema)]
pub struct CatalogView {
    pub view: ViewId,
    pub columns: Vec<CatalogColumn>,
    pub filterable: Vec<CatalogColumn>,
}

/// One card shape, described for the authoring agent (Wave 46 lane 1). The
/// prose is served, not duplicated in a prompt: a model that reads this
/// catalogue authors against the same rules the compiler enforces.
#[derive(Debug, Serialize, ToSchema)]
pub struct CatalogShape {
    /// `chart` | `stat` | `table`.
    pub shape: String,
    /// Viz hints that produce this shape in a spec.
    pub viz: Vec<String>,
    /// What the shape must be given: `rows`, `scalar`, `headers`.
    pub data: Vec<String>,
    pub default_span: String,
    pub default_height: String,
    /// Plain-English description the authoring model reads.
    pub agent_description: String,
    /// The refusal, in plain English, so the model chooses away from it.
    pub refuses_when: String,
}

/// GET /api/report-catalog envelope.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReportCatalog {
    pub views: Vec<CatalogView>,
    /// The card shapes both surfaces draw, with the prose the authoring
    /// agent reads.
    pub shapes: Vec<CatalogShape>,
    /// The card rules (zero rows, window versus refresh) in the same words
    /// the refusals use.
    pub card_rules: Vec<String>,
    /// Every span and height the grid accepts, so a picker cannot invent one.
    pub spans: Vec<String>,
    pub heights: Vec<String>,
}

fn value_type_slug(ty: core_reports::catalog::ValueType) -> &'static str {
    use core_reports::catalog::ValueType::*;
    match ty {
        Uuid => "uuid",
        Text => "text",
        Timestamp => "timestamp",
        Date => "date",
        Int | BigInt => "int",
        Bool => "bool",
    }
}

/// GET /api/report-catalog — the fixed field catalog for the R6 builder
/// (ADR 0001): per-view projected columns + filterable inventory with value
/// classes. Static data served from the same Rust constants the compiler
/// validates against, so the UI can never offer an identifier the backend
/// would reject.
#[utoipa::path(
    operation_id = "report_catalog",
    get,
    path = "/api/report-catalog",
    tag = "reports",
    security(("bearer" = [])),
    responses(
        (status = 200, body = ReportCatalog)
    )
)]
pub async fn catalog(extensions: Extensions) -> ApiResult<Json<ReportCatalog>> {
    let _authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &_authed.principal,
        Permission::Matrix {
            module: Module::Reporting,
            action: Action::View,
        },
    )?;
    let mut views = Vec::with_capacity(ViewId::ALL.len());
    for view in ViewId::ALL {
        let columns = view
            .columns()
            .iter()
            .map(|c| CatalogColumn {
                name: c.name.to_string(),
                value_type: value_type_slug(c.value_type).to_string(),
            })
            .collect();
        let filterable = view
            .filterable()
            .iter()
            .filter_map(|name| {
                view.columns()
                    .iter()
                    .find(|c| c.name == *name)
                    .map(|c| CatalogColumn {
                        name: c.name.to_string(),
                        value_type: value_type_slug(c.value_type).to_string(),
                    })
            })
            .collect();
        views.push(CatalogView {
            view,
            columns,
            filterable,
        });
    }
    // The card vocabulary travels with the field catalog: one description
    // per shape, the rules the refusals enforce, and the size names. Lane 5's
    // authoring prompt and the builder picker both read this, so neither can
    // carry a second, stale copy.
    let shapes = core_reports::shape_catalogue()
        .iter()
        .map(|info| CatalogShape {
            shape: info.shape.slug().to_string(),
            viz: info.viz.iter().map(|v| v.slug().to_string()).collect(),
            data: info.data.iter().map(|d| d.slug().to_string()).collect(),
            default_span: info.default_size.span.slug().to_string(),
            default_height: info.default_size.height.slug().to_string(),
            agent_description: info.agent_description.to_string(),
            refuses_when: info.refuses_when.to_string(),
        })
        .collect();
    let card_rules = core_reports::rules()
        .iter()
        .map(|r| (*r).to_string())
        .collect();
    let spans = core_reports::CardSpan::ALL
        .iter()
        .map(|s| s.slug().to_string())
        .collect();
    let heights = core_reports::CardHeight::ALL
        .iter()
        .map(|h| h.slug().to_string())
        .collect();
    Ok(Json(ReportCatalog {
        views,
        shapes,
        card_rules,
        spans,
        heights,
    }))
}

/// Query surface for running a stored definition (mirrors the prebuilt
/// kinds' loose GET shape; irrelevant fields ignored).
#[derive(Debug, Deserialize, utoipa::IntoParams, utoipa::ToSchema)]
pub struct ReportDefinitionRunQuery {
    /// Inclusive window start (UTC day, YYYY-MM-DD). Default: the stored
    /// card's `window_days` ending today, or 30 trailing days when the
    /// definition carries no card.
    pub from: Option<NaiveDate>,
    /// Inclusive window end (UTC day). Max span with `from`: 366 days.
    pub to: Option<NaiveDate>,
    /// Page size, clamped to [1, 500].
    pub limit: Option<i64>,
    /// `"csv"` streams text/csv instead of the JSON envelope.
    pub format: Option<String>,
}

/// GET /api/report-definitions/{id}/run — compile-on-demand execution of a
/// visible stored definition's spec (R6 builder runs). The spec re-validates
/// through the registry, compiles via `ReportSpec::compile_windowed`
/// (identifiers stay enum-matched catalog constants; values bind as `$N`
/// parameters), and executes against the reporting pool inside an
/// org-bound transaction. Non-admins are structurally self-scoped exactly
/// like the prebuilt kinds. A definition that carries a card defaults to the
/// card's window when the caller supplies no bounds; explicit bounds always
/// win.
#[utoipa::path(
    operation_id = "report_definitions_run",
    get,
    path = "/api/report-definitions/{id}/run",
    tag = "reports",
    security(("bearer" = [])),
    params(("id" = Uuid, Path), ReportDefinitionRunQuery),
    responses(
        (status = 200, description = "Report page (JSON envelope or CSV stream)", body = crate::routes::reports::ReportPage),
        (status = 404, description = "not visible in scope"),
        (status = 422, description = "spec failed registry validation or invalid window")
    )
)]
pub async fn run(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Query(q): Query<ReportDefinitionRunQuery>,
) -> ApiResult<Response> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Reporting,
            action: Action::View,
        },
    )?;
    let org_id = authed.principal.user.org_id;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let row = core_db::report_defs::get(&mut *tx, org_id, id)
        .await
        .map_err(|e| match e {
            core_db::DbError::NotFound => ApiError::NotFound,
            other => ApiError::from(other),
        })?;
    tx.commit().await.map_err(ApiError::from)?;
    if !visible(&authed.principal, &row) {
        return Err(ApiError::NotFound);
    }
    let spec = ReportSpec::from_parts(&row.kind, &row.spec, &row.viz).map_err(|e| {
        ApiError::Internal(anyhow::anyhow!("stored definition failed registry: {e}"))
    })?;
    let card = card_of(&row)?;

    core_reports::validate_limit_raw(q.limit.unwrap_or(core_reports::PAGE_LIMIT_DEFAULT))
        .map_err(|e| ApiError::Validation(e.to_string()))?;
    let limit = core_reports::clamp_limit(q.limit);
    let now = Utc::now();
    let (from, to) = run_window_bounds(q.from, q.to, card.as_ref(), now);
    let win = core_reports::resolve_window(from, to, now)
        .map_err(|e| ApiError::Validation(e.to_string()))?;
    let run_window = RunWindow {
        start: win.start,
        end: win.end,
    };

    // RBAC seam mirrors the prebuilt kinds: managers see org-wide, everyone
    // else is structurally narrowed onto their own rows.
    let scope = if authed.principal.is_owner_or_admin() {
        core_reports::Scope::OrgWide
    } else {
        core_reports::Scope::Self_(authed.principal.user.id)
    };

    let want_csv = matches!(q.format.as_deref(), Some("csv"));
    if let Some(other) = &q.format {
        if !want_csv {
            return Err(ApiError::Validation(format!(
                "unsupported format '{other}' (only 'csv')"
            )));
        }
    }

    let compiled = spec
        .compile_windowed(scope, limit, run_window)
        .map_err(|e| ApiError::Validation(e.to_string()))?;
    let rows =
        crate::routes::reports::run_rows(&state.reporting, &compiled.sql, &compiled.binds, org_id)
            .await?;
    let decoded: Vec<Value> = rows
        .iter()
        .map(|r| crate::routes::reports::row_json(r, &compiled.columns))
        .collect();

    match want_csv {
        false => {
            let summary = crate::routes::reports::run_rows(
                &state.reporting,
                &compiled.summary_sql,
                &compiled.summary_binds,
                org_id,
            )
            .await?;
            let summary_json = match summary.first() {
                Some(row) => crate::routes::reports::dynamic_row_json(row),
                None => serde_json::json!({}),
            };
            Ok(Json(crate::routes::reports::ReportPage {
                kind: spec.view.slug().to_string(),
                view: compiled.view.qualified_name().to_string(),
                columns: compiled
                    .columns
                    .iter()
                    .map(|c| c.name.to_string())
                    .collect(),
                rows: decoded,
                summary: summary_json,
                next_cursor: None,
            })
            .into_response())
        }
        true => Ok(crate::routes::reports::csv_response(
            spec.view.slug(),
            &compiled.columns,
            decoded,
        )),
    }
}

/// POST /api/reports/run-spec body: an inline spec (validated through the
/// same registry as stored definitions) plus the run window. This is the
/// builder's PREVIEW-before-save surface — nothing is persisted.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RunSpecRequest {
    /// The registry-validated report spec.
    pub spec: ReportSpec,
    /// Inclusive window start (UTC day, YYYY-MM-DD). Default: 30 trailing days.
    #[serde(default)]
    pub from: Option<NaiveDate>,
    /// Inclusive window end (UTC day). Max span with `from`: 366 days.
    #[serde(default)]
    pub to: Option<NaiveDate>,
    /// Page size, clamped to [1, 500].
    #[serde(default)]
    pub limit: Option<i64>,
    /// Optional card contract (Wave 46 lane 1): when present, the run is
    /// checked as a CARD and a card that cannot be drawn is a typed 422
    /// rather than an empty envelope. Absent means a plain report run, which
    /// is allowed to return zero rows.
    #[serde(default)]
    pub card: Option<core_reports::CardSpec>,
}

/// POST /api/reports/run-spec — execute an INLINE spec (R6 builder preview).
/// Same compile/execute/RBAC seam as running a stored definition; the spec
/// never touches storage and identifiers re-validate against the static
/// catalog before compilation.
#[utoipa::path(
    operation_id = "report_run_spec",
    post,
    path = "/api/reports/run-spec",
    tag = "reports",
    security(("bearer" = [])),
    request_body = RunSpecRequest,
    responses(
        (status = 200, body = crate::routes::reports::ReportPage),
        (status = 422, description = "spec failed registry validation or invalid window; with `card` present, a card that cannot be drawn is a 422 with `reason: card_contract`, naming the shape and the failure")
    )
)]
pub async fn run_spec(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<RunSpecRequest>,
) -> ApiResult<Response> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Reporting,
            action: Action::View,
        },
    )?;
    let org_id = authed.principal.user.org_id;
    core_reports::validate_limit_raw(req.limit.unwrap_or(core_reports::PAGE_LIMIT_DEFAULT))
        .map_err(|e| ApiError::Validation(e.to_string()))?;
    let limit = core_reports::clamp_limit(req.limit);
    let now = Utc::now();
    let win = core_reports::resolve_window(req.from, req.to, now)
        .map_err(|e| ApiError::Validation(e.to_string()))?;
    let run_window = RunWindow {
        start: win.start,
        end: win.end,
    };
    let scope = if authed.principal.is_owner_or_admin() {
        core_reports::Scope::OrgWide
    } else {
        core_reports::Scope::Self_(authed.principal.user.id)
    };
    let run = run_inline_spec(
        &state.reporting,
        org_id,
        scope,
        &req.spec,
        run_window,
        limit,
        req.card.as_ref(),
    )
    .await
    .map_err(inline_failure)?;
    Ok(Json(run).into_response())
}

/// Why an inline spec produced no page. Typed so the HTTP layer keeps its
/// contract refusal and the authoring verifier names the class instead of a
/// string it has to parse.
pub(crate) enum InlineSpecError {
    /// The spec failed registry validation or did not compile.
    Spec(core_reports::ReportsError),
    /// The card contract refused the finished run (or the card itself).
    Card(core_reports::ReportsError),
    /// The reporting pool could not answer the query.
    Db(sqlx::Error),
}

/// Compile one inline spec, run it on the reporting pool inside an org-bound
/// transaction, and check it as a card when one is supplied.
///
/// This is the ONLY inline execution path: `POST /api/reports/run-spec` calls
/// it, and the authoring verifier calls it, so a draft is checked by the query
/// the product would run rather than by a second implementation of the card
/// rules. The caller has already resolved the window and the caller's scope.
pub(crate) async fn run_inline_spec(
    pool: &sqlx::PgPool,
    org_id: Uuid,
    scope: core_reports::Scope,
    spec: &ReportSpec,
    window: RunWindow,
    limit: i64,
    card: Option<&CardSpec>,
) -> Result<crate::routes::reports::ReportPage, InlineSpecError> {
    let compiled = spec
        .compile_windowed(scope, limit, window)
        .map_err(InlineSpecError::Spec)?;
    let rows = crate::routes::reports::run_rows(pool, &compiled.sql, &compiled.binds, org_id)
        .await
        .map_err(InlineSpecError::Db)?;
    let decoded: Vec<Value> = rows
        .iter()
        .map(|r| crate::routes::reports::row_json(r, &compiled.columns))
        .collect();
    let summary = crate::routes::reports::run_rows(
        pool,
        &compiled.summary_sql,
        &compiled.summary_binds,
        org_id,
    )
    .await
    .map_err(InlineSpecError::Db)?;
    let summary_json = match summary.first() {
        Some(row) => crate::routes::reports::dynamic_row_json(row),
        None => serde_json::json!({}),
    };
    let columns: Vec<String> = compiled
        .columns
        .iter()
        .map(|c| c.name.to_string())
        .collect();
    // The card gate runs on the SAME run the product would render: a card
    // that would be handed back empty is a refusal here, not a blank frame
    // the reader has to interpret. The authoring verifier calls exactly this.
    if let Some(card) = card {
        // The card gate runs on the SAME run the product would render, and the
        // materialised card is discarded: what a caller needs to know is
        // whether the card could be drawn, and the contract's refusal is the
        // answer when it could not.
        core_reports::materialise(
            spec,
            card,
            &core_reports::CardRun {
                columns: &columns,
                rows: &decoded,
                summary: &summary_json,
            },
        )
        .map_err(InlineSpecError::Card)?;
    }
    Ok(crate::routes::reports::ReportPage {
        kind: spec.view.slug().to_string(),
        view: compiled.view.qualified_name().to_string(),
        columns,
        rows: decoded,
        summary: summary_json,
        next_cursor: None,
    })
}

/// Maps an inline-run failure onto the wire: a spec/limit/window problem is a
/// validation refusal, a card the contract cannot draw keeps its typed
/// `card_contract` shape, and a database error stays an internal failure.
fn inline_failure(err: InlineSpecError) -> ApiError {
    match err {
        InlineSpecError::Spec(e) => ApiError::Validation(e.to_string()),
        InlineSpecError::Card(e) => card_failure(e),
        InlineSpecError::Db(e) => ApiError::from(e),
    }
}

/// The production verification port: runs a candidate spec + card through
/// [`run_inline_spec`] — the same function `POST /api/reports/run-spec` uses —
/// on the request's reporting pool. Lives here because `AppState` and the
/// reporting pool do; the authoring loop only knows the trait.
pub(crate) struct ReportExecutionVerifier {
    pub pool: sqlx::PgPool,
    pub org_id: Uuid,
}

impl crate::routes::report_authoring::DraftVerifier for ReportExecutionVerifier {
    fn verify<'a>(
        &'a self,
        spec: &'a ReportSpec,
        card: &'a CardSpec,
        scope: core_reports::Scope,
        window: RunWindow,
        card_index: Option<usize>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Vec<crate::routes::report_authoring::DraftFinding>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            use crate::routes::report_authoring::DraftFinding;
            match run_inline_spec(
                &self.pool,
                self.org_id,
                scope,
                spec,
                window,
                core_reports::PAGE_LIMIT_DEFAULT,
                Some(card),
            )
            .await
            {
                Ok(_) => Vec::new(),
                Err(InlineSpecError::Card(error) | InlineSpecError::Spec(error)) => {
                    vec![crate::routes::report_authoring::finding_from_reports_error(
                        card_index, &error,
                    )]
                }
                Err(InlineSpecError::Db(error)) => vec![DraftFinding::new(
                    "run_failed",
                    card_index,
                    format!("The report could not run against the reporting data: {error}"),
                )],
            }
        })
    }
}

/// Maps a card-contract refusal onto the wire: an empty card keeps its
/// shape and failure class as machine-readable fields, so a repair loop can
/// say "the chart has no rows" without parsing prose. Everything else (a
/// window/refresh or bounds violation) is an ordinary validation refusal.
fn card_failure(err: core_reports::ReportsError) -> ApiError {
    let message = err.to_string();
    match err {
        core_reports::ReportsError::EmptyCard { shape, failure } => ApiError::CardContract {
            shape: shape.slug().to_string(),
            failure: failure.slug().to_string(),
            message,
        },
        other => ApiError::Validation(other.to_string()),
    }
}

/// Guard against catalog drift: every builder-facing surface is derived from
/// the SAME static catalog the compiler validates against. This test pins
/// that the serialized catalog covers every view exactly once and that the
/// filterable inventory is a subset of the projected columns.
#[cfg(test)]
mod catalog_tests {
    use super::*;

    #[test]
    fn catalog_covers_all_views_and_filterable_subset() {
        for view in ViewId::ALL {
            let names: Vec<&str> = view.columns().iter().map(|c| c.name).collect();
            for f in view.filterable() {
                assert!(
                    names.contains(f),
                    "{view:?}: filterable {f:?} not projected"
                );
            }
        }
    }

    #[test]
    fn group_by_and_measure_caps_are_advertised() {
        assert_eq!(core_reports::MAX_GROUP_BY, 3);
        assert_eq!(core_reports::MAX_MEASURES, 4);
        assert_eq!(core_reports::Aggregate::Count.slug(), "count");
    }

    /// The catalogue is what the authoring agent reads, so it must describe
    /// every shape the renderer can be handed, name every viz the registry
    /// accepts, and carry the size vocabulary lane 6 picks from. A shape
    /// missing here is a shape the model cannot choose.
    #[test]
    fn the_card_catalogue_describes_every_shape_and_viz() {
        let shapes = core_reports::shape_catalogue();
        for shape in core_reports::CardShape::ALL {
            assert!(
                shapes.iter().any(|s| s.shape == shape),
                "{shape:?} has no catalogue entry"
            );
        }
        for viz in core_reports::VizHint::ALL {
            assert!(
                shapes.iter().any(|s| s.viz.contains(&viz)),
                "{viz:?} maps onto no shape"
            );
        }
        assert_eq!(core_reports::rules().len(), 3);
        assert_eq!(core_reports::CardSpan::ALL.len(), 4);
        assert_eq!(core_reports::CardHeight::ALL.len(), 3);
    }

    /// A refusal a caller cannot branch on is a prose message with extra
    /// steps. The empty-card refusal must survive the mapping with its shape
    /// and failure intact, and the timing refusals must stay validation.
    #[test]
    fn an_empty_card_maps_onto_the_typed_refusal() {
        let empty = card_failure(core_reports::ReportsError::EmptyCard {
            shape: core_reports::CardShape::Chart,
            failure: core_reports::CardFailure::NoRows,
        });
        match empty {
            ApiError::CardContract { shape, failure, .. } => {
                assert_eq!(shape, "chart");
                assert_eq!(failure, "no_rows");
            }
            other => panic!("unexpected mapping {other:?}"),
        }

        let timing = card_failure(core_reports::ReportsError::WindowWiderThanRefresh {
            window_days: 30,
            reach_days: 7,
        });
        assert!(matches!(timing, ApiError::Validation(_)));
    }
}

/// The stored-definition run's default window (F103): a card's `window_days`
/// seeds the span when the caller supplies no bounds, and nothing else about
/// the resolution moves.
#[cfg(test)]
mod card_window_tests {
    use super::*;
    use chrono::TimeZone;
    use core_reports::{CardHeight, CardRefresh, CardSize, CardSpan};

    fn anchor() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 15, 12, 0, 0).unwrap()
    }

    fn card(window_days: u32) -> CardSpec {
        CardSpec {
            size: CardSize {
                span: CardSpan::Half,
                height: CardHeight::Regular,
            },
            window_days,
            refresh: CardRefresh {
                every_minutes: 60,
                reach_days: 0,
            },
        }
    }

    #[test]
    fn a_card_supplies_the_default_window_and_the_caller_keeps_its_own() {
        let today = anchor().date_naive();
        let seven = card(7);

        assert_eq!(
            run_window_bounds(None, None, Some(&seven), anchor()),
            (Some(today - Duration::days(6)), Some(today)),
            "no caller bounds: the card's seven days, ending today"
        );
        assert_eq!(
            run_window_bounds(None, None, None, anchor()),
            (None, None),
            "no card: the registry's trailing-30-day default is untouched"
        );

        let from = today - Duration::days(20);
        assert_eq!(
            run_window_bounds(Some(from), None, Some(&seven), anchor()),
            (Some(from), None),
            "one caller bound still collapses to that single day"
        );
        let to = today - Duration::days(2);
        assert_eq!(
            run_window_bounds(Some(from), Some(to), Some(&seven), anchor()),
            (Some(from), Some(to)),
            "both caller bounds win outright"
        );
    }
}
