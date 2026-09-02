//! Report definitions CRUD (TASKLIST R3; ADR 0001 §Decision).
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
//! Execution: deliberately absent. Definitions are windowless templates;
//! running them (R6 builder / R4 delivery) compiles on demand via
//! `core_reports::ReportSpec::compile` against the reporting pool. This
//! module never executes SQL against report data.
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
    extract::{Path, State},
    http::Extensions,
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use core_reports::ReportSpec;
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
    /// Requested visibility; `org` requires admin and non-admins are
    /// forced onto `self` regardless.
    #[serde(default)]
    pub scope: Option<DefinitionScope>,
    #[serde(flatten)]
    pub instant: MutationParams,
}

/// PUT /api/report-definitions/{id} body — full-replace semantics: every
/// member is applied; `description: null` clears it.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UpdateReportDefinitionRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub spec: ReportSpec,
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
    let rows = if authed.principal.is_owner_or_admin() {
        core_db::report_defs::list_all(&state.pool, org_id).await?
    } else {
        core_db::report_defs::list_visible(&state.pool, org_id, authed.principal.user.id).await?
    };
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
    let row = core_db::report_defs::get(&state.pool, org_id, id)
        .await
        .map_err(|e| match e {
            core_db::DbError::NotFound => ApiError::NotFound,
            other => ApiError::from(other),
        })?;
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
    let spec = ReportSpec::from_parts(&row.kind, &row.spec, &row.viz).map_err(|e| {
        ApiError::Internal(anyhow::anyhow!("stored definition failed registry: {e}"))
    })?;
    Ok(ReportDefinitionView {
        id: row.id,
        name: row.name,
        description: row.description,
        spec,
        scope: DefinitionScope::parse(&row.scope).unwrap_or(DefinitionScope::Self_),
        created_by: row.created_by,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
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
