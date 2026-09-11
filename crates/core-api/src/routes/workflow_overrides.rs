//! Per-client workflow overrides (Wave 12 lane 5, migration 0063).
//!
//! Purpose: the HTTP surface for client-scoped overrides of the org workflow
//! defaults — read the resolved document, read the typed registry, write one
//! override (set value / explicit disable), and clear one (inherit again).
//!
//! Contract posture:
//! - Resource-oriented REST on the v1 envelope (IF1–IF3): mutations ride the
//!   instant-first idempotency gate + If-Match version gate and carry the
//!   `mutation` metadata (seq/version/updated_at/before/after).
//! - Resolution precedence (documented on the domain registry): a client
//!   override wins over the org default; an explicit `disabled` row is a deny
//!   that wins over an org-enabled default; an absent row inherits.
//! - Typed-registry validation (§4.5 customizations-as-data): the key set is
//!   closed ([`core_domain::workflow_overrides`]); unknown keys, mistyped
//!   payloads, out-of-org references, non-entry statuses, statuses from a
//!   different board than the effective intake board, and disallowed disables
//!   are all typed 422s — never stored.
//! - Audited changes with provenance: every mutation appends an audit row, an
//!   `entity_writes` trail row (before/after — the undo substrate), and its
//!   `client.workflow_override_set|cleared` contract event in the SAME commit.
//! - Client-scoped: every path is anchored on the client id; a cross-org or
//!   unknown client is a 404 on reads and a typed `rls_sparse` inside
//!   mutations. RLS binds rows to the org (migration 0063).
//!
//! Non-purpose: no bulk import/export of override sets (the manifest owns the
//! export leg), no per-ticket overrides, no UI projection concerns.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    routing::get,
    Json, Router,
};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::events::types;
use core_domain::policy::Permission;
use core_domain::workflow_overrides as registry;
use core_domain::workflow_overrides::{
    ClientWorkflowResolution, OverrideState, Resolution, ResolutionSource, ResolvedDetail,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

pub fn routes(gen: crate::routes::ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(
            &format!("{base}/clients/{{id}}/workflow-overrides"),
            get(list_overrides),
        )
        .route(
            &format!("{base}/clients/{{id}}/workflow-overrides/{{key}}"),
            axum::routing::put(put_override).delete(clear_override),
        )
        .route(
            &format!("{base}/workflow-overrides/registry"),
            get(get_registry),
        )
}

fn manage_boards() -> Permission {
    Permission::ManageBoards
}

fn client_view() -> Permission {
    Permission::Matrix {
        module: core_domain::policy::Module::Clients,
        action: core_domain::policy::Action::View,
    }
}

// --- DTOs -----------------------------------------------------------------------

/// One registry declaration, projected for the settings/admin surface.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OverrideDeclarationView {
    pub key: String,
    /// board_ref | status_ref | sla_policy_ref | toggle.
    pub value_kind: String,
    pub allow_disable: bool,
    /// The settings-registry key carrying the org default for toggles.
    #[schema(nullable = true)]
    pub org_default_setting: Option<String>,
    pub label: String,
    pub description: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OverrideRegistryResponse {
    pub declarations: Vec<OverrideDeclarationView>,
}

/// PUT body: `{ "state": "set", "value": <typed payload> }` or
/// `{ "state": "disabled" }`, plus the instant-first parameters.
#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct PutOverrideRequest {
    #[serde(flatten)]
    pub instant: instant::MutationParams,
    /// "set" (value replaces the org default) or "disabled" (explicit deny
    /// that wins over an org-enabled default).
    pub state: String,
    /// The typed payload (uuid string for reference kinds, boolean for
    /// toggles); MUST be absent for "disabled".
    #[serde(default)]
    pub value: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OverrideMutationResponse {
    pub client_id: Uuid,
    pub key: String,
    pub resolution: Resolution,
    /// Which layer supplied the effective value.
    pub source: ResolutionSource,
    pub detail: Option<ResolvedDetail>,
    pub mutation: instant::MutationMeta,
}

fn snapshot(state: &str, value: Option<&Value>) -> Value {
    serde_json::json!({ "state": state, "value": value })
}

fn decl_or_422(key: &str) -> ApiResult<&'static registry::OverrideDeclaration> {
    registry::declaration(key).ok_or_else(|| {
        ApiError::Validation(format!(
            "'{key}' is not a registered workflow override (the registry is a closed set)"
        ))
    })
}

/// Resolves just the overridden construct's entry from the full document.
async fn resolved_entry(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    client_id: Uuid,
    key: &str,
) -> ApiResult<(Resolution, ResolutionSource, Option<ResolvedDetail>)> {
    let doc = core_db::workflow_overrides::resolve_client_workflow(tx, org_id, client_id).await?;
    doc.entries
        .into_iter()
        .find(|e| e.key == key)
        .map(|e| (e.resolution, e.source, e.detail))
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("registry key missing from resolution")))
}

/// GET /api/clients/{id}/workflow-overrides — the client's resolved workflow:
/// one entry per registry declaration with precedence outcome, provenance,
/// renderable details (names + semantic status categories), and drift issues.
#[utoipa::path(operation_id = "client_workflow_overrides_get", get,
    path = "/api/clients/{id}/workflow-overrides", tag = "workflow-overrides", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = ClientWorkflowResolution),
              (status = 404, description = "client not found in this org")))]
pub async fn list_overrides(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ClientWorkflowResolution>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, client_view())?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    if !core_db::workflow_overrides::client_exists(&mut tx, org_id, id).await? {
        return Err(ApiError::NotFound);
    }
    let doc = core_db::workflow_overrides::resolve_client_workflow(&mut tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(doc))
}

/// GET /api/workflow-overrides/registry — the closed set of overridable
/// constructs with their typed value kinds (the UI renders the editor FROM
/// this; policy §4.5: customizations are data against a typed registry).
#[utoipa::path(operation_id = "workflow_overrides_registry", get,
    path = "/api/workflow-overrides/registry", tag = "workflow-overrides", security(("bearer" = [])),
    responses((status = 200, body = OverrideRegistryResponse)))]
pub async fn get_registry() -> ApiResult<Json<OverrideRegistryResponse>> {
    Ok(Json(OverrideRegistryResponse {
        declarations: registry::declarations()
            .iter()
            .map(|d| OverrideDeclarationView {
                key: d.key.to_string(),
                value_kind: d.value_kind.as_str().to_string(),
                allow_disable: d.allow_disable,
                org_default_setting: d.org_default_setting.map(str::to_string),
                label: d.label.to_string(),
                description: d.description.to_string(),
            })
            .collect(),
    }))
}

/// PUT /api/clients/{id}/workflow-overrides/{key} — set a value or record an
/// explicit disable. Unknown keys, mistyped payloads, out-of-org references,
/// and disallowed disables are typed 422s before anything persists.
#[utoipa::path(operation_id = "client_workflow_override_put", put,
    path = "/api/clients/{id}/workflow-overrides/{key}", tag = "workflow-overrides", security(("bearer" = [])),
    request_body = PutOverrideRequest,
    responses((status = 200, body = OverrideMutationResponse),
              (status = 404, description = "client not found in this org (rls_sparse in-mutation)"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "unregistered key, mistyped value, invalid reference, or disallowed disable")))]
pub async fn put_override(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, key)): Path<(Uuid, String)>,
    Json(req): Json<PutOverrideRequest>,
) -> ApiResult<Json<OverrideMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_boards())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    // Registry validation FIRST (typed 422 before any transaction).
    let decl = decl_or_422(&key)?;
    let parsed = registry::parse_state(decl, &req.state, req.value.as_ref())
        .map_err(ApiError::Validation)?;
    let value_json = match parsed {
        OverrideState::Set(v) => Some(v.to_json()),
        OverrideState::Disabled => None,
    };

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let idem_key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "client_workflow_override",
                entity_id: Some(id),
                key: idem_key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        // Client scope: under RLS a foreign client is invisible, which
        // inside a mutation is the typed rls_sparse outcome (not 404).
        let exists = core_db::workflow_overrides::client_exists(&mut ctx.tx, org_id, id)
            .await
            .map_err(ApiError::from)?;
        if !exists {
            return Err(instant::mutation_scope_err(ApiError::NotFound));
        }

        // Reference validation against the org's constructs: out-of-org
        // references, non-entry statuses, cross-board statuses.
        if let OverrideState::Set(value) = &parsed {
            core_db::workflow_overrides::validate_value(&mut ctx.tx, org_id, id, decl, value)
                .await
                .map_err(ApiError::Validation)?;
        }

        let before_row =
            core_db::workflow_overrides::get_override(&mut ctx.tx, org_id, id, decl.key).await?;
        let (entity_id, is_create) = match &before_row {
            Some(row) => (row.id, false),
            None => (Uuid::now_v7(), true),
        };
        let before_json = before_row
            .as_ref()
            .map(|r| snapshot(&r.state, r.value.as_ref()));

        core_db::workflow_overrides::upsert_override(
            &mut ctx.tx,
            org_id,
            id,
            decl.key,
            state_str(&parsed),
            value_json.as_ref(),
            user_id,
        )
        .await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "client_workflow_override",
                entity_id,
                version: req.instant.version,
            },
        )
        .await?;

        let (resolution, source, detail) =
            resolved_entry(&mut ctx.tx, org_id, id, decl.key).await?;

        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "client.workflow_override_set".into(),
                entity_type: Some("client".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "key": decl.key,
                    "state": state_str(&parsed),
                    "value": value_json,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        ctx.emit(
            types::CLIENT_WORKFLOW_OVERRIDE_SET,
            "client",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ClientWorkflowOverrideSet {
                client_id: id,
                key: decl.key.to_string(),
                state: state_str(&parsed).to_string(),
                value: value_json.clone(),
                changed_by: Some(user_id),
                occurred_at: chrono::Utc::now(),
            })
            .unwrap_or_default(),
        )
        .await?;

        let after_json = snapshot(state_str(&parsed), value_json.as_ref());
        let updated_at = chrono::Utc::now();
        let stored = OverrideMutationResponse {
            client_id: id,
            key: decl.key.to_string(),
            resolution: resolution.clone(),
            source,
            detail: detail.clone(),
            mutation: instant::MutationMeta {
                seq: write_seq,
                version: write_seq,
                updated_at,
                before: before_json.clone(),
                after: Value::Null,
                idempotency_key: None,
            },
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "client_workflow_override",
                entity_id,
                op: if is_create { "create" } else { "update" },
                seq: write_seq,
                before: before_json,
                after: after_json,
                updated_at,
                key: idem_key.as_deref(),
                payload: &stored,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(OverrideMutationResponse {
                client_id: id,
                key: decl.key.to_string(),
                resolution,
                source,
                detail,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

fn state_str(parsed: &OverrideState) -> &'static str {
    match parsed {
        OverrideState::Set(_) => "set",
        OverrideState::Disabled => "disabled",
    }
}

/// DELETE /api/clients/{id}/workflow-overrides/{key} — remove the override so
/// the client inherits the org default again. Distinct from an explicit
/// disable; clearing an absent override is a no-op that still resolves.
#[utoipa::path(operation_id = "client_workflow_override_clear", delete,
    path = "/api/clients/{id}/workflow-overrides/{key}", tag = "workflow-overrides", security(("bearer" = [])),
    responses((status = 200, body = OverrideMutationResponse),
              (status = 404, description = "client not found in this org (rls_sparse in-mutation)"),
              (status = 422, description = "unknown override key")))]
pub async fn clear_override(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, key)): Path<(Uuid, String)>,
) -> ApiResult<Json<OverrideMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_boards())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let decl = decl_or_422(&key)?;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let exists = core_db::workflow_overrides::client_exists(&mut ctx.tx, org_id, id)
            .await
            .map_err(ApiError::from)?;
        if !exists {
            return Err(instant::mutation_scope_err(ApiError::NotFound));
        }
        let before_row =
            core_db::workflow_overrides::get_override(&mut ctx.tx, org_id, id, decl.key).await?;
        let entity_id = before_row
            .as_ref()
            .map(|r| r.id)
            .unwrap_or_else(Uuid::now_v7);
        let before_json = before_row
            .as_ref()
            .map(|r| snapshot(&r.state, r.value.as_ref()));

        let removed =
            core_db::workflow_overrides::delete_override(&mut ctx.tx, org_id, id, decl.key).await?;

        if removed {
            core_db::audit::insert(
                &mut *ctx.tx,
                &AuditEvent {
                    id: Uuid::now_v7(),
                    org_id,
                    actor: Actor::Human(user_id),
                    action: "client.workflow_override_cleared".into(),
                    entity_type: Some("client".into()),
                    entity_id: Some(id.to_string()),
                    detail: serde_json::json!({ "key": decl.key, "prior": before_json }),
                    ip: None,
                    occurred_at: chrono::Utc::now(),
                },
            )
            .await?;
            ctx.emit(
                types::CLIENT_WORKFLOW_OVERRIDE_CLEARED,
                "client",
                Some(id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::ClientWorkflowOverrideCleared {
                    client_id: id,
                    key: decl.key.to_string(),
                    cleared_by: Some(user_id),
                    occurred_at: chrono::Utc::now(),
                })
                .unwrap_or_default(),
            )
            .await?;
        }

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "client_workflow_override",
                entity_id,
                version: None,
            },
        )
        .await?;
        let (resolution, source, detail) =
            resolved_entry(&mut ctx.tx, org_id, id, decl.key).await?;

        let updated_at = chrono::Utc::now();
        let stored = OverrideMutationResponse {
            client_id: id,
            key: decl.key.to_string(),
            resolution: resolution.clone(),
            source,
            detail: detail.clone(),
            mutation: instant::MutationMeta {
                seq: write_seq,
                version: write_seq,
                updated_at,
                before: before_json.clone(),
                after: Value::Null,
                idempotency_key: None,
            },
        };
        // The revert is its own audited write on the trail (append-only),
        // flagged is_undo so provenance never rewrites the prior rows.
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "client_workflow_override",
                entity_id,
                op: "clear",
                seq: write_seq,
                before: before_json,
                after: Value::Null,
                updated_at,
                key: None,
                payload: &stored,
                provenance: instant::ProvenanceContext {
                    is_undo: true,
                    ..Default::default()
                },
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(OverrideMutationResponse {
                client_id: id,
                key: decl.key.to_string(),
                resolution,
                source,
                detail,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}
