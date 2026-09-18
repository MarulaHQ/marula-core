//! AI availability matrix (ADR 0008, Wave 18 lane 1): the policy +
//! ledger surface every AI-assisted area reads.
//!
//! Purpose:
//! - Admin (`/settings/ai/...`): curate the model catalog, grant
//!   per-surface access per role/user (deny-by-default, audited), set
//!   org-wide + per-user soft/hard token caps, manage ORG credentials,
//!   and read the org usage roll-up. Every mutation rides the
//!   instant-first write path (IF1–IF3) and emits its contract event +
//!   audit row.
//! - User (`/ai/me/...`): see granted surfaces, manage PERSONAL sealed
//!   credentials (gated by the `ai.personal_credentials_enabled` policy
//!   flag), keep sticky per-surface model picks inside the admin
//!   allowlist, read personal usage, and resolve calls through the
//!   completion edge (`POST /ai/me/authorize`).
//! - Completion edge: surface grant → model resolution (user pick → org
//!   pick → legacy flat pair while still allowlisted → first enabled
//!   catalog entry) → caps (hard = typed refusal BEFORE dispatch,
//!   degrade-never-5xx; soft = warn + notify) → sealed credential slot.
//!   Lane 2's ChatRunner consumes this same edge.
//!
//! Non-purpose: secret bytes NEVER cross back as responses, logs, trail
//! rows, or event payloads; no chat transport or tool loop (lane 2).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome, MutationParams};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use core_db::ai_policy::CredentialOwnerFilter;
use core_domain::entities::{Actor, AuditEvent, Principal};
use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

const GRANT_ENTITY: &str = "ai_surface_grant";
const CATALOG_ENTITY: &str = "ai_model_catalog";
const CREDENTIAL_ENTITY: &str = "ai_model_credential";
const CAP_ENTITY: &str = "ai_cap";
const PICK_ENTITY: &str = "ai_model_pick";

/// The period v1 caps + progress read models run on (`ai_caps.period`).
const PERIOD: &str = "month";

fn require_ai_admin(principal: &Principal, action: Action) -> ApiResult<()> {
    require_permission(
        principal,
        Permission::Matrix {
            module: Module::Admin,
            action,
        },
    )?;
    if !principal.is_owner_or_admin() {
        return Err(ApiError::Forbidden(
            "the AI matrix is an owner/admin surface".into(),
        ));
    }
    Ok(())
}

pub(super) fn month_start(now: chrono::DateTime<chrono::Utc>) -> chrono::DateTime<chrono::Utc> {
    use chrono::Datelike;
    chrono::NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
        .unwrap_or_else(|| chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(chrono::Utc)
        .single()
        .unwrap_or(now)
}

fn pct_of(used: i64, limit: Option<i64>) -> Option<f64> {
    match limit {
        Some(limit) if limit > 0 => Some(((used as f64) / (limit as f64) * 100.0).min(100.0)),
        _ => None,
    }
}

fn uid_opt_json(v: Option<Uuid>) -> Value {
    v.map_or(Value::Null, |id| Value::String(id.to_string()))
}

async fn audit_row(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    actor_id: Uuid,
    action: &str,
    entity_type: &str,
    entity_id: Uuid,
    detail: Value,
) -> ApiResult<()> {
    core_db::audit::insert(
        tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::Human(actor_id),
            action: action.to_string(),
            entity_type: Some(entity_type.to_string()),
            entity_id: Some(entity_id.to_string()),
            detail,
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await
    .map_err(ApiError::from)
}

/// RLS-bound read tx for the GET surfaces (settings table policies demand
/// app.org_id and the raw pool carries no GUC).
async fn org_bound_tx(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
) -> ApiResult<sqlx::Transaction<'static, sqlx::Postgres>> {
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    Ok(tx)
}

// --- DTOs ------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AiSurfaceDto {
    pub id: Uuid,
    #[schema(value_type = String, nullable = true)]
    pub org_id: Option<Uuid>,
    pub key: String,
    pub label: String,
    /// `native` | `internal` | `plugin`.
    pub kind: String,
    pub description: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AiGrantDto {
    pub id: Uuid,
    pub surface_id: Uuid,
    pub surface_key: String,
    /// `role` | `user`.
    pub grantee_kind: String,
    #[schema(value_type = String, nullable = true)]
    pub role_id: Option<Uuid>,
    #[schema(value_type = String, nullable = true)]
    pub user_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AiCatalogDto {
    pub id: Uuid,
    pub provider: String,
    pub model: String,
    pub label: String,
    pub capabilities: Value,
    pub cost_weight: i32,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AiCapDto {
    pub id: Uuid,
    #[schema(value_type = String, nullable = true)]
    pub user_id: Option<Uuid>,
    pub period: String,
    #[schema(value_type = i64, nullable = true)]
    pub soft_token_limit: Option<i64>,
    #[schema(value_type = i64, nullable = true)]
    pub hard_token_limit: Option<i64>,
}

/// Presence-only credential descriptor — never secret material.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AiCredentialDto {
    pub id: Uuid,
    #[schema(value_type = String, nullable = true)]
    pub owner_user_id: Option<Uuid>,
    pub provider: String,
    /// `api_key` | `oauth_token` | `subscription_account`.
    pub kind: String,
    pub label: String,
    pub credential_meta: Value,
    /// `active` | `revoked`.
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AiMatrixResponse {
    pub surfaces: Vec<AiSurfaceDto>,
    pub grants: Vec<AiGrantDto>,
    pub catalog: Vec<AiCatalogDto>,
    pub caps: Vec<AiCapDto>,
    pub credentials: Vec<AiCredentialDto>,
    /// The `ai.personal_credentials_enabled` admin policy flag state.
    pub personal_credentials_enabled: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PutAiCatalogRequest {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default = "default_cost_weight")]
    pub cost_weight: i32,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(flatten, default)]
    pub instant: MutationParams,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AiCatalogMutationResponse {
    pub status: AiCatalogDto,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PutAiGrantRequest {
    pub surface_key: String,
    /// `role` | `user`.
    pub grantee_kind: String,
    #[serde(default)]
    pub role_id: Option<Uuid>,
    #[serde(default)]
    pub user_id: Option<Uuid>,
    #[serde(flatten, default)]
    pub instant: MutationParams,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AiGrantMutationResponse {
    pub status: AiGrantDto,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PutAiCapRequest {
    /// `org` | `user`.
    pub scope: String,
    #[serde(default)]
    pub user_id: Option<Uuid>,
    #[serde(default = "default_period")]
    pub period: String,
    #[serde(default)]
    pub soft_token_limit: Option<i64>,
    #[serde(default)]
    pub hard_token_limit: Option<i64>,
    #[serde(flatten, default)]
    pub instant: MutationParams,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AiCapMutationResponse {
    pub status: AiCapDto,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PutAiCredentialRequest {
    pub provider: String,
    /// `api_key` | `oauth_token` | `subscription_account`.
    pub kind: String,
    /// Plaintext secret — sealed at rest, never surfaced back.
    pub secret: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub credential_meta: Value,
    #[serde(flatten, default)]
    pub instant: MutationParams,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AiCredentialMutationResponse {
    pub status: AiCredentialDto,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PutAiPickRequest {
    pub provider: String,
    pub model: String,
    #[serde(flatten, default)]
    pub instant: MutationParams,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AiPickMutationResponse {
    pub surface_key: String,
    /// `user` | `org`.
    pub scope: String,
    pub provider: String,
    pub model: String,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AiSurfaceAccessDto {
    pub surface_key: String,
    pub label: String,
    pub kind: String,
    pub granted: bool,
    pub models_available: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AiMySurfacesResponse {
    pub surfaces: Vec<AiSurfaceAccessDto>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AiAuthorizeRequest {
    pub surface_key: String,
    /// Optional explicit (provider, model) ask — must sit inside the
    /// admin allowlist (an out-of-list ask is REFUSED, never redirected).
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AiAuthorizedCredential {
    pub id: Uuid,
    pub kind: String,
    /// true when a personal (per-user) credential was selected.
    pub personal: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AiCapState {
    pub used_tokens: i64,
    #[schema(value_type = i64, nullable = true)]
    pub soft_token_limit: Option<i64>,
    #[schema(value_type = i64, nullable = true)]
    pub hard_token_limit: Option<i64>,
    #[schema(value_type = f64, nullable = true)]
    pub pct_hard: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AiAuthorizeResponse {
    pub surface_key: String,
    pub provider: String,
    pub model: String,
    /// `requested` | `user_pick` | `org_pick` | `legacy_org_default` |
    /// `catalog_first`.
    pub resolution: String,
    pub credential: AiAuthorizedCredential,
    pub caps: AiCapState,
    /// Non-empty when a SOFT cap was crossed: the call proceeds; the
    /// warn also rides an `ai_cap_warn` notification.
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AiMyUsageResponse {
    pub period: String,
    pub used_tokens: i64,
    #[schema(value_type = i64, nullable = true)]
    pub soft_token_limit: Option<i64>,
    #[schema(value_type = i64, nullable = true)]
    pub hard_token_limit: Option<i64>,
    #[schema(value_type = f64, nullable = true)]
    pub pct_hard: Option<f64>,
    #[schema(value_type = f64, nullable = true)]
    pub pct_soft: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AiOrgUsageResponse {
    pub period: String,
    pub used_tokens: i64,
    pub active_users: i64,
    #[schema(value_type = i64, nullable = true)]
    pub soft_token_limit: Option<i64>,
    #[schema(value_type = i64, nullable = true)]
    pub hard_token_limit: Option<i64>,
    #[schema(value_type = f64, nullable = true)]
    pub pct_hard: Option<f64>,
    /// surface → token totals for the period.
    pub tokens_by_surface: Value,
}

fn default_true() -> bool {
    true
}
fn default_cost_weight() -> i32 {
    1
}
fn default_period() -> String {
    PERIOD.to_string()
}

fn surface_dto(s: core_db::ai_policy::AiSurface) -> AiSurfaceDto {
    AiSurfaceDto {
        id: s.id,
        org_id: s.org_id,
        key: s.key,
        label: s.label,
        kind: s.kind,
        description: s.description,
        enabled: s.enabled,
    }
}

fn grant_dto(g: core_db::ai_policy::AiSurfaceGrant) -> AiGrantDto {
    AiGrantDto {
        id: g.id,
        surface_id: g.surface_id,
        surface_key: g.surface_key,
        grantee_kind: g.grantee_kind,
        role_id: g.role_id,
        user_id: g.user_id,
    }
}

fn catalog_dto(e: core_db::ai_policy::AiModelCatalogEntry) -> AiCatalogDto {
    AiCatalogDto {
        id: e.id,
        provider: e.provider,
        model: e.model,
        label: e.label,
        capabilities: e.capabilities,
        cost_weight: e.cost_weight,
        enabled: e.enabled,
    }
}

fn cap_dto(c: core_db::ai_policy::AiCap) -> AiCapDto {
    AiCapDto {
        id: c.id,
        user_id: c.user_id,
        period: c.period,
        soft_token_limit: c.soft_token_limit,
        hard_token_limit: c.hard_token_limit,
    }
}

fn credential_dto(c: core_db::ai_policy::AiModelCredentialSummary) -> AiCredentialDto {
    AiCredentialDto {
        id: c.id,
        owner_user_id: c.owner_user_id,
        provider: c.provider,
        kind: c.kind,
        label: c.label,
        credential_meta: c.credential_meta,
        status: c.status,
    }
}

fn grant_event_payload(
    surface_key: &str,
    grantee_kind: &str,
    role_id: Option<Uuid>,
    user_id: Option<Uuid>,
) -> Value {
    serde_json::json!({
        "surface_key": surface_key,
        "grantee_kind": grantee_kind,
        "role_id": uid_opt_json(role_id),
        "user_id": uid_opt_json(user_id),
    })
}

fn cap_event_payload(cap: &core_db::ai_policy::AiCap, present: bool) -> Value {
    serde_json::json!({
        "scope": if cap.user_id.is_some() { "user" } else { "org" },
        "user_present": cap.user_id.is_some(),
        "period": cap.period,
        "has_soft_limit": present && cap.soft_token_limit.is_some(),
        "has_hard_limit": present && cap.hard_token_limit.is_some(),
    })
}

async fn personal_credentials_enabled<'e, E>(executor: E, org_id: Uuid) -> ApiResult<bool>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let overrides = core_db::settings::list_org_overrides(executor, org_id)
        .await
        .map_err(ApiError::from)?;
    Ok(overrides
        .iter()
        .find(|o| o.key == core_domain::settings::AI_PERSONAL_CREDENTIALS_ENABLED)
        .and_then(|o| o.value.as_bool())
        .unwrap_or(false))
}

/// The legacy flat (provider, model) pair — readable, deprecated,
/// customization-aware. `none` on either side means unset.
async fn legacy_pair<'e, E>(executor: E, org_id: Uuid) -> ApiResult<Option<(String, String)>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let overrides = core_db::settings::list_org_overrides(executor, org_id)
        .await
        .map_err(ApiError::from)?;
    let setting = |key: &str| -> String {
        overrides
            .iter()
            .find(|o| o.key == key)
            .and_then(|o| o.value.as_str())
            .unwrap_or("none")
            .to_string()
    };
    let provider = setting(core_domain::settings::AI_COMPLETION_PROVIDER);
    let model = setting(core_domain::settings::AI_COMPLETION_MODEL);
    if provider == "none" || model == "none" {
        return Ok(None);
    }
    Ok(Some((provider, model)))
}

// --- matrix ------------------------------------------------------------------------

/// GET /settings/ai/matrix — the full admin view.
#[utoipa::path(
    operation_id = "settings_ai_matrix_get",
    get,
    path = "/api/settings/ai/matrix",
    tag = "settings",
    security(("bearer" = [])),
    responses((status = 200, body = AiMatrixResponse), (status = 403))
)]
pub async fn get_matrix(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<AiMatrixResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_ai_admin(&authed.principal, Action::View)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let mut tx = org_bound_tx(&state, org_id, user_id).await?;
    let surfaces = core_db::ai_policy::list_surfaces(&mut *tx, org_id)
        .await
        .map_err(ApiError::from)?;
    let grants = core_db::ai_policy::list_grants(&mut *tx, org_id)
        .await
        .map_err(ApiError::from)?;
    let catalog = core_db::ai_policy::list_catalog(&mut *tx, org_id)
        .await
        .map_err(ApiError::from)?;
    let caps = core_db::ai_policy::list_caps(&mut *tx, org_id)
        .await
        .map_err(ApiError::from)?;
    let credentials = core_db::ai_policy::list_credential_summaries(
        &mut *tx,
        org_id,
        CredentialOwnerFilter::AllOwners,
    )
    .await
    .map_err(ApiError::from)?;
    let personal_credentials_enabled = personal_credentials_enabled(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;

    Ok(Json(AiMatrixResponse {
        surfaces: surfaces.into_iter().map(surface_dto).collect(),
        grants: grants.into_iter().map(grant_dto).collect(),
        catalog: catalog.into_iter().map(catalog_dto).collect(),
        caps: caps.into_iter().map(cap_dto).collect(),
        credentials: credentials.into_iter().map(credential_dto).collect(),
        personal_credentials_enabled,
    }))
}

// --- catalog -----------------------------------------------------------------------

/// PUT /settings/ai/catalog — upsert one curated (provider, model) pair.
#[utoipa::path(
    operation_id = "settings_ai_catalog_put",
    put,
    path = "/api/settings/ai/catalog",
    tag = "settings",
    security(("bearer" = [])),
    request_body = PutAiCatalogRequest,
    responses(
        (status = 200, body = AiCatalogMutationResponse),
        (status = 403),
        (status = 409, description = "duplicate (provider, model) pair")
    )
)]
pub async fn put_catalog(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<PutAiCatalogRequest>,
) -> ApiResult<Json<AiCatalogMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_ai_admin(&authed.principal, Action::Edit)?;

    let provider = req.provider.trim().to_ascii_lowercase();
    let model = req.model.trim().to_string();
    if provider.is_empty() || model.is_empty() {
        return Err(ApiError::Validation(
            "provider and model are required".into(),
        ));
    }
    if !(1..=5).contains(&req.cost_weight) {
        return Err(ApiError::Validation(
            "cost_weight must be between 1 and 5".into(),
        ));
    }
    let capabilities =
        serde_json::to_value(&req.capabilities).unwrap_or_else(|_| serde_json::json!([]));

    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;
    let new_id = Uuid::now_v7();
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let idem = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: CATALOG_ENTITY,
                entity_id: None,
                key: idem.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }

        let (entry_id, inserted) = core_db::ai_policy::upsert_catalog_entry(
            &mut ctx.tx,
            new_id,
            ctx.org_id,
            &provider,
            &model,
            &req.label,
            &capabilities,
            req.cost_weight,
            req.enabled,
            Some(actor_id),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: CATALOG_ENTITY,
                entity_id: entry_id,
                version: req.instant.version,
            },
        )
        .await?;

        let updated_at = chrono::Utc::now();
        let status = AiCatalogDto {
            id: entry_id,
            provider: provider.clone(),
            model: model.clone(),
            label: req.label.clone(),
            capabilities: capabilities.clone(),
            cost_weight: req.cost_weight,
            enabled: req.enabled,
        };
        ctx.emit(
            types::AI_MODEL_CATALOG_UPDATED,
            CATALOG_ENTITY,
            Some(entry_id.to_string().as_str()),
            &serde_json::json!({
                "provider": provider, "model": model, "enabled": req.enabled,
            }),
        )
        .await?;
        audit_row(
            &mut ctx.tx,
            ctx.org_id,
            actor_id,
            "settings.ai_model_catalog_updated",
            CATALOG_ENTITY,
            entry_id,
            serde_json::json!({
                "provider": provider, "model": model, "enabled": req.enabled,
            }),
        )
        .await?;

        let snapshot = serde_json::json!({
            "provider": provider, "model": model, "enabled": req.enabled,
        });
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: CATALOG_ENTITY,
                entity_id: entry_id,
                op: if inserted { "create" } else { "update" },
                seq: write_seq,
                before: if inserted {
                    None
                } else {
                    Some(snapshot.clone())
                },
                after: snapshot,
                provenance: Default::default(),
                updated_at,
                key: idem.as_deref(),
                payload: &status,
            },
        )
        .await?;

        Ok((
            ctx,
            MutationOutcome::Applied(AiCatalogMutationResponse { status, mutation }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// DELETE /settings/ai/catalog/{entry_id}.
#[utoipa::path(
    operation_id = "settings_ai_catalog_delete",
    delete,
    path = "/api/settings/ai/catalog/{entry_id}",
    tag = "settings",
    security(("bearer" = [])),
    params(("entry_id" = Uuid, Path)),
    responses(
        (status = 200, body = AiCatalogMutationResponse),
        (status = 403),
        (status = 404, description = "rls_sparse")
    )
)]
pub async fn delete_catalog(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(entry_id): Path<Uuid>,
) -> ApiResult<Json<AiCatalogMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_ai_admin(&authed.principal, Action::Edit)?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        let entry = core_db::ai_policy::get_catalog_entry_by_id(&mut *ctx.tx, ctx.org_id, entry_id)
            .await
            .map_err(ApiError::from)?
            .ok_or(ApiError::RlsSparse)?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: CATALOG_ENTITY,
                entity_id: entry_id,
                version: None,
            },
        )
        .await?;
        core_db::ai_policy::delete_catalog_entry(&mut ctx.tx, ctx.org_id, entry_id)
            .await
            .map_err(ApiError::from)?;

        ctx.emit(
            types::AI_MODEL_CATALOG_UPDATED,
            CATALOG_ENTITY,
            Some(entry_id.to_string().as_str()),
            &serde_json::json!({
                "provider": entry.provider.clone(), "model": entry.model.clone(), "enabled": false,
            }),
        )
        .await?;
        audit_row(
            &mut ctx.tx,
            ctx.org_id,
            actor_id,
            "settings.ai_model_catalog_deleted",
            CATALOG_ENTITY,
            entry_id,
            serde_json::json!({
                "provider": entry.provider.clone(), "model": entry.model.clone(),
            }),
        )
        .await?;

        let status = AiCatalogDto {
            id: entry_id,
            provider: entry.provider.clone(),
            model: entry.model.clone(),
            label: entry.label.clone(),
            capabilities: entry.capabilities.clone(),
            cost_weight: entry.cost_weight,
            enabled: false,
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: CATALOG_ENTITY,
                entity_id: entry_id,
                op: "delete",
                seq: write_seq,
                before: Some(serde_json::json!({
                    "provider": entry.provider.clone(), "model": entry.model.clone(), "enabled": true,
                })),
                after: Value::Null,
                provenance: Default::default(),
                updated_at: chrono::Utc::now(),
                key: None,
                payload: &status,
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(AiCatalogMutationResponse { status, mutation }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

// --- grants ------------------------------------------------------------------------

/// PUT /settings/ai/grants — upsert one per-surface grant (audited).
#[utoipa::path(
    operation_id = "settings_ai_grant_put",
    put,
    path = "/api/settings/ai/grants",
    tag = "settings",
    security(("bearer" = [])),
    request_body = PutAiGrantRequest,
    responses(
        (status = 200, body = AiGrantMutationResponse),
        (status = 403),
        (status = 422, description = "unknown surface or grantee validation")
    )
)]
pub async fn put_grant(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<PutAiGrantRequest>,
) -> ApiResult<Json<AiGrantMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_ai_admin(&authed.principal, Action::Edit)?;
    let (grantee_kind, role_id, user_id) =
        validate_grantee_kind(&req.grantee_kind, req.role_id, req.user_id)?;

    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;
    let new_id = Uuid::now_v7();
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let idem = req.instant.idempotency_key.clone();
    let surface_key = req.surface_key.trim().to_string();
    if surface_key.is_empty() {
        return Err(ApiError::Validation("surface_key is required".into()));
    }

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: GRANT_ENTITY,
                entity_id: None,
                key: idem.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }

        let surface =
            core_db::ai_policy::get_surface_by_key(&mut *ctx.tx, ctx.org_id, &surface_key)
                .await
                .map_err(ApiError::from)?
                .ok_or_else(|| ApiError::Validation(format!("unknown surface '{surface_key}'")))?;
        if surface.kind == "internal" {
            return Err(ApiError::AiCallRefused {
                reason: "ai_surface_internal",
                message: "internal surfaces are never grant targets".into(),
                surface: surface.key.clone(),
            });
        }

        let (grant_id, inserted) = core_db::ai_policy::upsert_grant(
            &mut ctx.tx,
            new_id,
            ctx.org_id,
            surface.id,
            &grantee_kind,
            role_id,
            user_id,
            actor_id,
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: GRANT_ENTITY,
                entity_id: grant_id,
                version: req.instant.version,
            },
        )
        .await?;

        let updated_at = chrono::Utc::now();
        let status = AiGrantDto {
            id: grant_id,
            surface_id: surface.id,
            surface_key: surface.key.clone(),
            grantee_kind: grantee_kind.clone(),
            role_id,
            user_id,
        };
        ctx.emit(
            types::AI_SURFACE_GRANT_UPDATED,
            GRANT_ENTITY,
            Some(grant_id.to_string().as_str()),
            &grant_event_payload(&surface.key, &grantee_kind, role_id, user_id),
        )
        .await?;
        audit_row(
            &mut ctx.tx,
            ctx.org_id,
            actor_id,
            "settings.ai_surface_grant_updated",
            GRANT_ENTITY,
            grant_id,
            grant_event_payload(&surface.key, &grantee_kind, role_id, user_id),
        )
        .await?;

        let snapshot = grant_event_payload(&surface.key, &grantee_kind, role_id, user_id);
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: GRANT_ENTITY,
                entity_id: grant_id,
                op: if inserted { "create" } else { "update" },
                seq: write_seq,
                before: if inserted {
                    None
                } else {
                    Some(snapshot.clone())
                },
                after: snapshot,
                provenance: Default::default(),
                updated_at,
                key: idem.as_deref(),
                payload: &status,
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(AiGrantMutationResponse { status, mutation }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

fn validate_grantee_kind(
    grantee_kind: &str,
    role_id: Option<Uuid>,
    user_id: Option<Uuid>,
) -> ApiResult<(String, Option<Uuid>, Option<Uuid>)> {
    match grantee_kind {
        "role" => {
            if role_id.is_none() || user_id.is_some() {
                return Err(ApiError::Validation(
                    "role grants need role_id and no user_id".into(),
                ));
            }
            Ok((grantee_kind.to_string(), role_id, user_id))
        }
        "user" => {
            if user_id.is_none() || role_id.is_some() {
                return Err(ApiError::Validation(
                    "user grants need user_id and no role_id".into(),
                ));
            }
            Ok((grantee_kind.to_string(), role_id, user_id))
        }
        other => Err(ApiError::Validation(format!(
            "grantee_kind must be 'role' or 'user', got '{other}'"
        ))),
    }
}

/// DELETE /settings/ai/grants/{grant_id} — revoke one grant (audited).
#[utoipa::path(
    operation_id = "settings_ai_grant_delete",
    delete,
    path = "/api/settings/ai/grants/{grant_id}",
    tag = "settings",
    security(("bearer" = [])),
    params(("grant_id" = Uuid, Path)),
    responses(
        (status = 200, body = AiGrantMutationResponse),
        (status = 403),
        (status = 404, description = "rls_sparse")
    )
)]
pub async fn delete_grant(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(grant_id): Path<Uuid>,
) -> ApiResult<Json<AiGrantMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_ai_admin(&authed.principal, Action::Edit)?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        let grants = core_db::ai_policy::list_grants(&mut *ctx.tx, ctx.org_id)
            .await
            .map_err(ApiError::from)?;
        let found = grants
            .into_iter()
            .find(|g| g.id == grant_id)
            .ok_or(ApiError::RlsSparse)?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: GRANT_ENTITY,
                entity_id: grant_id,
                version: None,
            },
        )
        .await?;
        core_db::ai_policy::revoke_grant(&mut ctx.tx, ctx.org_id, grant_id)
            .await
            .map_err(ApiError::from)?;

        ctx.emit(
            types::AI_SURFACE_GRANT_REVOKED,
            GRANT_ENTITY,
            Some(grant_id.to_string().as_str()),
            &serde_json::json!({
                "grant_id": grant_id.to_string(),
                "surface_key": found.surface_key,
            }),
        )
        .await?;
        audit_row(
            &mut ctx.tx,
            ctx.org_id,
            actor_id,
            "settings.ai_surface_grant_revoked",
            GRANT_ENTITY,
            grant_id,
            serde_json::json!({
                "grant_id": grant_id.to_string(),
                "surface_key": found.surface_key,
            }),
        )
        .await?;

        let status = AiGrantDto {
            id: grant_id,
            surface_id: found.surface_id,
            surface_key: found.surface_key.clone(),
            grantee_kind: found.grantee_kind.clone(),
            role_id: found.role_id,
            user_id: found.user_id,
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: GRANT_ENTITY,
                entity_id: grant_id,
                op: "delete",
                seq: write_seq,
                before: Some(grant_event_payload(
                    &found.surface_key,
                    &found.grantee_kind,
                    found.role_id,
                    found.user_id,
                )),
                after: Value::Null,
                provenance: Default::default(),
                updated_at: found.updated_at,
                key: None,
                payload: &status,
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(AiGrantMutationResponse { status, mutation }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

// --- caps ---------------------------------------------------------------------------

/// PUT /settings/ai/caps — upsert org-wide or per-user soft/hard caps (audited).
#[utoipa::path(
    operation_id = "settings_ai_cap_put",
    put,
    path = "/api/settings/ai/caps",
    tag = "settings",
    security(("bearer" = [])),
    request_body = PutAiCapRequest,
    responses(
        (status = 200, body = AiCapMutationResponse),
        (status = 403),
        (status = 422, description = "cap validation")
    )
)]
pub async fn put_cap(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<PutAiCapRequest>,
) -> ApiResult<Json<AiCapMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_ai_admin(&authed.principal, Action::Edit)?;

    let (user_id, user_present) = match req.scope.as_str() {
        "org" => {
            if req.user_id.is_some() {
                return Err(ApiError::Validation(
                    "org-scope caps must not carry user_id".into(),
                ));
            }
            (None, false)
        }
        "user" => {
            let Some(user) = req.user_id else {
                return Err(ApiError::Validation("user caps need user_id".into()));
            };
            (Some(user), true)
        }
        other => {
            return Err(ApiError::Validation(format!(
                "cap scope must be 'org' or 'user', got '{other}'"
            )))
        }
    };
    if req.soft_token_limit.is_none() && req.hard_token_limit.is_none() {
        return Err(ApiError::Validation(
            "at least one of soft_token_limit / hard_token_limit is required".into(),
        ));
    }
    if req.soft_token_limit.unwrap_or(1) <= 0 || req.hard_token_limit.unwrap_or(1) <= 0 {
        return Err(ApiError::Validation("cap limits must be positive".into()));
    }

    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;
    let new_id = Uuid::now_v7();
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let idem = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: CAP_ENTITY,
                entity_id: None,
                key: idem.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }

        let (cap_id, inserted) = core_db::ai_policy::upsert_cap(
            &mut ctx.tx,
            new_id,
            ctx.org_id,
            user_id,
            &req.period,
            req.soft_token_limit,
            req.hard_token_limit,
            actor_id,
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: CAP_ENTITY,
                entity_id: cap_id,
                version: req.instant.version,
            },
        )
        .await?;

        let updated_at = chrono::Utc::now();
        let status = AiCapDto {
            id: cap_id,
            user_id,
            period: req.period.clone(),
            soft_token_limit: req.soft_token_limit,
            hard_token_limit: req.hard_token_limit,
        };
        ctx.emit(
            types::AI_CAP_UPDATED,
            CAP_ENTITY,
            Some(cap_id.to_string().as_str()),
            &cap_event_row(
                &req.period,
                user_id,
                req.soft_token_limit,
                req.hard_token_limit,
            ),
        )
        .await?;
        audit_row(
            &mut ctx.tx,
            ctx.org_id,
            actor_id,
            if user_present {
                "settings.ai_user_cap_updated"
            } else {
                "settings.ai_org_cap_updated"
            },
            CAP_ENTITY,
            cap_id,
            cap_event_row(
                &req.period,
                user_id,
                req.soft_token_limit,
                req.hard_token_limit,
            ),
        )
        .await?;

        let snapshot = cap_event_row(
            &req.period,
            user_id,
            req.soft_token_limit,
            req.hard_token_limit,
        );
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: CAP_ENTITY,
                entity_id: cap_id,
                op: if inserted { "create" } else { "update" },
                seq: write_seq,
                before: if inserted {
                    None
                } else {
                    Some(snapshot.clone())
                },
                after: snapshot,
                provenance: Default::default(),
                updated_at,
                key: idem.as_deref(),
                payload: &status,
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(AiCapMutationResponse { status, mutation }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

fn cap_event_row(
    period: &str,
    user_id: Option<Uuid>,
    soft: Option<i64>,
    hard: Option<i64>,
) -> Value {
    serde_json::json!({
        "period": period,
        "user_id": uid_opt_json(user_id),
        "soft_token_limit": soft,
        "hard_token_limit": hard,
    })
}

/// DELETE /settings/ai/caps/{cap_id}.
#[utoipa::path(
    operation_id = "settings_ai_cap_delete",
    delete,
    path = "/api/settings/ai/caps/{cap_id}",
    tag = "settings",
    security(("bearer" = [])),
    params(("cap_id" = Uuid, Path)),
    responses(
        (status = 200, body = AiCapMutationResponse),
        (status = 403),
        (status = 404, description = "rls_sparse")
    )
)]
pub async fn delete_cap(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(cap_id): Path<Uuid>,
) -> ApiResult<Json<AiCapMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_ai_admin(&authed.principal, Action::Edit)?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        let caps = core_db::ai_policy::list_caps(&mut *ctx.tx, ctx.org_id)
            .await
            .map_err(ApiError::from)?;
        let found = caps
            .into_iter()
            .find(|c| c.id == cap_id)
            .ok_or(ApiError::RlsSparse)?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: CAP_ENTITY,
                entity_id: cap_id,
                version: None,
            },
        )
        .await?;
        core_db::ai_policy::delete_cap(&mut ctx.tx, ctx.org_id, cap_id)
            .await
            .map_err(ApiError::from)?;

        ctx.emit(
            types::AI_CAP_UPDATED,
            CAP_ENTITY,
            Some(cap_id.to_string().as_str()),
            &cap_event_payload(&found, false),
        )
        .await?;
        audit_row(
            &mut ctx.tx,
            ctx.org_id,
            actor_id,
            "settings.ai_cap_deleted",
            CAP_ENTITY,
            cap_id,
            cap_event_row(
                &found.period,
                found.user_id,
                found.soft_token_limit,
                found.hard_token_limit,
            ),
        )
        .await?;

        let status = AiCapDto {
            id: cap_id,
            user_id: found.user_id,
            period: found.period.clone(),
            soft_token_limit: found.soft_token_limit,
            hard_token_limit: found.hard_token_limit,
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: CAP_ENTITY,
                entity_id: cap_id,
                op: "delete",
                seq: write_seq,
                before: Some(cap_event_row(
                    &found.period,
                    found.user_id,
                    found.soft_token_limit,
                    found.hard_token_limit,
                )),
                after: Value::Null,
                provenance: Default::default(),
                updated_at: found.updated_at,
                key: None,
                payload: &status,
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(AiCapMutationResponse { status, mutation }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

// --- credentials ----------------------------------------------------------------------

/// Shared seal path for ORG (admin) and PERSONAL (flag-gated user)
/// credentials: seed/format validation, AES-256-GCM sealing, instant-first
/// write path, contract event + audit row. Secret bytes never re-render.
pub async fn seal_credential(
    state: &AppState,
    extensions: &Extensions,
    req: PutAiCredentialRequest,
    owner: Option<Uuid>,
) -> ApiResult<(AiCredentialDto, instant::MutationMeta)> {
    let authed = authed::<crate::auth::AuthedUser>(extensions)?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

    match owner {
        // Org credentials: owner/admin standing on top of the grant.
        None => require_ai_admin(&authed.principal, Action::Edit)?,
        // Personal credentials: any authed user, but ONLY while the org
        // policy flag is on.
        Some(_) => {
            let mut tx = org_bound_tx(state, org_id, actor_id).await?;
            let allowed = personal_credentials_enabled(&mut *tx, org_id).await?;
            tx.commit().await.map_err(ApiError::from)?;
            if !allowed {
                return Err(ApiError::AiCallRefused {
                    reason: "ai_personal_credentials_disabled",
                    message: "personal AI credentials are disabled by org policy; ask your admin"
                        .into(),
                    surface: "credentials".into(),
                });
            }
        }
    }

    let provider = req.provider.trim().to_ascii_lowercase();
    let kind = req.kind.trim().to_string();
    if provider.is_empty() {
        return Err(ApiError::Validation("provider is required".into()));
    }
    let known = core_domain::settings::byok_provider_options().any(|p| p == provider);
    if !known {
        return Err(ApiError::Validation(format!(
            "unsupported provider: '{provider}'. Supported: {}",
            core_domain::settings::byok_provider_options()
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    if !matches!(
        kind.as_str(),
        "api_key" | "oauth_token" | "subscription_account"
    ) {
        return Err(ApiError::Validation(format!(
            "unsupported credential kind: '{kind}'. Supported: api_key, oauth_token, \
             subscription_account"
        )));
    }
    if req.secret.trim().is_empty() {
        return Err(ApiError::Validation("secret cannot be empty".into()));
    }

    let sealed = crate::crypto::seal(&state.config.secret_key, req.secret.as_bytes())
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("failed to seal credential: {e}")))?;

    let new_id = Uuid::now_v7();
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let idem = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(state, org_id, actor_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: CREDENTIAL_ENTITY,
                entity_id: None,
                key: idem.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }

        let (credential_id, inserted) = core_db::ai_policy::upsert_credential(
            &mut ctx.tx,
            new_id,
            ctx.org_id,
            owner,
            &provider,
            &kind,
            &req.label,
            &sealed,
            &req.credential_meta,
            actor_id,
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: CREDENTIAL_ENTITY,
                entity_id: credential_id,
                version: req.instant.version,
            },
        )
        .await?;

        let updated_at = chrono::Utc::now();
        let status = AiCredentialDto {
            id: credential_id,
            owner_user_id: owner,
            provider: provider.clone(),
            kind: kind.clone(),
            label: req.label.clone(),
            credential_meta: req.credential_meta.clone(),
            status: "active".to_string(),
        };
        ctx.emit(
            types::AI_MODEL_CREDENTIAL_UPDATED,
            CREDENTIAL_ENTITY,
            Some(credential_id.to_string().as_str()),
            &credential_event_payload(credential_id, &provider, &kind, owner, true),
        )
        .await?;
        audit_row(
            &mut ctx.tx,
            ctx.org_id,
            actor_id,
            "settings.ai_model_credential_updated",
            CREDENTIAL_ENTITY,
            credential_id,
            credential_event_payload(credential_id, &provider, &kind, owner, true),
        )
        .await?;

        let snapshot = credential_event_payload(credential_id, &provider, &kind, owner, true);
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: CREDENTIAL_ENTITY,
                entity_id: credential_id,
                op: if inserted { "create" } else { "update" },
                seq: write_seq,
                before: if inserted {
                    None
                } else {
                    Some(snapshot.clone())
                },
                after: snapshot,
                // Presence-only trail: the sealed bytes themselves never
                // enter the trail (same posture as byok).
                provenance: Default::default(),
                updated_at,
                key: idem.as_deref(),
                payload: &status,
            },
        )
        .await?;
        Ok((ctx, MutationOutcome::Applied((status, mutation))))
    })
    .await?;
    let (status, mutation) = outcome.into_response()?;
    Ok((status, mutation))
}

pub async fn revoke_credential(
    state: &AppState,
    extensions: &Extensions,
    credential_id: Uuid,
    owner_lock: Option<Uuid>,
) -> ApiResult<(AiCredentialDto, instant::MutationMeta)> {
    let authed = authed::<crate::auth::AuthedUser>(extensions)?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

    if owner_lock.is_none() {
        // Org credentials: owner/admin standing.
        require_ai_admin(&authed.principal, Action::Edit)?;
        // Personal: any authed user may revoke their OWN credential —
        // the owner_lock at the DB layer enforces ownership.
    }

    let outcome = org_tx::with_org_tx(state, org_id, actor_id, |mut ctx| async move {
        let meta = core_db::ai_policy::get_credential_meta(&mut *ctx.tx, ctx.org_id, credential_id)
            .await
            .map_err(ApiError::from)?
            .ok_or(ApiError::RlsSparse)?;
        // Owner-locked revoke: personal rows only leave through this
        // handler when the caller's id matches the row's owner.
        let affected = core_db::ai_policy::revoke_credential(
            &mut ctx.tx,
            ctx.org_id,
            credential_id,
            owner_lock,
        )
        .await
        .map_err(ApiError::from)?;
        if !affected {
            return Err(ApiError::RlsSparse);
        }

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: CREDENTIAL_ENTITY,
                entity_id: credential_id,
                version: None,
            },
        )
        .await?;

        ctx.emit(
            types::AI_MODEL_CREDENTIAL_REVOKED,
            CREDENTIAL_ENTITY,
            Some(credential_id.to_string().as_str()),
            &credential_event_payload(
                credential_id,
                &meta.provider,
                &meta.kind,
                meta.owner_user_id,
                false,
            ),
        )
        .await?;
        audit_row(
            &mut ctx.tx,
            ctx.org_id,
            actor_id,
            "settings.ai_model_credential_revoked",
            CREDENTIAL_ENTITY,
            credential_id,
            credential_event_payload(
                credential_id,
                &meta.provider,
                &meta.kind,
                meta.owner_user_id,
                false,
            ),
        )
        .await?;

        let status = AiCredentialDto {
            id: credential_id,
            owner_user_id: meta.owner_user_id,
            provider: meta.provider.clone(),
            kind: meta.kind.clone(),
            label: String::new(),
            credential_meta: serde_json::json!({}),
            status: "revoked".to_string(),
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: CREDENTIAL_ENTITY,
                entity_id: credential_id,
                op: "delete",
                seq: write_seq,
                before: Some(credential_event_payload(
                    credential_id,
                    &meta.provider,
                    &meta.kind,
                    meta.owner_user_id,
                    true,
                )),
                after: Value::Null,
                provenance: Default::default(),
                updated_at: meta.updated_at,
                key: None,
                payload: &status,
            },
        )
        .await?;
        Ok((ctx, MutationOutcome::Applied((status, mutation))))
    })
    .await?;
    let (status, mutation) = outcome.into_response()?;
    Ok((status, mutation))
}

fn credential_event_payload(
    credential_id: Uuid,
    provider: &str,
    kind: &str,
    owner: Option<Uuid>,
    has_secret: bool,
) -> Value {
    serde_json::json!({
        "credential_id": credential_id.to_string(),
        "provider": provider,
        "kind": kind,
        "owner_present": owner.is_some(),
        "has_secret": has_secret,
    })
}

// --- surface access -----------------------------------------------------------------

/// GET /ai/me/surfaces — which surfaces the CALLER may use
/// (deny-by-default: only granted, enabled, non-internal rows surface).
#[utoipa::path(
    operation_id = "ai_my_surfaces_get",
    get,
    path = "/api/ai/me/surfaces",
    tag = "ai",
    security(("bearer" = [])),
    responses((status = 200, body = AiMySurfacesResponse))
)]
pub async fn get_my_surfaces(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<AiMySurfacesResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let mut tx = org_bound_tx(&state, org_id, user_id).await?;
    let surfaces = core_db::ai_policy::list_surfaces(&mut *tx, org_id)
        .await
        .map_err(ApiError::from)?;
    let catalog = core_db::ai_policy::list_catalog(&mut *tx, org_id)
        .await
        .map_err(ApiError::from)?;
    let mut out = Vec::new();
    for surface in surfaces {
        if surface.kind == "internal" {
            continue;
        }
        let granted =
            core_db::ai_policy::user_has_surface_grant(&mut *tx, org_id, user_id, surface.id)
                .await
                .map_err(ApiError::from)?;
        out.push(AiSurfaceAccessDto {
            surface_key: surface.key.clone(),
            label: surface.label.clone(),
            kind: surface.kind.clone(),
            granted,
            models_available: catalog.iter().any(|entry| entry.enabled),
        });
    }
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(AiMySurfacesResponse { surfaces: out }))
}

/// GET /ai/me/credentials — the caller's personal credential summaries.
#[utoipa::path(
    operation_id = "ai_my_credentials_get",
    get,
    path = "/api/ai/me/credentials",
    tag = "ai",
    security(("bearer" = [])),
    responses((status = 200, body = Vec<AiCredentialDto>))
)]
pub async fn get_my_credentials(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<AiCredentialDto>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let mut tx = org_bound_tx(&state, org_id, user_id).await?;
    let summaries = core_db::ai_policy::list_credential_summaries(
        &mut *tx,
        org_id,
        CredentialOwnerFilter::PersonalOf(user_id),
    )
    .await
    .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(summaries.into_iter().map(credential_dto).collect()))
}

/// GET /settings/ai/credentials — the org's credential summaries (admin).
#[utoipa::path(
    operation_id = "settings_ai_credentials_get",
    get,
    path = "/api/settings/ai/credentials",
    tag = "settings",
    security(("bearer" = [])),
    responses((status = 200, body = Vec<AiCredentialDto>), (status = 403))
)]
pub async fn get_org_credentials(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<AiCredentialDto>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_ai_admin(&authed.principal, Action::View)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let mut tx = org_bound_tx(&state, org_id, user_id).await?;
    let summaries = core_db::ai_policy::list_credential_summaries(
        &mut *tx,
        org_id,
        CredentialOwnerFilter::OrgOnly,
    )
    .await
    .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(summaries.into_iter().map(credential_dto).collect()))
}

/// PUT /settings/ai/credentials — seal an ORG credential (admin).
#[utoipa::path(
    operation_id = "settings_ai_credential_put",
    put,
    path = "/api/settings/ai/credentials",
    tag = "settings",
    security(("bearer" = [])),
    request_body = PutAiCredentialRequest,
    responses(
        (status = 200, body = AiCredentialMutationResponse),
        (status = 403),
        (status = 422)
    )
)]
pub async fn put_org_credential(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<PutAiCredentialRequest>,
) -> ApiResult<Json<AiCredentialMutationResponse>> {
    let (status, mutation) = seal_credential(&state, &extensions, req, None).await?;
    Ok(Json(AiCredentialMutationResponse { status, mutation }))
}

/// DELETE /settings/ai/credentials/{credential_id} — revoke an ORG credential.
#[utoipa::path(
    operation_id = "settings_ai_credential_delete",
    delete,
    path = "/api/settings/ai/credentials/{credential_id}",
    tag = "settings",
    security(("bearer" = [])),
    params(("credential_id" = Uuid, Path)),
    responses(
        (status = 200, body = AiCredentialMutationResponse),
        (status = 403),
        (status = 404, description = "rls_sparse")
    )
)]
pub async fn delete_org_credential(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(credential_id): Path<Uuid>,
) -> ApiResult<Json<AiCredentialMutationResponse>> {
    let (status, mutation) = revoke_credential(&state, &extensions, credential_id, None).await?;
    Ok(Json(AiCredentialMutationResponse { status, mutation }))
}

/// PUT /ai/me/credentials — seal a PERSONAL credential (flag-gated).
#[utoipa::path(
    operation_id = "ai_my_credential_put",
    put,
    path = "/api/ai/me/credentials",
    tag = "ai",
    security(("bearer" = [])),
    request_body = PutAiCredentialRequest,
    responses(
        (status = 200, body = AiCredentialMutationResponse),
        (status = 403, description = "personal credentials disabled by policy"),
        (status = 422)
    )
)]
pub async fn put_my_credential(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<PutAiCredentialRequest>,
) -> ApiResult<Json<AiCredentialMutationResponse>> {
    let owner = authed::<crate::auth::AuthedUser>(&extensions)?
        .principal
        .user
        .id;
    let (status, mutation) = seal_credential(&state, &extensions, req, Some(owner)).await?;
    Ok(Json(AiCredentialMutationResponse { status, mutation }))
}

/// DELETE /ai/me/credentials/{credential_id} — revoke a PERSONAL credential.
#[utoipa::path(
    operation_id = "ai_my_credential_delete",
    delete,
    path = "/api/ai/me/credentials/{credential_id}",
    tag = "ai",
    security(("bearer" = [])),
    params(("credential_id" = Uuid, Path)),
    responses(
        (status = 200, body = AiCredentialMutationResponse),
        (status = 403),
        (status = 404, description = "rls_sparse")
    )
)]
pub async fn delete_my_credential(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(credential_id): Path<Uuid>,
) -> ApiResult<Json<AiCredentialMutationResponse>> {
    let owner = authed::<crate::auth::AuthedUser>(&extensions)?
        .principal
        .user
        .id;
    let (status, mutation) =
        revoke_credential(&state, &extensions, credential_id, Some(owner)).await?;
    Ok(Json(AiCredentialMutationResponse { status, mutation }))
}

// --- picks ---------------------------------------------------------------------------

/// PUT /ai/me/model-picks/{surface_key} — a sticky pick; the pair must sit
/// inside the admin allowlist or the pick is typed-refused.
#[utoipa::path(
    operation_id = "ai_my_pick_put",
    put,
    path = "/api/ai/me/model-picks/{surface_key}",
    tag = "ai",
    security(("bearer" = [])),
    params(("surface_key" = String, Path)),
    request_body = PutAiPickRequest,
    responses(
        (status = 200, body = AiPickMutationResponse),
        (status = 403),
        (status = 404, description = "rls_sparse"),
        (status = 422, description = "model not in the admin allowlist")
    )
)]
pub async fn put_my_pick(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(surface_key): Path<String>,
    Json(req): Json<PutAiPickRequest>,
) -> ApiResult<Json<AiPickMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;
    let surface_key = surface_key.trim().to_string();
    if surface_key.is_empty() {
        return Err(ApiError::Validation("surface_key is required".into()));
    }

    let new_id = Uuid::now_v7();
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let idem = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: PICK_ENTITY,
                entity_id: None,
                key: idem.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }

        let surface =
            core_db::ai_policy::get_surface_by_key(&mut *ctx.tx, ctx.org_id, &surface_key)
                .await
                .map_err(ApiError::from)?
                .ok_or(ApiError::RlsSparse)?;
        let entry = core_db::ai_policy::get_catalog_entry(
            &mut *ctx.tx,
            ctx.org_id,
            &req.provider,
            &req.model,
        )
        .await
        .map_err(ApiError::from)?;
        let allowed = entry.as_ref().map(|e| e.enabled).unwrap_or(false);
        if !allowed {
            return Err(ApiError::AiCallRefused {
                reason: "ai_model_not_allowed",
                message: format!(
                    "the (provider, model) pair '{}/{}' is not in the admin allowlist",
                    req.provider, req.model
                ),
                surface: surface_key.clone(),
            });
        }

        let (pick_id, inserted) = core_db::ai_policy::upsert_pick(
            &mut ctx.tx,
            new_id,
            ctx.org_id,
            Some(actor_id),
            surface.id,
            &req.provider.trim().to_ascii_lowercase(),
            req.model.trim(),
            actor_id,
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: PICK_ENTITY,
                entity_id: pick_id,
                version: req.instant.version,
            },
        )
        .await?;

        let updated_at = chrono::Utc::now();
        let payload = serde_json::json!({
            "surface_key": surface_key,
            "scope": "user",
            "provider": req.provider,
            "model": req.model,
        });
        ctx.emit(
            types::AI_MODEL_PICK_UPDATED,
            PICK_ENTITY,
            Some(pick_id.to_string().as_str()),
            &payload,
        )
        .await?;
        audit_row(
            &mut ctx.tx,
            ctx.org_id,
            actor_id,
            "settings.ai_model_pick_updated",
            PICK_ENTITY,
            pick_id,
            payload.clone(),
        )
        .await?;

        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: PICK_ENTITY,
                entity_id: pick_id,
                op: if inserted { "create" } else { "update" },
                seq: write_seq,
                before: if inserted {
                    None
                } else {
                    Some(payload.clone())
                },
                after: payload,
                provenance: Default::default(),
                updated_at,
                key: idem.as_deref(),
                payload: &serde_json::json!({
                    "surface_key": surface_key, "scope": "user",
                    "provider": req.provider, "model": req.model,
                }),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(AiPickMutationResponse {
                surface_key,
                scope: "user".to_string(),
                provider: req.provider,
                model: req.model,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// PUT /settings/ai/model-picks/{surface_key} — the ORG default pick
/// (owner/admin standing; stored as user_id NULL so every member's
/// resolution falls back onto it when no personal pick holds).
#[utoipa::path(
    operation_id = "settings_ai_org_pick_put",
    put,
    path = "/api/settings/ai/model-picks/{surface_key}",
    tag = "settings",
    security(("bearer" = [])),
    params(("surface_key" = String, Path)),
    request_body = PutAiPickRequest,
    responses(
        (status = 200, body = AiPickMutationResponse),
        (status = 403),
        (status = 404, description = "rls_sparse"),
        (status = 422, description = "model not in the admin allowlist")
    )
)]
pub async fn put_org_pick(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(surface_key): Path<String>,
    Json(req): Json<PutAiPickRequest>,
) -> ApiResult<Json<AiPickMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_ai_admin(&authed.principal, Action::Edit)?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;
    let surface_key = surface_key.trim().to_string();
    if surface_key.is_empty() {
        return Err(ApiError::Validation("surface_key is required".into()));
    }

    let new_id = Uuid::now_v7();
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let idem = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: PICK_ENTITY,
                entity_id: None,
                key: idem.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }

        let surface =
            core_db::ai_policy::get_surface_by_key(&mut *ctx.tx, ctx.org_id, &surface_key)
                .await
                .map_err(ApiError::from)?
                .ok_or(ApiError::RlsSparse)?;
        let entry = core_db::ai_policy::get_catalog_entry(
            &mut *ctx.tx,
            ctx.org_id,
            &req.provider,
            &req.model,
        )
        .await
        .map_err(ApiError::from)?;
        let allowed = entry.as_ref().map(|e| e.enabled).unwrap_or(false);
        if !allowed {
            return Err(ApiError::AiCallRefused {
                reason: "ai_model_not_allowed",
                message: format!(
                    "the (provider, model) pair '{}/{}' is not in the admin allowlist",
                    req.provider, req.model
                ),
                surface: surface_key.clone(),
            });
        }

        let (pick_id, inserted) = core_db::ai_policy::upsert_pick(
            &mut ctx.tx,
            new_id,
            ctx.org_id,
            None, // NULL owner_user_id = the org default pick
            surface.id,
            &req.provider.trim().to_ascii_lowercase(),
            req.model.trim(),
            actor_id,
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: PICK_ENTITY,
                entity_id: pick_id,
                version: req.instant.version,
            },
        )
        .await?;

        let updated_at = chrono::Utc::now();
        let payload = serde_json::json!({
            "surface_key": surface_key,
            "scope": "org",
            "provider": req.provider,
            "model": req.model,
        });
        ctx.emit(
            types::AI_MODEL_PICK_UPDATED,
            PICK_ENTITY,
            Some(pick_id.to_string().as_str()),
            &payload,
        )
        .await?;
        audit_row(
            &mut ctx.tx,
            ctx.org_id,
            actor_id,
            "settings.ai_org_pick_updated",
            PICK_ENTITY,
            pick_id,
            payload.clone(),
        )
        .await?;

        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: PICK_ENTITY,
                entity_id: pick_id,
                op: if inserted { "create" } else { "update" },
                seq: write_seq,
                before: if inserted {
                    None
                } else {
                    Some(payload.clone())
                },
                after: payload,
                provenance: Default::default(),
                updated_at,
                key: idem.as_deref(),
                payload: &serde_json::json!({
                    "surface_key": surface_key, "scope": "org",
                    "provider": req.provider, "model": req.model,
                }),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(AiPickMutationResponse {
                surface_key,
                scope: "org".to_string(),
                provider: req.provider,
                model: req.model,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// DELETE /ai/me/model-picks/{surface_key} — clears the sticky pick.
#[utoipa::path(
    operation_id = "ai_my_pick_delete",
    delete,
    path = "/api/ai/me/model-picks/{surface_key}",
    tag = "ai",
    security(("bearer" = [])),
    params(("surface_key" = String, Path)),
    responses(
        (status = 200, body = AiPickMutationResponse),
        (status = 403),
        (status = 404, description = "rls_sparse")
    )
)]
pub async fn delete_my_pick(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(surface_key): Path<String>,
) -> ApiResult<Json<AiPickMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;
    let surface_key = surface_key.trim().to_string();

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        let surface =
            core_db::ai_policy::get_surface_by_key(&mut *ctx.tx, ctx.org_id, &surface_key)
                .await
                .map_err(ApiError::from)?
                .ok_or(ApiError::RlsSparse)?;
        let prior =
            core_db::ai_policy::get_user_pick(&mut *ctx.tx, ctx.org_id, actor_id, surface.id)
                .await
                .map_err(ApiError::from)?
                .ok_or(ApiError::RlsSparse)?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: PICK_ENTITY,
                entity_id: prior.id,
                version: None,
            },
        )
        .await?;
        core_db::ai_policy::delete_user_pick(&mut ctx.tx, ctx.org_id, actor_id, surface.id)
            .await
            .map_err(ApiError::from)?;

        ctx.emit(
            types::AI_MODEL_PICK_UPDATED,
            PICK_ENTITY,
            Some(prior.id.to_string().as_str()),
            &serde_json::json!({
                "surface_key": surface_key,
                "scope": "user",
                "provider": prior.provider,
                "model": prior.model,
                "removed": true,
            }),
        )
        .await?;
        audit_row(
            &mut ctx.tx,
            ctx.org_id,
            actor_id,
            "settings.ai_model_pick_cleared",
            PICK_ENTITY,
            prior.id,
            serde_json::json!({
                "surface_key": surface_key,
                "scope": "user",
                "provider": prior.provider,
                "model": prior.model,
                "removed": true,
            }),
        )
        .await?;

        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: PICK_ENTITY,
                entity_id: prior.id,
                op: "delete",
                seq: write_seq,
                before: Some(serde_json::json!({
                    "surface_key": surface_key,
                    "scope": "user",
                    "provider": prior.provider,
                    "model": prior.model,
                })),
                after: Value::Null,
                provenance: Default::default(),
                updated_at: chrono::Utc::now(),
                key: None,
                payload: &serde_json::json!({
                    "surface_key": surface_key,
                    "scope": "user",
                    "removed": true,
                }),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(AiPickMutationResponse {
                surface_key,
                scope: "user".to_string(),
                provider: String::new(),
                model: String::new(),
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

// --- usage ---------------------------------------------------------------------------

/// GET /ai/me/usage — personal progress against caps.
#[utoipa::path(
    operation_id = "ai_my_usage_get",
    get,
    path = "/api/ai/me/usage",
    tag = "ai",
    security(("bearer" = [])),
    responses((status = 200, body = AiMyUsageResponse))
)]
pub async fn get_my_usage(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<AiMyUsageResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let mut tx = org_bound_tx(&state, org_id, user_id).await?;
    let (soft_limit, hard_limit) =
        core_db::ai_policy::effective_cap(&mut tx, org_id, user_id, PERIOD)
            .await
            .map_err(ApiError::from)?
            .unwrap_or((None, None));
    let used = core_db::ai_policy::user_usage_since(
        &mut *tx,
        org_id,
        user_id,
        month_start(chrono::Utc::now()),
    )
    .await
    .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;

    Ok(Json(AiMyUsageResponse {
        period: PERIOD.to_string(),
        used_tokens: used,
        soft_token_limit: soft_limit,
        hard_token_limit: hard_limit,
        pct_hard: pct_of(used, hard_limit),
        pct_soft: pct_of(used, soft_limit),
    }))
}

/// GET /settings/ai/usage/org — org progress roll-up (admin).
#[utoipa::path(
    operation_id = "settings_ai_usage_org_get",
    get,
    path = "/api/settings/ai/usage/org",
    tag = "settings",
    security(("bearer" = [])),
    responses((status = 200, body = AiOrgUsageResponse), (status = 403))
)]
pub async fn get_org_usage(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<AiOrgUsageResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_ai_admin(&authed.principal, Action::View)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let mut tx = org_bound_tx(&state, org_id, user_id).await?;
    let caps = core_db::ai_policy::list_caps(&mut *tx, org_id)
        .await
        .map_err(ApiError::from)?;
    let org_wide = caps
        .iter()
        .find(|c| c.user_id.is_none() && c.period == PERIOD);
    let since = month_start(chrono::Utc::now());
    let used = core_db::ai_policy::org_usage_since(&mut *tx, org_id, since)
        .await
        .map_err(ApiError::from)?;
    let active_users: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT user_id) FROM ai_usage_ledger WHERE org_id = $1 AND created_at >= $2",
    )
    .bind(org_id)
    .bind(since)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::from)?;
    let by_surface: Vec<(String, i64)> = sqlx::query_as(
        r#"SELECT surface, SUM(prompt_tokens::bigint + completion_tokens::bigint)
           FROM ai_usage_ledger
           WHERE org_id = $1 AND created_at >= $2
           GROUP BY surface
           ORDER BY surface"#,
    )
    .bind(org_id)
    .bind(since)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;

    let hard_limit = org_wide.and_then(|c| c.hard_token_limit);
    let soft_limit = org_wide.and_then(|c| c.soft_token_limit);
    Ok(Json(AiOrgUsageResponse {
        period: PERIOD.to_string(),
        used_tokens: used,
        active_users,
        soft_token_limit: soft_limit,
        hard_token_limit: hard_limit,
        pct_hard: pct_of(used, hard_limit),
        tokens_by_surface: serde_json::to_value(
            by_surface
                .into_iter()
                .collect::<std::collections::BTreeMap<_, _>>(),
        )
        .unwrap_or_else(|_| serde_json::json!({})),
    }))
}

// --- completion edge -----------------------------------------------------------------

/// The shared authorization pipeline (ADR 0008 §1). Surface checks →
/// model resolution → caps (hard bound BEFORE dispatch) → sealed
/// credential slot (+ tamper check) → soft-cap warn. The HTTP handler and
/// the ChatRunner (Wave 18 lane 2) consume the SAME pipeline — one
/// deny-by-default edge, no shadow re-wiring.
///
/// Caller owns the tx lifecycle: the soft-cap warn path writes a
/// notification into `tx` — commit AFTER consuming the result.
pub(super) async fn authorize_surface_call(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    surface_key: &str,
    provider_ask: Option<&str>,
    model_ask: Option<&str>,
) -> ApiResult<AuthorizedAiCall> {
    let surface = core_db::ai_policy::get_surface_by_key(&mut *tx, org_id, surface_key)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::RlsSparse)?;
    if surface.kind == "internal" {
        return Err(ApiError::AiCallRefused {
            reason: "ai_surface_internal",
            message: format!(
                "internal surfaces such as '{surface_key}' are never directly callable"
            ),
            surface: surface_key.to_string(),
        });
    }
    if !surface.enabled {
        return Err(ApiError::AiCallRefused {
            reason: "ai_surface_disabled",
            message: format!("the '{surface_key}' surface is disabled for this org"),
            surface: surface_key.to_string(),
        });
    }
    if !core_db::ai_policy::user_has_surface_grant(&mut *tx, org_id, user_id, surface.id)
        .await
        .map_err(ApiError::from)?
    {
        return Err(ApiError::AiCallRefused {
            reason: "ai_surface_not_granted",
            message: format!(
                "no admin grant covers you on the '{surface_key}' surface (deny-by-default)"
            ),
            surface: surface_key.to_string(),
        });
    }

    // Allowlist = enabled catalog entries (global for the surface).
    let catalog = core_db::ai_policy::list_catalog(&mut *tx, org_id)
        .await
        .map_err(ApiError::from)?;
    let allowlist: Vec<(String, String)> = catalog
        .iter()
        .filter(|c| c.enabled)
        .map(|c| (c.provider.clone(), c.model.clone()))
        .collect();

    let (provider, model, resolution) = match (provider_ask, model_ask) {
        (Some(provider), Some(model))
            if !provider.trim().is_empty() && !model.trim().is_empty() =>
        {
            if !allowlist.contains(&(provider.trim().to_string(), model.trim().to_string())) {
                return Err(ApiError::AiCallRefused {
                    reason: "ai_model_not_allowed",
                    message: format!(
                        "the (provider, model) pair '{}/{}' is not in the admin allowlist",
                        provider.trim(),
                        model.trim()
                    ),
                    surface: surface_key.to_string(),
                });
            }
            (
                provider.trim().to_string(),
                model.trim().to_string(),
                "requested",
            )
        }
        _ => resolve_pick(tx, org_id, user_id, surface.id, surface_key, &allowlist).await?,
    };

    // Caps: per-user cap when present, else org-wide; hard bound BEFORE
    // dispatch (typed refusal; degrade-never-5xx).
    let (soft_limit, hard_limit) =
        core_db::ai_policy::effective_cap(&mut *tx, org_id, user_id, PERIOD)
            .await
            .map_err(ApiError::from)?
            .unwrap_or((None, None));
    let since = month_start(chrono::Utc::now());
    let used = core_db::ai_policy::user_usage_since(&mut *tx, org_id, user_id, since)
        .await
        .map_err(ApiError::from)?;
    if let Some(hard) = hard_limit {
        if used >= hard {
            return Err(ApiError::AiCallRefused {
                reason: "ai_hard_cap_reached",
                message: format!(
                    "your '{surface_key}' usage hit the hard cap for this period ({used}/{hard} tokens)"
                ),
                surface: surface_key.to_string(),
            });
        }
    }

    // Credential slot: personal first when the org policy flag allows,
    // else org. Missing sealed material is a typed refusal.
    let personal_allowed = personal_credentials_enabled(&mut *tx, org_id).await?;
    let credential = core_db::ai_policy::resolve_credential_secret(
        &mut *tx,
        org_id,
        user_id,
        &provider,
        personal_allowed,
    )
    .await
    .map_err(ApiError::from)?;
    let Some((credential_id, sealed_bytes)) = credential else {
        return Err(ApiError::AiCallRefused {
            reason: "ai_no_credential",
            message: format!(
                "no active '{provider}' credential is available for this call (org or personal)"
            ),
            surface: surface_key.to_string(),
        });
    };

    // Which owner the winning credential belongs to (for the `personal`
    // flag) + its kind, from presence-only summaries.
    let org_owned: bool = core_db::ai_policy::list_credential_summaries(
        &mut *tx,
        org_id,
        CredentialOwnerFilter::OrgOnly,
    )
    .await
    .map_err(ApiError::from)?
    .iter()
    .any(|s| s.id == credential_id);
    let personal_selected = !org_owned;
    let kind = core_db::ai_policy::list_credential_summaries(
        &mut *tx,
        org_id,
        if personal_selected {
            CredentialOwnerFilter::PersonalOf(user_id)
        } else {
            CredentialOwnerFilter::OrgOnly
        },
    )
    .await
    .map_err(ApiError::from)?
    .into_iter()
    .find(|s| s.id == credential_id)
    .map(|s| s.kind)
    .unwrap_or_else(|| "api_key".to_string());

    // Tamper check: an unopenable sealed credential degrades typed.
    let plaintext = crate::crypto::open(&state.config.secret_key, &sealed_bytes).map_err(|_| {
        ApiError::AiCallRefused {
            reason: "ai_no_credential",
            message: "the sealed credential for this call failed to decrypt (tampered or rotated)"
                .into(),
            surface: surface_key.to_string(),
        }
    })?;

    // Soft cap: warn + notify IN THE CALLER'S TX (the call proceeds).
    let mut warnings = Vec::new();
    if let Some(soft) = soft_limit {
        if used >= soft {
            warnings.push("ai_soft_cap_reached".to_string());
            core_db::notifications::insert(
                tx,
                Uuid::now_v7(),
                org_id,
                user_id,
                "ai_cap_warn",
                None,
                "Approaching your AI usage cap",
                Some(
                    format!(
                        "You used {used} of {soft} tokens this period; the hard cap stays binding."
                    )
                    .as_str(),
                ),
            )
            .await
            .map_err(ApiError::from)?;
        }
    }

    Ok(AuthorizedAiCall {
        surface_key: surface_key.to_string(),
        provider,
        model,
        resolution,
        credential_id,
        credential_kind: kind,
        credential_personal: personal_selected,
        plaintext_secret: plaintext,
        used_tokens: used,
        soft_limit,
        hard_limit,
        warnings,
    })
}

/// The completion edge's shared admission result. `plaintext_secret` holds
/// the OPENED bytes of the winning sealed credential — the caller must
/// NEVER log, emit, or persist them. Honest gap (unwrapping this outside a
/// forward call is a review red flag).
#[derive(Debug)]
pub struct AuthorizedAiCall {
    pub surface_key: String,
    pub provider: String,
    pub model: String,
    /// `requested` | `user_pick` | `org_pick` | `legacy_org_default` |
    /// `catalog_first`.
    pub resolution: &'static str,
    pub credential_id: Uuid,
    pub credential_kind: String,
    pub credential_personal: bool,
    pub plaintext_secret: Vec<u8>,
    pub used_tokens: i64,
    pub soft_limit: Option<i64>,
    pub hard_limit: Option<i64>,
    /// Soft-cap warnings (also ride `ai_cap_warn` notifications).
    pub warnings: Vec<String>,
}

/// POST /ai/me/authorize — the completion-edge resolution (typed refusals;
/// degrade-never-5xx), delegating to the shared pipeline.
#[utoipa::path(
    operation_id = "ai_my_authorize_post",
    post,
    path = "/api/ai/me/authorize",
    tag = "ai",
    security(("bearer" = [])),
    request_body = AiAuthorizeRequest,
    responses(
        (status = 200, body = AiAuthorizeResponse),
        (status = 403, description = "typed refusal: ai_surface_not_granted | ai_surface_disabled | ai_surface_internal | ai_model_not_allowed | ai_no_models_configured | ai_hard_cap_reached | ai_no_credential"),
        (status = 404, description = "rls_sparse")
    )
)]
pub async fn authorize(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<AiAuthorizeRequest>,
) -> ApiResult<Json<AiAuthorizeResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let surface_key = req.surface_key.trim().to_string();
    if surface_key.is_empty() {
        return Err(ApiError::Validation("surface_key is required".into()));
    }

    let mut tx = org_bound_tx(&state, org_id, user_id).await?;
    let call = authorize_surface_call(
        &state,
        &mut tx,
        org_id,
        user_id,
        &surface_key,
        req.provider.as_deref(),
        req.model.as_deref(),
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;

    Ok(Json(AiAuthorizeResponse {
        surface_key: call.surface_key,
        provider: call.provider,
        model: call.model,
        resolution: call.resolution.to_string(),
        credential: AiAuthorizedCredential {
            id: call.credential_id,
            kind: call.credential_kind,
            personal: call.credential_personal,
        },
        caps: AiCapState {
            used_tokens: call.used_tokens,
            soft_token_limit: call.soft_limit,
            hard_token_limit: call.hard_limit,
            pct_hard: pct_of(call.used_tokens, call.hard_limit),
        },
        warnings: call.warnings,
    }))
}

/// Model pick precedence (ADR 0008 §1): user pick → org pick → legacy flat
/// pair while still allowlisted → first enabled catalog entry. An invalid
/// / stale pick falls through honestly instead of poisoning the call.
async fn resolve_pick(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    surface_id: Uuid,
    surface_key: &str,
    allowlist: &[(String, String)],
) -> ApiResult<(String, String, &'static str)> {
    if let Some(pick) = core_db::ai_policy::get_user_pick(&mut *tx, org_id, user_id, surface_id)
        .await
        .map_err(ApiError::from)?
    {
        if allowlist.contains(&(pick.provider.clone(), pick.model.clone())) {
            return Ok((pick.provider, pick.model, "user_pick"));
        }
    }
    if let Some(pick) = core_db::ai_policy::get_org_pick(&mut *tx, org_id, surface_id)
        .await
        .map_err(ApiError::from)?
    {
        if allowlist.contains(&(pick.provider.clone(), pick.model.clone())) {
            return Ok((pick.provider, pick.model, "org_pick"));
        }
    }
    // Legacy flat pair (supersede posture, customization-aware): honored
    // only while the admin-allowlisted catalog still contains the pair.
    if let Some((provider, model)) = legacy_pair(&mut *tx, org_id).await? {
        if allowlist.contains(&(provider.clone(), model.clone())) {
            return Ok((provider, model, "legacy_org_default"));
        }
    }
    match allowlist.first() {
        Some((provider, model)) => Ok((provider.clone(), model.clone(), "catalog_first")),
        None => Err(ApiError::AiCallRefused {
            reason: "ai_no_models_configured",
            message: format!(
                "no admin-curated enabled models are in the catalog; ask your admin to curate \
                 at least one before '{surface_key}' calls resolve"
            ),
            surface: surface_key.to_string(),
        }),
    }
}
