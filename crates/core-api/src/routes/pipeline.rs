//! Sales pipeline (the private build tracker Phase 3, Wave 12 lane 2; migration 0060):
//! stage CRUD, deal (opportunity) CRUD, and audited stage transitions on
//! the IF1–IF3 instant-first envelope.
//!
//! Stages are org-scoped ordered rows with a kind (`open` | `won` |
//! `lost`); deals link one client and OPTIONALLY a quote and sit on
//! exactly one stage. Moving a deal appends an IMMUTABLE transition row
//! (from → to, actor, note, time) in the same transaction — the ledger is
//! the audit trail; `pipeline.deal_stage_changed` is the consumer signal.
//! Landing on a won/lost stage closes the deal (`status` + `closed_at`).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::routes::ApiGeneration;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    routing::{get, patch, post},
    Json, Router,
};
use core_domain::entities::{
    Actor, AuditEvent, DealStatus, PipelineDeal, PipelineDealTransition, PipelineStage, StageType,
};
use core_domain::events::{
    types, PipelineDealCreated, PipelineDealStageChanged, PipelineDealUpdated,
    PipelineStageCreated, PipelineStageUpdated,
};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

fn commercial(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Commercial,
        action,
    }
}

const STAGE_ENTITY: &str = "pipeline_stage";
const DEAL_ENTITY: &str = "pipeline_deal";
const LIST_LIMIT: i64 = 200;

pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(
            &format!("{base}/pipeline/stages"),
            get(list_stages).post(create_stage),
        )
        .route(
            &format!("{base}/pipeline/stages/{{id}}"),
            patch(update_stage),
        )
        .route(
            &format!("{base}/pipeline/deals"),
            get(list_deals).post(create_deal),
        )
        .route(
            &format!("{base}/pipeline/deals/{{id}}"),
            get(get_one).patch(update_deal),
        )
        .route(
            &format!("{base}/pipeline/deals/{{id}}/stage"),
            post(move_stage),
        )
}

// --- wire structs ----------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct StageListQuery {
    /// `None` = active stages only (default); `true` includes inactive.
    pub include_inactive: Option<bool>,
}

/// POST /api/pipeline/stages body.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateStageRequest {
    pub name: String,
    pub position: Option<i32>,
    /// `open|won|lost` (default `open`).
    pub stage_type: Option<StageType>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// PATCH /api/pipeline/stages/{id} body.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateStageRequest {
    pub name: Option<String>,
    pub position: Option<i32>,
    pub stage_type: Option<StageType>,
    pub active: Option<bool>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// GET /api/pipeline/deals query.
#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct DealListQuery {
    pub client_id: Option<Uuid>,
    pub stage_id: Option<Uuid>,
    pub quote_id: Option<Uuid>,
    /// Exact status: `open|won|lost`.
    pub status: Option<String>,
    pub owner_user_id: Option<Uuid>,
}

/// POST /api/pipeline/deals body.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateDealRequest {
    pub client_id: Uuid,
    /// Optional originating quote (same client).
    pub quote_id: Option<Uuid>,
    pub stage_id: Uuid,
    pub name: String,
    #[serde(default)]
    pub amount_minor: i64,
    pub currency: String,
    pub expected_close_on: Option<chrono::NaiveDate>,
    pub owner_user_id: Option<Uuid>,
    pub notes: Option<String>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// PATCH /api/pipeline/deals/{id} body — editable fields only; the stage
/// moves via the /stage leg.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateDealRequest {
    pub client_id: Option<Uuid>,
    pub quote_id: Option<Option<Uuid>>,
    pub name: Option<String>,
    pub amount_minor: Option<i64>,
    pub currency: Option<String>,
    pub expected_close_on: Option<Option<chrono::NaiveDate>>,
    pub owner_user_id: Option<Option<Uuid>>,
    pub notes: Option<Option<String>>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// POST /api/pipeline/deals/{id}/stage body — one audited transition.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct MoveStageRequest {
    pub to_stage_id: Uuid,
    pub note: Option<String>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// Instant-first mutation responses (IF2).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct StageMutationResponse {
    #[serde(flatten)]
    pub stage: PipelineStage,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DealMutationResponse {
    #[serde(flatten)]
    pub deal: PipelineDeal,
    pub mutation: instant::MutationMeta,
}

/// The deal aggregate: document + transition ledger.
#[derive(Debug, Serialize, ToSchema)]
pub struct DealDetail {
    #[serde(flatten)]
    pub deal: PipelineDeal,
    pub transitions: Vec<PipelineDealTransition>,
}

// --- reads ------------------------------------------------------------------

/// GET /api/pipeline/stages — ordered pipeline definition.
#[utoipa::path(
    operation_id = "pipeline_stages_list",
    get,
    path = "/api/pipeline/stages",
    tag = "commercial",
    security(("bearer" = [])),
    params(StageListQuery),
    responses((status = 200, body = [PipelineStage]))
)]
pub async fn list_stages(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<StageListQuery>,
) -> ApiResult<Json<Vec<PipelineStage>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let stages = core_db::pipeline::list_stages(
        &mut *tx,
        authed.principal.user.org_id,
        q.include_inactive.unwrap_or(false),
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(stages))
}

/// GET /api/pipeline/deals — newest first, filterable.
#[utoipa::path(
    operation_id = "pipeline_deals_list",
    get,
    path = "/api/pipeline/deals",
    tag = "commercial",
    security(("bearer" = [])),
    params(DealListQuery),
    responses((status = 200, body = [PipelineDeal]))
)]
pub async fn list_deals(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<DealListQuery>,
) -> ApiResult<Json<Vec<PipelineDeal>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let filters = core_db::pipeline::DealFilters {
        client_id: q.client_id,
        stage_id: q.stage_id,
        quote_id: q.quote_id,
        status: q.status,
        owner_user_id: q.owner_user_id,
    };
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let deals =
        core_db::pipeline::list_deals(&mut *tx, authed.principal.user.org_id, &filters, LIST_LIMIT)
            .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(deals))
}

/// GET /api/pipeline/deals/{id} — the deal plus its transition ledger.
#[utoipa::path(
    operation_id = "pipeline_deals_get",
    get,
    path = "/api/pipeline/deals/{id}",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = DealDetail),
        (status = 404, description = "not visible in scope")
    )
)]
pub async fn get_deal(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<DealDetail>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let deal = core_db::pipeline::get_deal(&mut *tx, authed.principal.user.org_id, id).await?;
    let transitions =
        core_db::pipeline::list_transitions(&mut *tx, authed.principal.user.org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(DealDetail { deal, transitions }))
}

// --- stages -----------------------------------------------------------------

/// POST /api/pipeline/stages — create a stage (name unique per org).
#[utoipa::path(
    operation_id = "pipeline_stages_create",
    post,
    path = "/api/pipeline/stages",
    tag = "commercial",
    security(("bearer" = [])),
    request_body = CreateStageRequest,
    responses(
        (status = 200, body = StageMutationResponse),
        (status = 409, description = "stage name already exists or idempotency key reused"),
        (status = 422, description = "validation failure")
    )
)]
pub async fn create_stage(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateStageRequest>,
) -> ApiResult<Json<StageMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let stage_type = req.stage_type.unwrap_or(StageType::Open);
    let stage = PipelineStage {
        id: Uuid::now_v7(),
        org_id,
        name: req.name.clone(),
        position: req.position.unwrap_or_default(),
        stage_type,
        active: true,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    stage.validate().map_err(ApiError::from)?;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: STAGE_ENTITY,
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        core_db::pipeline::insert_stage(&mut *ctx.tx, &stage).await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: STAGE_ENTITY,
                entity_id: stage.id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            types::PIPELINE_STAGE_CREATED,
            STAGE_ENTITY,
            Some(stage.id.to_string().as_str()),
            &serde_json::to_value(PipelineStageCreated {
                stage: stage.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "pipeline.stage_created".into(),
                entity_type: Some(STAGE_ENTITY.into()),
                entity_id: Some(stage.id.to_string()),
                detail: serde_json::json!({
                    "name": stage.name,
                    "position": stage.position,
                    "stage_type": stage.stage_type.as_str(),
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: STAGE_ENTITY,
                entity_id: stage.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&stage).unwrap_or(Value::Null),
                updated_at: stage.updated_at,
                key: key.as_deref(),
                payload: &stage,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(StageMutationResponse { stage, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/pipeline/stages/{id}.
#[utoipa::path(
    operation_id = "pipeline_stages_update",
    patch,
    path = "/api/pipeline/stages/{id}",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = UpdateStageRequest,
    responses(
        (status = 200, body = StageMutationResponse),
        (status = 404, description = "stage not visible in scope"),
        (status = 409, description = "stage name already exists, version conflict, or idempotency key reused"),
        (status = 422, description = "validation failure")
    )
)]
pub async fn update_stage(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateStageRequest>,
) -> ApiResult<Json<StageMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: STAGE_ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::pipeline::get_stage(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        let mut stage = before.clone();
        if let Some(name) = &req.name {
            stage.name = name.clone();
        }
        if let Some(position) = req.position {
            stage.position = position;
        }
        if let Some(stage_type) = req.stage_type {
            stage.stage_type = stage_type;
        }
        if let Some(active) = req.active {
            stage.active = active;
        }
        stage.validate().map_err(ApiError::from)?;
        let after = core_db::pipeline::update_stage(&mut *ctx.tx, &stage)
            .await
            .map_err(map_unique_conflict)?
            .ok_or(ApiError::NotFound)?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: STAGE_ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            types::PIPELINE_STAGE_UPDATED,
            STAGE_ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(PipelineStageUpdated {
                stage: after.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "pipeline.stage_updated".into(),
                entity_type: Some(STAGE_ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "name": after.name,
                    "position": after.position,
                    "stage_type": after.stage_type.as_str(),
                    "active": after.active,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: STAGE_ENTITY,
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&after).unwrap_or(Value::Null),
                updated_at: after.updated_at,
                key: key.as_deref(),
                payload: &after,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(StageMutationResponse {
                stage: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- deals ------------------------------------------------------------------

/// GET /api/pipeline/deals/{id} — deal + transition ledger.
/// (Registered at the `get_one` name in the router.)
#[utoipa::path(
    operation_id = "pipeline_deals_get_one",
    get,
    path = "/api/pipeline/deals/{id}",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = DealDetail),
        (status = 404, description = "not visible in scope")
    )
)]
pub async fn get_one(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<DealDetail>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let deal = core_db::pipeline::get_deal(&mut *tx, authed.principal.user.org_id, id).await?;
    let transitions =
        core_db::pipeline::list_transitions(&mut *tx, authed.principal.user.org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(DealDetail { deal, transitions }))
}

/// POST /api/pipeline/deals — create an opportunity and record its
/// initial placement in the transition ledger (same transaction).
#[utoipa::path(
    operation_id = "pipeline_deals_create",
    post,
    path = "/api/pipeline/deals",
    tag = "commercial",
    security(("bearer" = [])),
    request_body = CreateDealRequest,
    responses(
        (status = 200, body = DealMutationResponse),
        (status = 404, description = "client, quote, or stage not visible in scope"),
        (status = 409, description = "idempotency key reused"),
        (status = 422, description = "validation failure")
    )
)]
pub async fn create_deal(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateDealRequest>,
) -> ApiResult<Json<DealMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: DEAL_ENTITY,
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        // Scope checks: client, stage, optional quote.
        core_db::clients::get_client(&mut *ctx.tx, ctx.org_id, req.client_id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        let stage = core_db::pipeline::get_stage(&mut *ctx.tx, ctx.org_id, req.stage_id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        if let Some(quote_id) = req.quote_id {
            let quote = core_db::quotes::get_quote(&mut *ctx.tx, ctx.org_id, quote_id)
                .await
                .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
            if quote.client_id != req.client_id {
                return Err(ApiError::Validation(
                    "the linked quote belongs to a different client".into(),
                ));
            }
        }
        let now = chrono::Utc::now();
        let status = DealStatus::for_stage(stage.stage_type);
        let deal = PipelineDeal {
            id: Uuid::now_v7(),
            org_id: ctx.org_id,
            client_id: req.client_id,
            quote_id: req.quote_id,
            stage_id: stage.id,
            name: req.name.clone(),
            amount_minor: req.amount_minor,
            currency: req.currency.clone(),
            status,
            expected_close_on: req.expected_close_on,
            owner_user_id: req.owner_user_id,
            notes: req.notes.clone(),
            closed_at: if stage.stage_type.is_terminal() {
                Some(now)
            } else {
                None
            },
            created_by: Some(user_id),
            created_at: now,
            updated_at: now,
        };
        deal.validate().map_err(ApiError::from)?;
        core_db::pipeline::insert_deal(&mut *ctx.tx, &deal).await?;
        // Initial placement rides the ledger (NULL from_stage).
        core_db::pipeline::insert_transition(
            &mut *ctx.tx,
            &PipelineDealTransition {
                id: Uuid::now_v7(),
                org_id: ctx.org_id,
                deal_id: deal.id,
                from_stage_id: None,
                to_stage_id: stage.id,
                note: Some("deal created".into()),
                actor_id: Some(user_id),
                created_at: now,
            },
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: DEAL_ENTITY,
                entity_id: deal.id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            types::PIPELINE_DEAL_CREATED,
            DEAL_ENTITY,
            Some(deal.id.to_string().as_str()),
            &serde_json::to_value(PipelineDealCreated { deal: deal.clone() })
                .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "pipeline.deal_created".into(),
                entity_type: Some(DEAL_ENTITY.into()),
                entity_id: Some(deal.id.to_string()),
                detail: serde_json::json!({
                    "name": deal.name,
                    "client_id": deal.client_id,
                    "stage_id": deal.stage_id,
                    "amount_minor": deal.amount_minor,
                }),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: DEAL_ENTITY,
                entity_id: deal.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&deal).unwrap_or(Value::Null),
                updated_at: deal.updated_at,
                key: key.as_deref(),
                payload: &deal,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(DealMutationResponse { deal, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/pipeline/deals/{id} — editable fields; the stage is not
/// touched here (that is the audited /stage leg).
#[utoipa::path(
    operation_id = "pipeline_deals_update",
    patch,
    path = "/api/pipeline/deals/{id}",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = UpdateDealRequest,
    responses(
        (status = 200, body = DealMutationResponse),
        (status = 404, description = "deal, client, or quote not visible in scope"),
        (status = 409, description = "version conflict or idempotency key reused"),
        (status = 422, description = "validation failure")
    )
)]
pub async fn update_deal(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateDealRequest>,
) -> ApiResult<Json<DealMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: DEAL_ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::pipeline::get_deal(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        let mut deal = before.clone();
        if let Some(client_id) = req.client_id {
            deal.client_id = client_id;
        }
        if let Some(quote_id) = req.quote_id {
            deal.quote_id = quote_id;
        }
        if let Some(name) = &req.name {
            deal.name = name.clone();
        }
        if let Some(amount_minor) = req.amount_minor {
            deal.amount_minor = amount_minor;
        }
        if let Some(currency) = &req.currency {
            deal.currency = currency.clone();
        }
        if let Some(expected_close_on) = req.expected_close_on {
            deal.expected_close_on = expected_close_on;
        }
        if let Some(owner_user_id) = req.owner_user_id {
            deal.owner_user_id = owner_user_id;
        }
        if let Some(notes) = req.notes {
            deal.notes = notes;
        }
        deal.validate().map_err(ApiError::from)?;
        // The optional quote must exist in scope and match the client.
        if let Some(quote_id) = deal.quote_id {
            let quote = core_db::quotes::get_quote(&mut *ctx.tx, org_id, quote_id)
                .await
                .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
            if quote.client_id != deal.client_id {
                return Err(ApiError::Validation(
                    "the linked quote belongs to a different client".into(),
                ));
            }
        }
        let after = core_db::pipeline::update_deal(&mut *ctx.tx, &deal)
            .await?
            .ok_or(ApiError::NotFound)?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: DEAL_ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            types::PIPELINE_DEAL_UPDATED,
            DEAL_ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(PipelineDealUpdated {
                deal: after.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "pipeline.deal_updated".into(),
                entity_type: Some(DEAL_ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "name": after.name,
                    "amount_minor": after.amount_minor,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: DEAL_ENTITY,
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&after).unwrap_or(Value::Null),
                updated_at: after.updated_at,
                key: key.as_deref(),
                payload: &after,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(DealMutationResponse {
                deal: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/pipeline/deals/{id}/stage — one audited stage transition.
/// Guarded on the CURRENT stage (concurrent moves: first wins, loser gets
/// a typed 409); the ledger row commits in the same transaction.
#[utoipa::path(
    operation_id = "pipeline_deals_move_stage",
    post,
    path = "/api/pipeline/deals/{id}/stage",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = MoveStageRequest,
    responses(
        (status = 200, body = DealMutationResponse),
        (status = 404, description = "deal or target stage not visible in scope"),
        (status = 409, description = "concurrent move or version conflict"),
        (status = 422, description = "validation failure")
    )
)]
pub async fn move_stage(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<MoveStageRequest>,
) -> ApiResult<Json<DealMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "pipeline_deal_stage",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::pipeline::get_deal(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        let target = core_db::pipeline::get_stage(&mut *ctx.tx, org_id, req.to_stage_id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        if !target.active && target.id != before.stage_id {
            return Err(ApiError::Validation(
                "cannot move a deal onto an inactive stage".into(),
            ));
        }
        let status = DealStatus::for_stage(target.stage_type);
        let after = core_db::pipeline::move_deal_stage(
            &mut *ctx.tx,
            org_id,
            id,
            before.stage_id,
            target.id,
            status.as_str(),
        )
        .await?
        .ok_or_else(|| ApiError::Conflict("deal moved concurrently; reload and retry".into()))?;
        // The immutable ledger row (audit trail for the transition).
        core_db::pipeline::insert_transition(
            &mut *ctx.tx,
            &PipelineDealTransition {
                id: Uuid::now_v7(),
                org_id,
                deal_id: id,
                from_stage_id: Some(before.stage_id),
                to_stage_id: target.id,
                note: req.note.clone(),
                actor_id: Some(user_id),
                created_at: chrono::Utc::now(),
            },
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: DEAL_ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            types::PIPELINE_DEAL_STAGE_CHANGED,
            DEAL_ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(PipelineDealStageChanged {
                deal_id: id,
                name: after.name.clone(),
                from_stage_id: Some(before.stage_id),
                to_stage_id: target.id,
                to_stage_type: target.stage_type,
                status: after.status,
                amount_minor: after.amount_minor,
                currency: after.currency.clone(),
                actor_id: Some(user_id),
                occurred_at: chrono::Utc::now(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "pipeline.deal_stage_changed".into(),
                entity_type: Some(DEAL_ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "name": after.name,
                    "from_stage_id": before.stage_id,
                    "to_stage_id": target.id,
                    "to_stage_type": target.stage_type.as_str(),
                    "note": req.note,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: DEAL_ENTITY,
                entity_id: id,
                op: "stage_change",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&after).unwrap_or(Value::Null),
                updated_at: after.updated_at,
                key: key.as_deref(),
                payload: &after,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(DealMutationResponse {
                deal: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

fn map_unique_conflict(e: core_db::DbError) -> ApiError {
    match e {
        core_db::DbError::Sql(ref sql_err) if core_db::catalog::is_unique_conflict(sql_err) => {
            ApiError::Conflict("a pipeline stage with this name already exists".into())
        }
        other => other.into(),
    }
}
