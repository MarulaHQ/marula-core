//! Template registry (Wave 13 lane 5 — KB3/KB4, ADR 0004 "template
//! registry", planning A13).
//!
//! Purpose: the one `Template` entity, typed across ticket / note / email /
//! KB / quote-invoice / project / workflow. Shipped defaults are immutable
//! seed rows (`is_system` + `builtin`, mirroring lane 6's preset-roles
//! pattern): orgs fork, never edit in place, and can restore a forked
//! preset row to its canonical copy. Org-created templates are first-class.
//! Render resolves typed variables (`{{client.name}}`, `{{ticket.number}}`)
//! from real entity context via the closed per-kind vocabulary in
//! `core_domain::templates` — never raw string interpolation of user data;
//! an out-of-vocabulary placeholder is a typed 400.
//!
//! Permissions: reads render behind `admin:view`; create/fork/patch/delete/
//! restore behind `admin:create` / `admin:edit` / `admin:delete`. Mutations
//! ride the IF1–IF3 instant-first envelope (idempotency-keyed claim,
//! version-gated seq, read-your-write `mutation` metadata) and emit
//! `template.created` / `template.updated` / `template.removed` + audit
//! rows in the same commit.
//!
//! Non-purpose: no per-type template tables (one entity), no auto-insertion
//! into tickets/emails yet (callers fetch the rendered subject/body through
//! `render` and apply them in their own flows).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use core_db::templates::TemplateEntry;
use core_domain::entities::templates::{TemplateKind, TemplateVariable};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::events::types;
use core_domain::policy::{check_any, Action, Module, Permission};
use core_domain::templates::{self as tmpl, RenderError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

const ENTITY: &str = "template";

fn admin(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action,
    }
}

fn require_reader(principal: &core_domain::entities::Principal) -> ApiResult<()> {
    check_any(&principal.permissions, &[admin(Action::View)])
        .expect()
        .map_err(ApiError::Forbidden)
}

// --- wire structs ------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TemplateVariableView {
    pub key: String,
    pub label: String,
}

/// One template row: a shipped default (`is_system` + `builtin`), a fork
/// (`forked_from`), or an org-created template.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TemplateView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub org_id: Uuid,
    pub kind: String,
    pub slug: String,
    pub name: String,
    pub description: String,
    #[schema(nullable = true)]
    pub subject: Option<String>,
    pub body: String,
    pub variables: Vec<TemplateVariableView>,
    /// Shipped preset tag, `null` for org-created rows.
    #[schema(nullable = true)]
    pub builtin: Option<String>,
    /// Shipped defaults are system data: immutable, restorable, never
    /// editable or deletable in place — orgs fork.
    pub is_system: bool,
    /// For a forked copy: the origin's builtin tag.
    #[schema(nullable = true)]
    pub forked_from: Option<String>,
    #[schema(value_type = String)]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[schema(value_type = String)]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl TemplateView {
    fn from_entry(e: TemplateEntry) -> ApiResult<Self> {
        let kind = TemplateKind::parse(&e.kind).ok_or_else(|| {
            ApiError::Internal(anyhow::anyhow!("template row carries unknown kind"))
        })?;
        let variables: Vec<TemplateVariable> = serde_json::from_value(e.variables.clone())
            .map_err(|_| {
                ApiError::Internal(anyhow::anyhow!("template row carries invalid variables"))
            })?;
        Ok(Self {
            id: e.id,
            org_id: e.org_id,
            kind: kind.as_str().to_string(),
            slug: e.slug,
            name: e.name,
            description: e.description,
            subject: e.subject,
            body: e.body,
            variables: variables
                .into_iter()
                .map(|v| TemplateVariableView {
                    key: v.key,
                    label: v.label,
                })
                .collect(),
            builtin: e.builtin,
            is_system: e.is_system,
            forked_from: e.forked_from,
            created_at: e.created_at,
            updated_at: e.updated_at,
        })
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TemplateListQuery {
    /// Filter to one template kind (snake_case: `quote_invoice`, …).
    pub kind: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TemplatesListResponse {
    pub templates: Vec<TemplateView>,
    /// The closed per-kind variable vocabulary, for editors: kind → keys.
    pub vocabulary: Vec<TemplateKindVocabulary>,
}

/// The closed variable vocabulary for one kind (editor UX + validation
/// source of truth: `core_domain::templates::variables_for_kind`).
#[derive(Debug, Serialize, ToSchema)]
pub struct TemplateKindVocabulary {
    pub kind: String,
    pub variables: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateTemplateRequest {
    pub kind: String,
    /// URL-safe unique slug within org + kind (typed 409
    /// `template_slug_taken` otherwise).
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub subject: Option<String>,
    pub body: String,
    /// Declared variables; every key must be inside the kind's closed
    /// vocabulary (typed 400 `template_unknown_variable` otherwise).
    #[serde(default)]
    pub variables: Vec<TemplateVariableView>,
    /// Instant-first autosave parameters (IF1): idempotency key. Absent
    /// in legacy clients — purely additive. (Creates carry no version.)
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ForkTemplateRequest {
    pub slug: String,
    pub name: String,
    /// Instant-first autosave parameters (IF1): idempotency key.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateTemplateRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub variables: Option<Vec<TemplateVariableView>>,
    /// Instant-first autosave parameters (IF1): idempotency key + version.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RestoreTemplateRequest {
    /// Instant-first autosave parameters (IF1): idempotency key + version.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// Instant-first mutation response (IF2): the template at the top level
/// plus the additive `mutation` metadata.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TemplateMutationResponse {
    #[serde(flatten)]
    pub template: TemplateView,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RenderTemplateRequest {
    /// The entity the variables resolve from: a ticket id for
    /// ticket/note/email/workflow kinds, a project id for `project`, a
    /// quote id for `quote_invoice`. `null` (or omitted) for `kb`.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub context_id: Option<Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RenderedTemplate {
    #[schema(nullable = true)]
    pub subject: Option<String>,
    pub body: String,
}

// --- validation helpers --------------------------------------------------------

fn parse_kind(kind: &str) -> ApiResult<TemplateKind> {
    TemplateKind::parse(kind).ok_or_else(|| {
        ApiError::Validation(format!(
            "unknown template kind `{kind}`; expected one of ticket, note, email, kb, quote_invoice, project, workflow"
        ))
    })
}

fn validate_slug(slug: &str) -> ApiResult<()> {
    let ok = !slug.is_empty()
        && slug.len() <= 64
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && slug
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if ok {
        Ok(())
    } else {
        Err(ApiError::Validation(
            "slug must be 1..=64 lowercase alphanumeric characters, dashes or underscores, starting alphanumeric".into(),
        ))
    }
}

fn validate_name(name: &str) -> ApiResult<()> {
    if name.trim().is_empty() || name.len() > 128 {
        return Err(ApiError::Validation(
            "template name is required (1..=128 characters)".into(),
        ));
    }
    Ok(())
}

/// Declared variables must live inside the kind's closed vocabulary (KB3:
/// typed variables, never free-form interpolation slots).
fn validate_variables(kind: TemplateKind, vars: &[TemplateVariableView]) -> ApiResult<()> {
    let keys: Vec<String> = vars.iter().map(|v| v.key.clone()).collect();
    for v in vars {
        if v.label.trim().is_empty() || v.label.len() > 128 {
            return Err(ApiError::Validation(
                "each template variable needs a label (1..=128 characters)".into(),
            ));
        }
    }
    tmpl::validate_variables(kind, &keys).map_err(|e| match e {
        RenderError::UnknownVariable(key, k) => ApiError::Validation(format!(
            "template variable `{key}` is not in the closed vocabulary for kind `{k}`"
        )),
        other => ApiError::Validation(other.to_string()),
    })
}

fn variables_json(vars: &[TemplateVariableView]) -> Value {
    serde_json::to_value(vars).unwrap_or(Value::Array(vec![]))
}

fn render_error(e: RenderError) -> ApiError {
    match e {
        RenderError::UnknownVariable(key, kind) => ApiError::Validation(format!(
            "template variable `{key}` is not in the closed vocabulary for kind `{kind}`"
        )),
        other => ApiError::Validation(other.to_string()),
    }
}

// --- handlers ------------------------------------------------------------------

/// GET /api/templates — the org's templates (shipped defaults first, then
/// org-owned rows), optionally filtered by kind.
#[utoipa::path(operation_id = "templates_list", get, path = "/api/templates", tag = "admin", security(("bearer" = [])),
    params(("kind" = Option<String>, Query, description = "filter to one template kind"),
           ("limit" = Option<i64>, Query), ("offset" = Option<i64>, Query)),
    responses((status = 200, body = TemplatesListResponse),
              (status = 403, description = "requires admin:view")))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<TemplateListQuery>,
) -> ApiResult<Json<TemplatesListResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_reader(&authed.principal)?;
    let org_id = authed.principal.user.org_id;
    let kind = q.kind.as_deref().map(str::trim).filter(|k| !k.is_empty());
    if let Some(k) = kind {
        parse_kind(k)?;
    }
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let entries = core_db::templates::list_templates(&mut *tx, org_id, kind, limit, offset).await?;
    tx.commit().await.map_err(ApiError::from)?;

    let templates = entries
        .into_iter()
        .map(TemplateView::from_entry)
        .collect::<ApiResult<Vec<_>>>()?;
    let vocabulary = TemplateKind::ALL
        .iter()
        .map(|k| TemplateKindVocabulary {
            kind: k.as_str().to_string(),
            variables: tmpl::variables_for_kind(*k)
                .iter()
                .map(|s| s.to_string())
                .collect(),
        })
        .collect();
    Ok(Json(TemplatesListResponse {
        templates,
        vocabulary,
    }))
}

/// GET /api/templates/{id}
#[utoipa::path(operation_id = "templates_get", get, path = "/api/templates/{id}", tag = "admin", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = TemplateView),
              (status = 403, description = "requires admin:view"),
              (status = 404, description = "template not visible in scope")))]
pub async fn get(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<TemplateView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_reader(&authed.principal)?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let entry = core_db::templates::get_template(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(TemplateView::from_entry(entry)?))
}

/// POST /api/templates — create an org-owned template (shipped defaults
/// are seed data; fork them instead of copying by hand).
#[utoipa::path(operation_id = "templates_create", post, path = "/api/templates", tag = "admin", security(("bearer" = [])),
    request_body = CreateTemplateRequest,
    responses((status = 200, body = TemplateMutationResponse),
              (status = 400, description = "validation failure (unknown kind, bad slug, out-of-vocabulary variable)"),
              (status = 403, description = "requires admin:create"),
              (status = 409, description = "slug taken or idempotency key reused")))]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateTemplateRequest>,
) -> ApiResult<Json<TemplateMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Create))?;
    let kind = parse_kind(&req.kind)?;
    let slug = req.slug.trim().to_string();
    let name = req.name.trim().to_string();
    validate_slug(&slug)?;
    validate_name(&name)?;
    if req.body.trim().is_empty() || req.body.len() > 64_000 {
        return Err(ApiError::Validation(
            "template body is required (1..=64000 characters)".into(),
        ));
    }
    if req.subject.as_deref().is_some_and(str::is_empty) {
        return Err(ApiError::Validation(
            "subject must be null or non-empty".into(),
        ));
    }
    validate_variables(kind, &req.variables)?;
    tmpl::validate_content(kind, req.subject.as_deref(), &req.body).map_err(render_error)?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let vars = variables_json(&req.variables);
    let body = req.body.clone();
    let subject = req.subject.clone();
    let description = req.description.trim().to_string();
    let kind_str = kind.as_str().to_string();

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec { entity: ENTITY, entity_id: None, key: key.as_deref(), fingerprint: &fingerprint },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let id = Uuid::now_v7();
        let entry = core_db::templates::insert_template(
            &mut ctx.tx, id, org_id, &kind_str, &slug, &name, &description,
            subject.as_deref(), &body, &vars, None,
        )
        .await?;

        ctx.emit(
            types::TEMPLATE_CREATED,
            ENTITY,
            Some(entry.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TemplateCreated {
                template_id: entry.id,
                kind: entry.kind.clone(),
                slug: entry.slug.clone(),
                from_builtin: None,
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(actor_id),
                action: "template.created".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(entry.id.to_string()),
                detail: serde_json::json!({ "kind": entry.kind, "slug": entry.slug, "name": entry.name }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec { entity: ENTITY, entity_id: entry.id, version: None },
        )
        .await?;
        let view = TemplateView::from_entry(entry)?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: view.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((ctx, MutationOutcome::Applied(TemplateMutationResponse { template: view, mutation })))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/templates/{id}/fork — duplicate a template (typically a
/// shipped default) into a mutable org-owned copy. The origin row is
/// untouched.
#[utoipa::path(operation_id = "templates_fork", post, path = "/api/templates/{id}/fork", tag = "admin", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = ForkTemplateRequest,
    responses((status = 200, body = TemplateMutationResponse),
              (status = 404, description = "template not visible in scope"),
              (status = 409, description = "slug taken or idempotency key reused"),
              (status = 422, description = "validation failure")))]
pub async fn fork(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<ForkTemplateRequest>,
) -> ApiResult<Json<TemplateMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Create))?;
    let slug = req.slug.trim().to_string();
    let name = req.name.trim().to_string();
    validate_slug(&slug)?;
    validate_name(&name)?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
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
        let origin = core_db::templates::get_template(&mut *ctx.tx, org_id, id)
            .await
            .map_err(ApiError::from)
            .map_err(instant::mutation_scope_err)?;
        // The fork inherits the origin's declared variables verbatim; they
        // were vocabulary-validated when the origin was written.
        let forked_from = origin.builtin.clone().or_else(|| Some(origin.slug.clone()));
        let copy = core_db::templates::insert_template(
            &mut ctx.tx,
            Uuid::now_v7(),
            org_id,
            &origin.kind,
            &slug,
            &name,
            &origin.description,
            origin.subject.as_deref(),
            &origin.body,
            &origin.variables,
            forked_from.as_deref(),
        )
        .await?;

        ctx.emit(
            types::TEMPLATE_CREATED,
            ENTITY,
            Some(copy.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TemplateCreated {
                template_id: copy.id,
                kind: copy.kind.clone(),
                slug: copy.slug.clone(),
                from_builtin: origin.builtin.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(actor_id),
                action: "template.forked".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(copy.id.to_string()),
                detail: serde_json::json!({
                    "name": copy.name,
                    "from_template_id": origin.id,
                    "from_builtin": origin.builtin,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: copy.id,
                version: None,
            },
        )
        .await?;
        let view = TemplateView::from_entry(copy)?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: view.id,
                op: "fork",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TemplateMutationResponse {
                template: view,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/templates/{id} — edit an org-owned template's content.
/// Shipped defaults are immutable (typed 409 `template_immutable`): fork
/// first, edit the fork.
#[utoipa::path(operation_id = "templates_update", patch, path = "/api/templates/{id}", tag = "admin", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = UpdateTemplateRequest,
    responses((status = 200, body = TemplateMutationResponse),
              (status = 404, description = "template not visible in scope"),
              (status = 409, description = "template immutable, version conflict, or idempotency key reused"),
              (status = 422, description = "validation failure")))]
pub async fn update(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateTemplateRequest>,
) -> ApiResult<Json<TemplateMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
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
        let before = core_db::templates::get_template(&mut *ctx.tx, org_id, id)
            .await
            .map_err(ApiError::from)
            .map_err(instant::mutation_scope_err)?;
        if before.is_system {
            return Err(ApiError::UniqueConflict {
                kind: "template_immutable",
                message:
                    "shipped default templates are immutable — fork the template and edit the fork",
            });
        }
        let kind = TemplateKind::parse(&before.kind).ok_or_else(|| {
            ApiError::Internal(anyhow::anyhow!("template row carries unknown kind"))
        })?;
        let name = req.name.clone().unwrap_or_else(|| before.name.clone());
        validate_name(&name)?;
        let description = req
            .description
            .clone()
            .unwrap_or_else(|| before.description.clone());
        let subject = req.subject.clone().or_else(|| before.subject.clone());
        let body = req.body.clone().unwrap_or_else(|| before.body.clone());
        if body.trim().is_empty() || body.len() > 64_000 {
            return Err(ApiError::Validation(
                "template body is required (1..=64000 characters)".into(),
            ));
        }
        if subject.as_deref().is_some_and(str::is_empty) {
            return Err(ApiError::Validation(
                "subject must be null or non-empty".into(),
            ));
        }
        let vars_view = req.variables.clone().unwrap_or_else(|| {
            serde_json::from_value::<Vec<TemplateVariable>>(before.variables.clone())
                .unwrap_or_default()
                .into_iter()
                .map(|v| TemplateVariableView {
                    key: v.key,
                    label: v.label,
                })
                .collect()
        });
        validate_variables(kind, &vars_view)?;
        tmpl::validate_content(kind, subject.as_deref(), &body).map_err(render_error)?;

        let after = core_db::templates::update_template(
            &mut ctx.tx,
            org_id,
            id,
            &name,
            &description,
            subject.as_deref(),
            &body,
            &variables_json(&vars_view),
        )
        .await
        .map_err(ApiError::from)
        .map_err(instant::mutation_scope_err)?;

        ctx.emit(
            types::TEMPLATE_UPDATED,
            ENTITY,
            Some(after.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TemplateUpdated {
                template_id: after.id,
                kind: after.kind.clone(),
                slug: after.slug.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(actor_id),
                action: "template.updated".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(after.id.to_string()),
                detail: serde_json::json!({ "kind": after.kind, "slug": after.slug }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: after.id,
                version: req.instant.version,
            },
        )
        .await?;
        let view = TemplateView::from_entry(after.clone())?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: view.id,
                op: "update",
                seq: write_seq,
                before: serde_json::to_value(TemplateView::from_entry(before)?).ok(),
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TemplateMutationResponse {
                template: view,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/templates/{id}/restore — reset a shipped default's content to
/// the canonical copy (KB4: presets stay canonical, orgs fork). Org-owned
/// rows and forks are org-owned data and are never restorable.
#[utoipa::path(operation_id = "templates_restore", post, path = "/api/templates/{id}/restore", tag = "admin", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = RestoreTemplateRequest,
    responses((status = 200, body = TemplateMutationResponse),
              (status = 404, description = "template not visible in scope, or not a shipped default"),
              (status = 409, description = "version conflict or idempotency key reused")))]
pub async fn restore(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<RestoreTemplateRequest>,
) -> ApiResult<Json<TemplateMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
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
        let before = core_db::templates::get_template(&mut *ctx.tx, org_id, id)
            .await
            .map_err(ApiError::from)
            .map_err(instant::mutation_scope_err)?;
        if !(before.is_system && before.builtin.is_some()) {
            return Err(ApiError::Validation(
                "only shipped default templates are restorable; org-owned rows are org data".into(),
            ));
        }
        let canonical = tmpl::DEFAULT_TEMPLATES
            .iter()
            .find(|d| Some(d.builtin) == before.builtin.as_deref())
            .ok_or_else(|| {
                ApiError::Internal(anyhow::anyhow!(
                    "system template row carries an unknown builtin tag"
                ))
            })?;

        let restored = core_db::templates::restore_default_template(
            &mut ctx.tx,
            org_id,
            id,
            canonical.subject,
            canonical.body,
        )
        .await
        .map_err(ApiError::from)
        .map_err(instant::mutation_scope_err)?;

        ctx.emit(
            types::TEMPLATE_UPDATED,
            ENTITY,
            Some(restored.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TemplateUpdated {
                template_id: restored.id,
                kind: restored.kind.clone(),
                slug: restored.slug.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(actor_id),
                action: "template.restored".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(restored.id.to_string()),
                detail: serde_json::json!({ "kind": restored.kind, "builtin": restored.builtin }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: restored.id,
                version: req.instant.version,
            },
        )
        .await?;
        let view = TemplateView::from_entry(restored.clone())?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: view.id,
                op: "restore",
                seq: write_seq,
                before: serde_json::to_value(TemplateView::from_entry(before)?).ok(),
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TemplateMutationResponse {
                template: view,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/templates/{id} — remove an org-owned template. Shipped
/// defaults are immutable (typed 409 `template_immutable`).
#[utoipa::path(operation_id = "templates_delete", delete, path = "/api/templates/{id}", tag = "admin", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = TemplateMutationResponse),
              (status = 404, description = "template not visible in scope"),
              (status = 409, description = "template immutable or idempotency key reused")))]
pub async fn delete(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<TemplateMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: ENTITY,
                entity_id: Some(id),
                key: None,
                fingerprint: &Value::Null,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::templates::get_template(&mut *ctx.tx, org_id, id)
            .await
            .map_err(ApiError::from)
            .map_err(instant::mutation_scope_err)?;
        if before.is_system {
            return Err(ApiError::UniqueConflict {
                kind: "template_immutable",
                message: "shipped default templates are immutable and cannot be deleted",
            });
        }
        let removed = core_db::templates::delete_template(&mut ctx.tx, org_id, id)
            .await
            .map_err(ApiError::from)
            .map_err(instant::mutation_scope_err)?;

        ctx.emit(
            types::TEMPLATE_REMOVED,
            ENTITY,
            Some(removed.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TemplateRemoved {
                template_id: removed.id,
                kind: removed.kind.clone(),
                slug: removed.slug.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(actor_id),
                action: "template.removed".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(removed.id.to_string()),
                detail: serde_json::json!({ "kind": removed.kind, "slug": removed.slug }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: removed.id,
                version: None,
            },
        )
        .await?;
        let view = TemplateView::from_entry(removed.clone())?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: view.id,
                op: "delete",
                seq: write_seq,
                before: serde_json::to_value(&view).ok(),
                after: Value::Null,
                updated_at: chrono::Utc::now(),
                key: None,
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TemplateMutationResponse {
                template: view,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/templates/{id}/render — resolve the template's typed
/// variables against real entity context and return the rendered
/// subject/body. Out-of-vocabulary placeholders are typed 400s, never
/// silent empties (KB3).
#[utoipa::path(operation_id = "templates_render", post, path = "/api/templates/{id}/render", tag = "admin", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = RenderTemplateRequest,
    responses((status = 200, body = RenderedTemplate),
              (status = 400, description = "out-of-vocabulary or malformed variable"),
              (status = 403, description = "requires admin:view"),
              (status = 404, description = "template or context entity not visible in scope")))]
pub async fn render(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    req: Option<Json<RenderTemplateRequest>>,
) -> ApiResult<Json<RenderedTemplate>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_reader(&authed.principal)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let context_id = req.and_then(|Json(r)| r.context_id);

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    let entry = core_db::templates::get_template(&mut *tx, org_id, id).await?;
    let kind = TemplateKind::parse(&entry.kind)
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("template row carries unknown kind")))?;

    let row = match kind {
        TemplateKind::Ticket
        | TemplateKind::Note
        | TemplateKind::Email
        | TemplateKind::Workflow => {
            let cid = context_id.ok_or_else(|| {
                ApiError::Validation(
                    "this template kind renders from a ticket — pass context_id".into(),
                )
            })?;
            core_db::templates::ticket_render_context(&mut *tx, org_id, cid, user_id).await?
        }
        TemplateKind::Project => {
            let cid = context_id.ok_or_else(|| {
                ApiError::Validation(
                    "project templates render from a project — pass context_id".into(),
                )
            })?;
            core_db::templates::project_render_context(&mut *tx, org_id, cid, user_id).await?
        }
        TemplateKind::QuoteInvoice => {
            let cid = context_id.ok_or_else(|| {
                ApiError::Validation(
                    "quote-invoice templates render from a quote — pass context_id".into(),
                )
            })?;
            core_db::templates::quote_render_context(&mut *tx, org_id, cid, user_id).await?
        }
        TemplateKind::Kb => {
            core_db::templates::org_render_context(&mut *tx, org_id, user_id).await?
        }
    };
    tx.commit().await.map_err(ApiError::from)?;

    let (subject, body) =
        tmpl::render_template(kind, entry.subject.as_deref(), &entry.body, &row.to_map())
            .map_err(render_error)?;
    Ok(Json(RenderedTemplate { subject, body }))
}
