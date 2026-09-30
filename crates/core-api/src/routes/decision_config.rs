//! Decision-harness admin config (ADR 0010 §4 folded UI scope + ADR 0011
//! §3 honesty): the HTTP seam the settings/ai decision-harness leg
//! rides. The harness schema, edge and curated starter sets landed in
//! Wave 23 lanes 2–4, but the per-surface decision config (mode /
//! enabled / pinned model / forked `state_template`) and the org master
//! switch `ai.features_enabled` had NO HTTP surface — an honest gap that
//! blocked the decision-UI wiring. Closed here riding the existing seams
//! exactly — no new schema, no consumer dispatch wiring:
//!
//! - GET  /settings/ai/decision — per-decision-surface overview: fork
//!   state, mode, enabled, pinned model, master switch, latest curated
//!   starter version (the "v2 available" provenance input) and the
//!   decision-capable-catalog presence (the honest "decision model not
//!   configured" input).
//! - PUT  /settings/ai/decision/features — the org master switch.
//! - PUT  /settings/ai/decision/{surface_key}/config — fork-on-first-edit
//!   config editor: `core_decision::validate_config` refusals ride the
//!   typed reason verbatim (`ai_decision_config_refused`); thresholds
//!   ride this route too (they live in the question map).
//! - POST /settings/ai/decision/{surface_key}/preview — compose the
//!   state for a REAL org ticket/alert per the template (derived
//!   fields computed in code — after_hours / within_sla_window /
//!   elapsed_sum / minutes_since_created / overage_minutes_remaining —
//!   never raw temporal leaves), optionally FIRE one shadow run through
//!   the real `decide()` edge: admission pipeline, shadow answer store,
//!   usage ledger, caps, degrade-never-5xx typed quiet.
//!
//! Non-purpose: no secret bytes; no worker fan-out dispatch (consumers
//! own it); no chat transport.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome, MutationParams};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use core_decision::{validate_config, DerivedField, SurfaceConfig, TemplateField};
use core_domain::entities::{Actor, AuditEvent, Principal};
use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

const DECISION_ENTITY: &str = "ai_decision_surface_config";
const FEATURE_ENTITY: &str = "ai_features";

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

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DecisionSurfaceOverview {
    pub surface_key: String,
    pub label: String,
    /// Internal decision surfaces (search_rerank / llm_verify /
    /// tool_routing / loop_termination) are never grant targets; the UI
    /// renders their posture note-only.
    pub internal: bool,
    pub description: String,
    pub starter_version: i32,
    /// None while no curated set exists for the surface version.
    pub latest_starter_version: Option<i32>,
    pub customized: bool,
    /// False while the org never configured the surface — the ADR 0011
    /// §3 posture: silently quiet, shown with its what-activates note.
    pub configured: bool,
    pub mode: String,
    pub enabled: bool,
    pub model_provider: Option<String>,
    pub model_id: Option<String>,
    pub decision_model_curated: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DecisionOverviewResponse {
    pub features_enabled: bool,
    pub surfaces: Vec<DecisionSurfaceOverview>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DecisionFeatureMutationResponse {
    pub features_enabled: bool,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DecisionConfigMutationResponse {
    pub surface: DecisionSurfaceOverview,
    pub mutation: instant::MutationMeta,
}

// ---------------------------------------------------------------------------
// GET /settings/ai/decision
// ---------------------------------------------------------------------------

#[utoipa::path(
    operation_id = "settings_ai_decision_overview",
    get,
    path = "/api/settings/ai/decision",
    tag = "settings",
    security(("bearer" = [])),
    responses((status = 200, body = DecisionOverviewResponse), (status = 403))
)]
pub async fn get_decision_overview(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<DecisionOverviewResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_ai_admin(&authed.principal, Action::View)?;
    let org_id = authed.principal.user.org_id;

    let mut read_tx =
        super::ai_policy::org_bound_tx(&state, org_id, authed.principal.user.id).await?;
    let features_enabled = core_db::ai_policy::features_enabled(&mut *read_tx, org_id)
        .await
        .map_err(ApiError::from)?;
    let surfaces = core_db::ai_policy::list_surfaces(&mut *read_tx, org_id)
        .await
        .map_err(ApiError::from)?;
    let catalog = core_db::ai_policy::list_catalog(&mut *read_tx, org_id)
        .await
        .map_err(ApiError::from)?;
    let decision_model_curated = catalog
        .iter()
        .any(|entry| entry.enabled && entry_is_decision_capable(&entry.capabilities));

    let mut out = Vec::new();
    for surface in surfaces
        .iter()
        .filter(|s| s.kind == "decision" || s.kind == "internal")
    {
        let cfg = core_db::decision::get_surface_config(&mut *read_tx, org_id, &surface.key)
            .await
            .map_err(ApiError::from)?;
        let latest: Option<i32> = sqlx::query_scalar(
            "SELECT MAX(version) FROM ai_decision_question_sets WHERE surface_key = $1",
        )
        .bind(&surface.key)
        .fetch_one(&mut *read_tx)
        .await
        .ok()
        .flatten();
        out.push(DecisionSurfaceOverview {
            surface_key: surface.key.clone(),
            label: surface.label.clone(),
            internal: surface.kind == "internal",
            description: surface.description.clone(),
            starter_version: cfg.as_ref().map(|c| c.starter_version).unwrap_or(1),
            latest_starter_version: latest,
            customized: cfg.as_ref().map(|c| c.customized).unwrap_or(false),
            configured: cfg.is_some(),
            mode: cfg.as_ref().map(|c| c.mode.clone()).unwrap_or_default(),
            enabled: cfg.as_ref().map(|c| c.enabled).unwrap_or(false),
            model_provider: cfg.as_ref().and_then(|c| c.model_provider.clone()),
            model_id: cfg.as_ref().and_then(|c| c.model_id.clone()),
            decision_model_curated,
        });
    }
    read_tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(DecisionOverviewResponse {
        features_enabled,
        surfaces: out,
    }))
}

// ---------------------------------------------------------------------------
// PUT /settings/ai/decision/features — the org master switch
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PutDecisionFeaturesRequest {
    #[serde(default)]
    pub instant: MutationParams,
    pub enabled: bool,
}

#[utoipa::path(
    operation_id = "settings_ai_decision_features_put",
    put,
    path = "/api/settings/ai/decision/features",
    tag = "settings",
    security(("bearer" = [])),
    request_body = PutDecisionFeaturesRequest,
    responses((status = 200, body = DecisionFeatureMutationResponse), (status = 403))
)]
pub async fn put_decision_features(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<PutDecisionFeaturesRequest>,
) -> ApiResult<Json<DecisionFeatureMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_ai_admin(&authed.principal, Action::Edit)?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let idem = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        let claim = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: FEATURE_ENTITY,
                entity_id: None,
                key: idem.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?;
        if let InstantClaim::Replay(stored) = claim {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        core_db::settings::upsert_org_setting(
            &mut ctx.tx,
            ctx.org_id,
            core_domain::settings::AI_FEATURES_ENABLED,
            &json!(req.enabled),
            ctx.user_id,
        )
        .await
        .map_err(ApiError::from)?;
        let snapshot = json!({ "org_id": ctx.org_id, "features_enabled": req.enabled });
        ctx.emit(
            types::AI_DECISION_STATE_CHANGED,
            FEATURE_ENTITY,
            None,
            &snapshot,
        )
        .await?;
        audit_row(
            &mut ctx.tx,
            ctx.org_id,
            ctx.user_id,
            "settings.ai_features_toggled",
            FEATURE_ENTITY,
            ctx.org_id,
            snapshot.clone(),
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(DecisionFeatureMutationResponse {
                features_enabled: req.enabled,
                mutation: instant::MutationMeta {
                    seq: 0,
                    version: 0,
                    updated_at: chrono::Utc::now(),
                    before: None,
                    after: snapshot,
                    idempotency_key: idem.clone(),
                },
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

// ---------------------------------------------------------------------------
// PUT /settings/ai/decision/{surface_key}/config — fork-on-first-edit
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PutDecisionConfigRequest {
    #[serde(default)]
    pub instant: MutationParams,
    /// `shadow` (record only) | `live`. Shadow is the landing posture.
    pub mode: Option<String>,
    pub enabled: Option<bool>,
    /// Pinned decision model (e.g. `typesafe` / `jev-1.13.0`, never a
    /// floating tag — answer provenance records exactly this id).
    pub model_provider: Option<String>,
    pub model_id: Option<String>,
    /// Full `SurfaceConfig` JSON (state_template + questions +
    /// thresholds). Threshold editors ride this.
    pub config: Option<Value>,
    /// Discard the fork; re-reference the LATEST curated starter set.
    pub reset_to_starter: Option<bool>,
}

#[utoipa::path(
    operation_id = "settings_ai_decision_config_put",
    put,
    path = "/api/settings/ai/decision/{surface_key}/config",
    tag = "settings",
    security(("bearer" = [])),
    request_body = PutDecisionConfigRequest,
    responses(
        (status = 200, body = DecisionConfigMutationResponse),
        (status = 403),
        (status = 422, description = "unknown surface, or the config rode a typed refusal (reason rides verbatim)"),
    )
)]
pub async fn put_decision_config(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(surface_key): Path<String>,
    Json(req): Json<PutDecisionConfigRequest>,
) -> ApiResult<Json<DecisionConfigMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_ai_admin(&authed.principal, Action::Edit)?;
    let surface_key = surface_key.trim().to_string();
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let idem = req.instant.idempotency_key.clone();

    if let Some(mode) = req.mode.as_deref() {
        if mode != "shadow" && mode != "live" {
            return Err(ApiError::Validation(
                "mode must be 'shadow' or 'live'".into(),
            ));
        }
    }

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        let claim = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: DECISION_ENTITY,
                entity_id: None,
                key: idem.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?;
        if let InstantClaim::Replay(stored) = claim {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }

        let surface = core_db::ai_policy::get_surface_by_key(&mut *ctx.tx, ctx.org_id, &surface_key)
            .await
            .map_err(ApiError::from)?
            .ok_or_else(|| ApiError::Validation(format!("unknown surface '{surface_key}'")))?;
        if surface.kind != "decision" && surface.kind != "internal" {
            return Err(ApiError::Validation(format!(
                "surface '{}' is not a decision surface",
                surface.key
            )));
        }

        let mut row: core_db::decision::DecisionSurfaceConfig =
            core_db::decision::get_surface_config(&mut *ctx.tx, ctx.org_id, &surface.key)
                .await
                .map_err(ApiError::from)?
                .unwrap_or(core_db::decision::DecisionSurfaceConfig {
                    org_id: ctx.org_id,
                    surface_key: surface.key.clone(),
                    starter_version: 1,
                    customized: false,
                    config: None,
                    mode: "shadow".into(),
                    model_provider: None,
                    model_id: None,
                    enabled: false,
                });

        if req.reset_to_starter == Some(true) {
            row.starter_version = sqlx::query_scalar::<_, i32>(
                "SELECT COALESCE(MAX(version), 1) FROM ai_decision_question_sets WHERE surface_key = $1",
            )
            .bind(&surface.key)
            .fetch_one(&mut *ctx.tx)
            .await
            .unwrap_or(row.starter_version);
            row.customized = false;
            row.config = None;
        } else if let Some(config) = req.config.as_ref() {
            let parsed: SurfaceConfig = serde_json::from_value(config.clone())
                .map_err(|e| ApiError::Validation(format!("invalid decision config: {e}")))?;
            if let Err((reason, message)) = validate_config(&parsed) {
                return Err(ApiError::AiCallRefused {
                    reason: "ai_decision_config_refused",
                    message: format!("the state template was refused: {reason} — {message}"),
                    surface: surface.key.clone(),
                });
            }
            // Fork-on-first-edit: once customized, the fork REPLACES the
            // starter reference (curated "v2 available" banners stop —
            // honest provenance; orgs re-inherit only on reset).
            row.customized = true;
            row.config = Some(config.clone());
        }
        if let Some(mode) = req.mode.clone() {
            row.mode = mode;
        }
        if let Some(enabled) = req.enabled {
            row.enabled = enabled;
        }
        if req.model_provider.is_some() || req.model_id.is_some() {
            row.model_provider = req.model_provider.clone();
            row.model_id = req.model_id.clone();
        }
        core_db::decision::upsert_surface_config(&mut ctx.tx, &row, ctx.user_id)
            .await
            .map_err(ApiError::from)?;

        let entity_id = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM ai_decision_surface_configs WHERE org_id = $1 AND surface_key = $2",
        )
        .bind(ctx.org_id)
        .bind(&surface.key)
        .fetch_one(&mut *ctx.tx)
        .await
        .map_err(ApiError::from)?;

        let snapshot = json!({
            "surface_key": surface.key,
            "mode": row.mode,
            "enabled": row.enabled,
            "customized": row.customized,
            "starter_version": row.starter_version,
            "model_provider": row.model_provider,
            "model_id": row.model_id,
        });
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: DECISION_ENTITY,
                entity_id,
                version: req.instant.version,
            },
        )
        .await?;

        ctx.emit(
            types::AI_DECISION_STATE_CHANGED,
            DECISION_ENTITY,
            Some(entity_id.to_string().as_str()),
            &json!({ "surface_key": surface.key, "mode": row.mode, "enabled": row.enabled }),
        )
        .await?;
        audit_row(
            &mut ctx.tx,
            ctx.org_id,
            ctx.user_id,
            "settings.ai_decision_config_updated",
            DECISION_ENTITY,
            entity_id,
            snapshot.clone(),
        )
        .await?;

        let updated_at = chrono::Utc::now();
        let status = surface_status(&surface, &row);
        let body = DecisionConfigMutationResponse {
            surface: status,
            mutation: instant::MutationMeta {
                seq: write_seq,
                version: write_seq,
                updated_at,
                before: None,
                after: snapshot.clone(),
                idempotency_key: idem.clone(),
            },
        };
        let _ = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: DECISION_ENTITY,
                entity_id,
                op: "update",
                seq: write_seq,
                before: None,
                after: snapshot,
                provenance: Default::default(),
                updated_at,
                key: idem.as_deref(),
                payload: &body,
            },
        )
        .await?;
        Ok((ctx, MutationOutcome::Applied(body)))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

fn surface_status(
    surface: &core_db::ai_policy::AiSurface,
    row: &core_db::decision::DecisionSurfaceConfig,
) -> DecisionSurfaceOverview {
    DecisionSurfaceOverview {
        surface_key: surface.key.clone(),
        label: surface.label.clone(),
        internal: surface.kind == "internal",
        description: surface.description.clone(),
        starter_version: row.starter_version,
        latest_starter_version: None,
        customized: row.customized,
        configured: true,
        mode: row.mode.clone(),
        enabled: row.enabled,
        model_provider: row.model_provider.clone(),
        model_id: row.model_id.clone(),
        decision_model_curated: false,
    }
}

// ---------------------------------------------------------------------------
// POST /settings/ai/decision/{surface_key}/preview
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DecisionPreviewRequest {
    /// `ticket` | `alert` — the REAL org entity to compose from.
    pub entity_type: String,
    pub entity_id: Uuid,
    /// When true, one shadow decision run rides the REAL edge
    /// (admission, shadow answer store, ledger, caps, typed quiet).
    pub fire: Option<bool>,
}

#[utoipa::path(
    operation_id = "settings_ai_decision_preview",
    post,
    path = "/api/settings/ai/decision/{surface_key}/preview",
    tag = "settings",
    security(("bearer" = [])),
    request_body = DecisionPreviewRequest,
    responses((status = 200, body = serde_json::Value), (status = 403))
)]
pub async fn post_decision_preview(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(surface_key): Path<String>,
    Json(req): Json<DecisionPreviewRequest>,
) -> ApiResult<Json<Value>> {
    use crate::routes::decision_edge::{self, DecisionAsk};

    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_ai_admin(&authed.principal, Action::View)?;
    let surface_key = surface_key.trim().to_string();
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    if req.entity_type != "ticket" && req.entity_type != "alert" {
        return Err(ApiError::Validation(
            "entity_type must be 'ticket' or 'alert'".into(),
        ));
    }

    let mut tx = super::ai_policy::org_bound_tx(&state, org_id, user_id).await?;
    let cfg = core_db::decision::get_surface_config(&mut *tx, org_id, &surface_key)
        .await
        .map_err(ApiError::from)?;
    let Some(cfg) = cfg else {
        // Honest ADR 0011 §3 preview: unconfigured ⇒ quiet.
        return Ok(Json(json!({
            "quiet": "ai_decision_unconfigured",
            "state": Value::Null,
            "derived_fields": Value::Null,
            "run": Value::Null,
        })));
    };

    let snapshot = match req.entity_type.as_str() {
        "ticket" => load_ticket_snapshot(&mut tx, org_id, req.entity_id).await?,
        _ => load_alert_snapshot(&mut tx, org_id, req.entity_id).await?,
    };
    if !snapshot["ticket"].is_object() && !snapshot["alert"].is_object() {
        return Err(ApiError::Validation(
            "the entity is not live in this organization".into(),
        ));
    }

    let mut tx = super::ai_policy::org_bound_tx(&state, org_id, user_id).await?;
    let resolved: SurfaceConfig = decision_edge::effective_config(&mut tx, &surface_key, &cfg)
        .await
        .map_err(|quiet| ApiError::AiCallRefused {
            reason: "ai_decision_unresolved",
            message: quiet.reason(),
            surface: surface_key.clone(),
        })?;

    // The curated starter the fork detached from (fork-provenance "v2"
    // diff) — included when the org is customized.
    let starter: Option<SurfaceConfig> = if cfg.customized {
        core_db::decision::get_question_set(tx.as_mut(), &surface_key, cfg.starter_version)
            .await
            .ok()
            .flatten()
            .and_then(|set| serde_json::from_value::<core_decision::starters::StarterSet>(set).ok())
            .map(|w| w.config)
    } else {
        None
    };

    let composed = compose_state(&resolved, &snapshot);
    tx.rollback().await.map_err(ApiError::from)?;

    let run_json = if req.fire == Some(true) {
        let mut tx = super::ai_policy::org_bound_tx(&state, org_id, user_id).await?;
        let inventories = org_inventories(&mut tx, org_id).await;
        tx.commit().await.map_err(ApiError::from)?;
        let ask = DecisionAsk {
            org_id,
            user_id,
            surface_key: &surface_key,
            entity_type: &req.entity_type,
            entity_id: Some(req.entity_id),
            state: composed.state.clone(),
            request_id: format!("decision-preview-{}", Uuid::now_v7()),
            inventories: inventories.clone(),
        };
        match decision_edge::decide(&state, ask).await? {
            Ok(run) => Some(json!({
                "status": run.mode,
                "provider": run.provider,
                "model": run.model,
                "answers": decision_edge::answers_to_json(&run.config, &run.response, &inventories),
            })),
            Err(quiet) => Some(json!({
                "status": "quiet",
                "reason": quiet.reason(),
            })),
        }
    } else {
        None
    };

    Ok(Json(json!({
        "surface_key": surface_key,
        "entity_type": req.entity_type,
        "entity_id": req.entity_id,
        "state": composed.state,
        "derived_fields": composed.derived,
        "run": run_json,
        "starter": starter.and_then(|s| serde_json::to_value(&s).ok()),
        "active_config": serde_json::to_value(&resolved).ok(),
    })))
}

// ---------------------------------------------------------------------------
// snapshot loading + ADR 0011 §4 composition
// ---------------------------------------------------------------------------

/// The real ticket snapshot: ticket + client + the ACTIVE contract
/// (included minutes / overage rate) + elapsed minutes — the arithmetic
/// happens HERE, never in the model (ADR 0011 §4).
async fn load_ticket_snapshot(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    ticket_id: Uuid,
) -> ApiResult<Value> {
    let row: Option<(Value, Value, Option<Value>)> = sqlx::query_as(
        r#"SELECT to_jsonb(t.*) AS ticket,
                  COALESCE(to_jsonb(c.*), '{}'::jsonb) AS client,
                  (SELECT jsonb_build_object(
                      'name', k.name, 'number', k.number,
                      'included_minutes', k.included_minutes,
                      'overage_rate_minor', k.overage_rate_minor)
                   FROM contracts k
                   WHERE k.org_id = $1 AND k.client_id = t.client_id
                     AND k.status = 'active'
                   LIMIT 1) AS contract
           FROM tickets t LEFT JOIN clients c ON c.id = t.client_id
           WHERE t.id = $2 AND t.org_id = $1"#,
    )
    .bind(org_id)
    .bind(ticket_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::from)?;
    let Some((ticket, client, contract)) = row else {
        return Ok(json!({}));
    };
    let elapsed = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(minutes), 0) FROM time_entries
         WHERE org_id = $1 AND ticket_id = $2",
    )
    .bind(org_id)
    .bind(ticket_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap_or(0);
    let mut snapshot = json!({
        "ticket": ticket,
        "client": client,
        "contract": contract.unwrap_or(json!({})),
        "org": {"boards": org_board_names(tx, org_id).await},
    });
    snapshot["ticket"]["elapsed_sum"] = json!(elapsed);
    Ok(snapshot)
}

async fn org_board_names(exec: &mut sqlx::PgConnection, org_id: Uuid) -> Vec<String> {
    sqlx::query_scalar::<_, String>("SELECT name FROM boards WHERE org_id = $1 ORDER BY name")
        .bind(org_id)
        .fetch_all(exec)
        .await
        .unwrap_or_default()
}

async fn load_alert_snapshot(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    alert_id: Uuid,
) -> ApiResult<Value> {
    let row: Option<Value> = sqlx::query_scalar(
        r#"SELECT to_jsonb(a.*) FROM alerts a
           WHERE a.id = $2 AND a.org_id = $1"#,
    )
    .bind(org_id)
    .bind(alert_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::from)?;
    let Some(alert) = row else {
        return Ok(json!({}));
    };
    // The asset name rides the alert attributes when the source.stamped
    // it — the alert table carries no asset FK to mirror faithfully.
    let asset_name = alert
        .pointer("/attributes/asset_name")
        .or_else(|| alert.pointer("/attributes/asset/name"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let elapsed = alert
        .pointer("/attributes/incident_count")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let mut snapshot = json!({ "alert": alert, "asset": { "name": asset_name } });
    snapshot["alert"]["elapsed_sum"] = json!(elapsed);
    Ok(snapshot)
}

/// Org board names + the granted MCP tool capability inventory: the
/// decision_tool_routing Choice set (the gate NARROWS the
/// already-granted inventory — never widens it).
async fn org_inventories(
    exec: &mut sqlx::PgConnection,
    org_id: Uuid,
) -> std::collections::HashMap<String, Vec<String>> {
    let mut inventories = std::collections::HashMap::new();
    let boards = org_board_names(exec, org_id).await;
    // dispatch queues mirror the same list: in this product a dispatch
    // queue IS a board's queue (worker decision.rs posture mirrored).
    inventories.insert("org.boards".into(), boards.clone());
    inventories.insert("org.dispatch_queues".into(), boards);
    let tools = core_db::agent_platform::list_capability_grants(&mut *exec, org_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|g| g.mode != "revoked")
        .map(|g| g.capability)
        .collect::<Vec<String>>();
    inventories.insert("org.tools".into(), tools.clone());
    inventories.insert("granted_tools".into(), tools.clone());
    inventories.insert("org.granted_tools".into(), tools);
    inventories
}

fn json_stringify_leaf(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Resolve one composition dot-path into the snapshot.
fn resolve_path<'a>(snapshot: &'a Value, path: &str) -> Option<&'a Value> {
    let mut node = snapshot;
    for segment in path.split('.') {
        node = node.get(segment)?;
    }
    Some(node)
}

struct ComposedState {
    state: Value,
    derived: Value,
}

/// Compose the state per the template — derived fields computed HERE
/// (ADR 0011 §4: temporal/ordinal math is pre-computed, never asked).
fn compose_state(config: &SurfaceConfig, snapshot: &Value) -> ComposedState {
    let now = chrono::Utc::now();
    // The entity kind narrows where the temporal leaves + the elapsed
    // sum live (ticket vs alert snapshot shapes).
    let is_ticket = snapshot
        .get("ticket")
        .map(Value::is_object)
        .unwrap_or(false);
    let entity_root = if is_ticket { "ticket" } else { "alert" };
    let due_at = snapshot
        .pointer("/ticket/due_at")
        .and_then(Value::as_str)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc));
    let created_at = snapshot
        .pointer("/ticket/created_at")
        .and_then(Value::as_str)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc));
    let alert_occurred = snapshot
        .pointer("/alert/occurred_at")
        .and_then(Value::as_str)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc));
    let elapsed = snapshot
        .pointer(&format!("/{entity_root}/elapsed_sum"))
        .and_then(Value::as_i64)
        .unwrap_or(0);

    let mut fields: Map<String, Value> = Map::new();
    let mut derived: Map<String, Value> = Map::new();
    for (name, field) in &config.state_template.fields {
        match field {
            TemplateField::From { path, max_chars } => {
                let raw = resolve_path(snapshot, path)
                    .map(json_stringify_leaf)
                    .unwrap_or_default();
                // 512 mirrors core-decision's DEFAULT_FIELD_MAX_CHARS.
                let cap = max_chars.unwrap_or(512);
                fields.insert(
                    name.clone(),
                    json!(raw.chars().take(cap).collect::<String>()),
                );
            }
            TemplateField::Literal { value } => {
                fields.insert(
                    name.clone(),
                    serde_json::to_value(value).unwrap_or(Value::Null),
                );
            }
            TemplateField::Derived { name: compute } => {
                let computed = match compute {
                    DerivedField::AfterHours => json!(after_hours(now)),
                    DerivedField::WithinSlaWindow => json!(within_sla_window(due_at, now)),
                    DerivedField::ElapsedSum => json!(elapsed),
                    DerivedField::MinutesSinceCreated => {
                        let at = created_at.or(alert_occurred);
                        json!(at
                            .map(|t| now.signed_duration_since(t).num_minutes())
                            .unwrap_or(0))
                    }
                    DerivedField::OverageMinutesRemaining => {
                        let included = snapshot
                            .pointer("/contract/included_minutes")
                            .and_then(Value::as_i64)
                            .unwrap_or(0);
                        json!(included - elapsed)
                    }
                };
                derived.insert(name.clone(), computed);
            }
        }
    }
    let _ = is_ticket;
    let derived_value = Value::Object(derived);
    if let Value::Object(map) = &derived_value {
        for (k, v) in map {
            fields.insert(k.clone(), v.clone());
        }
    }
    ComposedState {
        state: Value::Object(fields),
        derived: derived_value,
    }
}

/// Flags a catalog entry decision-capable: `capabilities` carries the
/// `decision` edge tag on an ENABLED entry (ADR 0011 §3 posture:
/// catalog-curated is not credential-wired — the honest "decision model
/// not configured" posture holds while the org credential slot is empty
/// even with the entry curated).
fn entry_is_decision_capable(entry_capabilities: &Value) -> bool {
    entry_capabilities
        .as_object()
        .map(|o| o.contains_key("decision"))
        .unwrap_or(false)
}

/// Worker-mirrored after-hours semantics (worker decision.rs): the
/// derived field is pre-computed in code — never a question.
pub fn after_hours(at: chrono::DateTime<chrono::Utc>) -> bool {
    use chrono::{Datelike, Timelike};
    let weekday = at.weekday();
    let hour = at.hour();
    !(matches!(
        weekday,
        chrono::Weekday::Mon
            | chrono::Weekday::Tue
            | chrono::Weekday::Wed
            | chrono::Weekday::Thu
            | chrono::Weekday::Fri
    ) && (8..18).contains(&hour))
}

fn within_sla_window(
    due_at: Option<chrono::DateTime<chrono::Utc>>,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    due_at.is_some_and(|due| due > now)
}

// ---------------------------------------------------------------------------
// Wave 41 lane 12: the shadow run log (pack state 11 + 12)
//
// Recording already happened: every answered run — shadow or live — has been
// in the append-only `decision_answers` store since migration 0090. This is
// the READ the settings pack asks for, and the two numbers it shows are
// measured, not narrated:
//
//   ABOVE THRESHOLD — the run's answer confidence against the surface's own
//     `act` threshold (ADR 0010 §5), so moving a threshold in the drawer
//     visibly moves this count.
//   AGREEMENT — a shadow run proposes a label; the label the ticket actually
//     reached is the human's answer. Comparing them is the only agreement
//     signal the product has, and it is a real measurement rather than a guess.
// ---------------------------------------------------------------------------

/// One answered run, in the words the run log draws.
#[derive(Debug, Serialize, ToSchema)]
pub struct DecisionRunView {
    pub id: uuid::Uuid,
    pub mode: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub model: String,
    /// The ticket the run was about, when it was about one.
    pub ticket_id: Option<uuid::Uuid>,
    pub ticket_number: Option<String>,
    pub ticket_title: Option<String>,
    /// What it would have said: the picked label per question, with the
    /// confidence that decided it.
    pub answers: Vec<DecisionRunAnswer>,
    /// True when any answer cleared the surface's `act` threshold.
    pub above_threshold: bool,
    /// The highest confidence in the run, for the log's meter.
    pub top_confidence: Option<f64>,
    /// The status the ticket sits in now — the human's answer, and the only
    /// thing agreement can be measured against.
    pub current_status: Option<String>,
    /// True when a picked label matches the status the ticket actually
    /// reached. False when the run disagreed, None when the ticket moved on
    /// before its status could be compared.
    pub agreed: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DecisionRunAnswer {
    pub question_id: String,
    pub kind: String,
    /// The label the model would have used, in words. Never an index.
    pub label: String,
    pub confidence: Option<f64>,
    /// True when this answer cleared the question's own `act` threshold.
    pub above_threshold: bool,
}

/// The four figures the run log leads with.
#[derive(Debug, Serialize, ToSchema)]
pub struct DecisionRunSummary {
    /// Runs recorded in the window.
    pub runs: i64,
    /// Runs with at least one answer above its `act` threshold.
    pub above_threshold: i64,
    /// Runs whose proposal matches the status the ticket actually reached.
    pub agreed: i64,
    /// Runs that could be compared at all (a ticket that still exists).
    pub comparable: i64,
    /// The `act` threshold in force, so the log and the drawer agree.
    pub threshold: f64,
    /// False when the product has never recorded a run for this surface, so
    /// the page can say "no evidence yet" rather than showing zeros.
    pub has_evidence: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DecisionRunsResponse {
    pub surface_key: String,
    pub days: i32,
    pub summary: DecisionRunSummary,
    pub runs: Vec<DecisionRunView>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct DecisionRunsQuery {
    /// Window in days, 1..=90. Default 7 — the log's own window.
    pub days: Option<i32>,
    pub limit: Option<i64>,
}

/// GET /api/settings/ai/decision/{surface_key}/runs — the shadow run log.
#[utoipa::path(
    operation_id = "settings_ai_decision_runs",
    get,
    path = "/api/settings/ai/decision/{surface_key}/runs",
    tag = "settings",
    security(("bearer" = [])),
    params(("surface_key" = String, Path), DecisionRunsQuery),
    responses((status = 200, body = DecisionRunsResponse), (status = 404))
)]
pub async fn get_decision_runs(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(surface_key): Path<String>,
    Query(q): Query<DecisionRunsQuery>,
) -> ApiResult<Json<DecisionRunsResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_ai_admin(&authed.principal, Action::View)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let days = q.days.unwrap_or(7).clamp(1, 90);
    let limit = q.limit.unwrap_or(50).clamp(1, 500);

    let payload = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        // Thresholds come from the org's own config, so the log's "above
        // threshold" is the same rule the consumers act on: the forked
        // questions when the org edited them, the curated starter set
        // otherwise.
        let config = core_db::decision::get_surface_config(&mut ctx.tx, org_id, &surface_key)
            .await?
            .ok_or(ApiError::NotFound)?;
        let document = match config.config.clone() {
            Some(fork) => fork,
            None => core_db::decision::get_question_set(
                &mut *ctx.tx,
                &surface_key,
                config.starter_version,
            )
            .await?
            .ok_or(ApiError::NotFound)?,
        };
        let effective: SurfaceConfig = serde_json::from_value(document).map_err(|e| {
            ApiError::Internal(anyhow::anyhow!("stored question set is unreadable: {e}"))
        })?;
        let rows = core_db::decision::list_runs(
            &mut *ctx.tx,
            org_id,
            &surface_key,
            days,
            limit,
        )
        .await?;
        Ok((ctx, (effective, rows)))
    })
    .await?;

    let (effective, rows) = payload;
    let mut runs = Vec::with_capacity(rows.len());
    let mut above_threshold = 0i64;
    let mut agreed = 0i64;
    let mut comparable = 0i64;
    for row in &rows {
        let mut answers: Vec<DecisionRunAnswer> = Vec::new();
        let mut run_above = false;
        let mut top: Option<f64> = None;
        if let Some(map) = row.answers.as_object() {
            for (question_id, entry) in map {
                let Some(entry) = entry.as_object() else { continue };
                let kind = entry
                    .get("kind")
                    .and_then(|k| k.as_str())
                    .unwrap_or_default()
                    .to_string();
                let label = entry
                    .get("picked_label")
                    .and_then(|l| l.as_str())
                    .unwrap_or_default()
                    .to_string();
                let confidence = entry.get("confidence").and_then(|c| c.as_f64());
                let threshold = effective
                    .questions
                    .iter()
                    .find(|q| q.id == *question_id)
                    .map(|q| q.thresholds.act);
                let question_above = match (confidence, threshold) {
                    (Some(c), Some(t)) => c >= t,
                    _ => false,
                };
                if let Some(c) = confidence {
                    top = Some(top.map_or(c, |prev: f64| prev.max(c)));
                }
                run_above |= question_above;
                answers.push(DecisionRunAnswer {
                    question_id: question_id.clone(),
                    kind,
                    label,
                    confidence,
                    above_threshold: question_above,
                });
            }
        }
        answers.sort_by(|a, b| a.question_id.cmp(&b.question_id));
        if run_above {
            above_threshold += 1;
        }
        // Agreement compares a proposed label against where the ticket
        // actually is. A ticket that has been deleted cannot be compared, and
        // saying so beats a silent zero.
        let comparison = row.current_status.as_ref().and_then(|current| {
            let needle = current.trim().to_lowercase();
            answers
                .iter()
                .any(|a| !a.label.is_empty() && a.label.trim().to_lowercase() == needle)
        });
        if row.current_status.is_some() {
            comparable += 1;
        }
        if comparison == Some(true) {
            agreed += 1;
        }
        runs.push(DecisionRunView {
            id: row.id,
            mode: row.mode.clone(),
            created_at: row.created_at,
            model: row.model.clone(),
            ticket_id: row.entity_id,
            ticket_number: row.ticket_number.clone(),
            ticket_title: row.ticket_title.clone(),
            answers,
            above_threshold: run_above,
            top_confidence: top,
            current_status: row.current_status.clone(),
            agreed: comparison,
        });
    }

    let threshold = effective
        .questions
        .iter()
        .map(|q| q.thresholds.act)
        .fold(0.0f64, f64::max);
    Ok(Json(DecisionRunsResponse {
        surface_key,
        days,
        summary: DecisionRunSummary {
            runs: rows.len() as i64,
            above_threshold,
            agreed,
            comparable,
            threshold,
            has_evidence: !rows.is_empty(),
        },
        runs,
    }))
}
