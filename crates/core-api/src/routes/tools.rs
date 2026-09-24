//! Tools checkout + toolboxes API (Wave 32 lane 3, `p32-tools-loans`;
//! migration 0104).
//!
//! Public surface on top of the lane-1 stock core:
//!   * tool kits (toolboxes): CRUD + member set + per-location readiness;
//!   * check-out/check-in transactions: a kit or a list of physical units
//!     leaves stock into ONE holder's hands, each unit riding its own
//!     append-only `stock_movements` leg (`checkout`/`checkin`);
//!   * the two named read models: `/stock-serials` (where is it now —
//!     holder or client, derived overdue, tools AND loaned devices) and
//!     `/tools/availability` (counts per tool item + location).
//!
//! Overdue is derived at read time, never stored. Mutations ride the
//! IF1-IF3 envelope (idempotency key, version gate, seq, audit trail) and
//! emit `tool.*` events plus the per-leg `stock.movement_recorded` events
//! (lane 1's contract) through the outbox in the same commit.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::routes::ApiGeneration;
use crate::state::AppState;
use axum::http::Extensions;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use core_domain::entities::{
    Actor, AuditEvent, ToolCheckout, ToolCheckoutLine, ToolKit, ToolKitMember,
};
use core_domain::policy::{Action, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

const KIT_ENTITY: &str = "tool_kit";
const CHECKOUT_ENTITY: &str = "tool_checkout";
const LIST_LIMIT: i64 = 500;

fn commercial(action: Action) -> Permission {
    Permission::Matrix {
        module: core_domain::policy::Module::Commercial,
        action,
    }
}

/// Maps the kit-name / member-uniqueness violation to a typed 409.
fn unique_conflict(e: core_db::DbError, what: &str) -> ApiError {
    let unique = matches!(
        &e,
        core_db::DbError::Sql(sqlx::Error::Database(db_err))
            if db_err.is_unique_violation()
    );
    if unique {
        ApiError::Conflict(what.to_string())
    } else {
        e.into()
    }
}

pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(
            &format!("{base}/tool-kits"),
            get(list_kits).post(create_kit),
        )
        .route(
            &format!("{base}/tool-kits/{{id}}"),
            get(get_kit).patch(update_kit),
        )
        .route(
            &format!("{base}/tool-kits/{{id}}/members"),
            post(add_kit_member),
        )
        .route(
            &format!("{base}/tool-kits/{{id}}/members/{{member_id}}"),
            delete(remove_kit_member),
        )
        .route(
            &format!("{base}/tool-kits/{{id}}/availability"),
            get(kit_availability),
        )
        .route(
            &format!("{base}/tool-checkouts"),
            get(list_checkouts).post(create_checkout),
        )
        .route(&format!("{base}/tool-checkouts/{{id}}"), get(get_checkout))
        .route(
            &format!("{base}/tool-checkouts/{{id}}/checkin"),
            post(checkin),
        )
        .route(&format!("{base}/stock-serials"), get(tool_serials))
        .route(
            &format!("{base}/tools/availability"),
            get(tool_availability),
        )
}

// --- kits ---------------------------------------------------------------------

/// POST /api/tool-kits.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateKitRequest {
    pub name: String,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// PATCH /api/tool-kits/{id} — absent fields keep their value.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateKitRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub code: Option<Option<String>>,
    #[serde(default)]
    pub note: Option<Option<String>>,
    #[serde(default)]
    pub active: Option<bool>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ToolKitView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub org_id: Uuid,
    pub name: String,
    #[schema(nullable = true)]
    pub code: Option<String>,
    #[schema(nullable = true)]
    pub note: Option<String>,
    pub active: bool,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub created_by: Option<Uuid>,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String)]
    pub updated_at: DateTime<Utc>,
}

impl From<ToolKit> for ToolKitView {
    fn from(k: ToolKit) -> Self {
        Self {
            id: k.id,
            org_id: k.org_id,
            name: k.name,
            code: k.code,
            note: k.note,
            active: k.active,
            created_by: k.created_by,
            created_at: k.created_at,
            updated_at: k.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct KitMemberView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub stock_item_id: Uuid,
    pub item_name: String,
    #[schema(nullable = true)]
    pub item_sku: Option<String>,
    pub qty_milli: i64,
    pub is_tool: bool,
    pub is_serialized: bool,
}

impl From<core_db::tools::KitMemberView> for KitMemberView {
    fn from(m: core_db::tools::KitMemberView) -> Self {
        Self {
            id: m.member.id,
            stock_item_id: m.member.stock_item_id,
            item_name: m.item_name,
            item_sku: m.item_sku,
            qty_milli: m.member.qty_milli,
            is_tool: m.item_is_tool,
            is_serialized: m.item_is_serialized,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ToolKitMutationResponse {
    #[serde(flatten)]
    pub kit: ToolKitView,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct KitDetailResponse {
    pub kit: ToolKitView,
    pub members: Vec<KitMemberView>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct KitMutationWithMembersResponse {
    pub kit: ToolKitView,
    pub members: Vec<KitMemberView>,
    pub mutation: instant::MutationMeta,
}

/// POST /api/tool-kits/{id}/members.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct AddKitMemberRequest {
    #[schema(value_type = uuid::Uuid)]
    pub stock_item_id: Uuid,
    /// Whole units of the member tool (defaults to one unit = 1000 milli).
    #[serde(default)]
    pub qty_milli: Option<i64>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct KitListQuery {
    /// `true` = active kits only; absent = all.
    #[serde(default)]
    pub active: Option<bool>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// GET /api/tool-kits/{id}/availability?location_id=
#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct KitAvailabilityQuery {
    #[serde(default)]
    pub location_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct KitAvailabilityMemberView {
    #[schema(value_type = uuid::Uuid)]
    pub stock_item_id: Uuid,
    pub item_name: String,
    pub required_units: i64,
    pub available_units: i64,
    pub ready: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct KitAvailabilityResponse {
    #[schema(value_type = uuid::Uuid)]
    pub kit_id: Uuid,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub location_id: Option<Uuid>,
    /// True when every member has enough in-stock units at the location.
    pub ready: bool,
    pub members: Vec<KitAvailabilityMemberView>,
}

/// GET /api/tool-kits.
#[utoipa::path(
    operation_id = "tool_kits_list",
    get,
    path = "/api/tool-kits",
    tag = "commercial",
    security(("bearer" = [])),
    params(KitListQuery),
    responses((status = 200, body = [ToolKitView]))
)]
pub async fn list_kits(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Query(q): axum::extract::Query<KitListQuery>,
) -> ApiResult<Json<Vec<ToolKitView>>> {
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
    let kits = core_db::tools::list_kits(
        &mut tx,
        authed.principal.user.org_id,
        q.active == Some(true),
        q.limit.unwrap_or(LIST_LIMIT).min(LIST_LIMIT),
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(kits.into_iter().map(ToolKitView::from).collect()))
}

/// POST /api/tool-kits — instant-first (IF1/IF2).
#[utoipa::path(
    operation_id = "tool_kits_create",
    post,
    path = "/api/tool-kits",
    tag = "commercial",
    security(("bearer" = [])),
    request_body = CreateKitRequest,
    responses(
        (status = 200, body = ToolKitMutationResponse),
        (status = 409, description = "kit name already exists")
    )
)]
pub async fn create_kit(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Json(req): axum::extract::Json<CreateKitRequest>,
) -> ApiResult<Json<ToolKitMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let kit = ToolKit {
        id: Uuid::now_v7(),
        org_id,
        name: req.name.clone(),
        code: req.code.clone(),
        note: req.note.clone(),
        active: true,
        created_by: Some(user_id),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    kit.validate().map_err(ApiError::from)?;

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: KIT_ENTITY,
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        core_db::tools::insert_kit(&mut ctx.tx, &kit)
            .await
            .map_err(|e| unique_conflict(e, "a kit with that name already exists"))?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: KIT_ENTITY,
                entity_id: kit.id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            core_domain::events::types::TOOL_KIT_CREATED,
            KIT_ENTITY,
            Some(kit.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ToolKitCreated { kit: kit.clone() })
                .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "tool.kit_created".into(),
                entity_type: Some(KIT_ENTITY.into()),
                entity_id: Some(kit.id.to_string()),
                detail: serde_json::json!({ "name": kit.name, "code": kit.code }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        let view = ToolKitView::from(kit.clone());
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: KIT_ENTITY,
                entity_id: kit.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&kit).unwrap_or(Value::Null),
                updated_at: kit.updated_at,
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ToolKitMutationResponse {
                kit: view,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// GET /api/tool-kits/{id}.
#[utoipa::path(
    operation_id = "tool_kits_get",
    get,
    path = "/api/tool-kits/{id}",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = KitDetailResponse),
        (status = 404, description = "not visible in scope")
    )
)]
pub async fn get_kit(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<KitDetailResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let kit = core_db::tools::get_kit(&mut tx, org_id, id).await?;
    let members = core_db::tools::list_kit_members(&mut tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(KitDetailResponse {
        kit: ToolKitView::from(kit),
        members: members.into_iter().map(KitMemberView::from).collect(),
    }))
}

/// PATCH /api/tool-kits/{id} — instant-first (IF1/IF2).
#[utoipa::path(
    operation_id = "tool_kits_update",
    patch,
    path = "/api/tool-kits/{id}",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = UpdateKitRequest,
    responses(
        (status = 200, body = ToolKitMutationResponse),
        (status = 404, description = "not visible in scope"),
        (status = 409, description = "version conflict")
    )
)]
pub async fn update_kit(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<UpdateKitRequest>,
) -> ApiResult<Json<ToolKitMutationResponse>> {
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
                entity: KIT_ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::tools::get_kit(&mut ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        let mut kit = before.clone();
        if let Some(name) = &req.name {
            kit.name = name.clone();
        }
        if let Some(code) = &req.code {
            kit.code = code.clone();
        }
        if let Some(note) = &req.note {
            kit.note = note.clone();
        }
        if let Some(active) = req.active {
            kit.active = active;
        }
        kit.updated_at = Utc::now();
        kit.validate().map_err(ApiError::from)?;
        core_db::tools::update_kit(&mut ctx.tx, &kit)
            .await
            .map_err(|e| unique_conflict(e, "a kit with that name already exists"))?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: KIT_ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            core_domain::events::types::TOOL_KIT_UPDATED,
            KIT_ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ToolKitUpdated { kit: kit.clone() })
                .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "tool.kit_updated".into(),
                entity_type: Some(KIT_ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "name": kit.name, "active": kit.active }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        let view = ToolKitView::from(kit.clone());
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: KIT_ENTITY,
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&kit).unwrap_or(Value::Null),
                updated_at: kit.updated_at,
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ToolKitMutationResponse {
                kit: view,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// POST /api/tool-kits/{id}/members — add a member tool to the kit.
#[utoipa::path(
    operation_id = "tool_kits_add_member",
    post,
    path = "/api/tool-kits/{id}/members",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = AddKitMemberRequest,
    responses(
        (status = 200, body = KitMutationWithMembersResponse),
        (status = 404, description = "kit or item not visible in scope"),
        (status = 409, description = "item is not a tool / already a member")
    )
)]
pub async fn add_kit_member(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<AddKitMemberRequest>,
) -> ApiResult<Json<KitMutationWithMembersResponse>> {
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
                entity: KIT_ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let kit = core_db::tools::get_kit(&mut ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        let item = core_db::stock::get_stock_item(&mut *ctx.tx, org_id, req.stock_item_id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        if !item.is_tool {
            return Err(ApiError::Conflict(format!(
                "{} is not flagged as a tool",
                item.name
            )));
        }
        if item.kind != "serialized" {
            return Err(ApiError::Conflict(format!(
                "{} is not serialized; kit members are tracked per physical unit",
                item.name
            )));
        }
        let member = ToolKitMember {
            id: Uuid::now_v7(),
            org_id,
            kit_id: id,
            stock_item_id: req.stock_item_id,
            qty_milli: req.qty_milli.unwrap_or(1000),
            created_at: Utc::now(),
        };
        member.validate().map_err(ApiError::from)?;
        core_db::tools::add_kit_member(&mut ctx.tx, &member)
            .await
            .map_err(|e| unique_conflict(e, "that tool is already a member of this kit"))?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: KIT_ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "tool.kit_member_added".into(),
                entity_type: Some(KIT_ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "stock_item_id": member.stock_item_id,
                    "qty_milli": member.qty_milli,
                }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        let view = ToolKitView::from(kit.clone());
        let members: Vec<KitMemberView> = core_db::tools::list_kit_members(&mut ctx.tx, org_id, id)
            .await?
            .into_iter()
            .map(KitMemberView::from)
            .collect();
        let payload = serde_json::json!({ "kit": &view, "members": &members });
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: KIT_ENTITY,
                entity_id: id,
                op: "member_add",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&member).unwrap_or(Value::Null),
                updated_at: kit.updated_at,
                key: key.as_deref(),
                payload: &payload,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(KitMutationWithMembersResponse {
                kit: view,
                members,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/tool-kits/{id}/members/{member_id}.
#[utoipa::path(
    operation_id = "tool_kits_remove_member",
    delete,
    path = "/api/tool-kits/{id}/members/{member_id}",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path), ("member_id" = Uuid, Path)),
    responses(
        (status = 200, body = KitDetailResponse),
        (status = 404, description = "kit or member not visible in scope")
    )
)]
pub async fn remove_kit_member(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path((id, member_id)): axum::extract::Path<(Uuid, Uuid)>,
) -> ApiResult<Json<KitDetailResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    let kit = core_db::tools::get_kit(&mut tx, org_id, id).await?;
    let removed_item = core_db::tools::remove_kit_member(&mut tx, org_id, id, member_id).await?;
    core_db::audit::insert(
        &mut *tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::Human(user_id),
            action: "tool.kit_member_removed".into(),
            entity_type: Some(KIT_ENTITY.into()),
            entity_id: Some(id.to_string()),
            detail: serde_json::json!({
                "member_id": member_id,
                "stock_item_id": removed_item,
            }),
            ip: None,
            occurred_at: Utc::now(),
        },
    )
    .await?;
    let members = core_db::tools::list_kit_members(&mut tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(KitDetailResponse {
        kit: ToolKitView::from(kit),
        members: members.into_iter().map(KitMemberView::from).collect(),
    }))
}

/// GET /api/tool-kits/{id}/availability — can the kit go out here?
#[utoipa::path(
    operation_id = "tool_kits_availability",
    get,
    path = "/api/tool-kits/{id}/availability",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path), KitAvailabilityQuery),
    responses(
        (status = 200, body = KitAvailabilityResponse),
        (status = 404, description = "not visible in scope")
    )
)]
pub async fn kit_availability(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<KitAvailabilityQuery>,
) -> ApiResult<Json<KitAvailabilityResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    core_db::tools::get_kit(&mut tx, org_id, id).await?;
    let rows = core_db::tools::kit_availability(&mut tx, org_id, id, q.location_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    let members: Vec<KitAvailabilityMemberView> = rows
        .into_iter()
        .map(|r| {
            let ready = r.is_ready();
            KitAvailabilityMemberView {
                stock_item_id: r.stock_item_id,
                item_name: r.item_name,
                required_units: r.required_units,
                available_units: r.available_units,
                ready,
            }
        })
        .collect();
    let ready = !members.is_empty() && members.iter().all(|m| m.ready);
    Ok(Json(KitAvailabilityResponse {
        kit_id: id,
        location_id: q.location_id,
        ready,
        members,
    }))
}

// --- checkouts ------------------------------------------------------------------

/// POST /api/tool-checkouts.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateCheckoutRequest {
    #[schema(value_type = uuid::Uuid)]
    pub holder_user_id: Uuid,
    /// Set for a whole-kit (toolbox) checkout; `items` must then be empty.
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub kit_id: Option<Uuid>,
    /// Individual tools; an absent serial auto-picks the oldest in-stock
    /// unit at the checkout location.
    #[serde(default)]
    pub items: Vec<CheckoutItemRequest>,
    #[schema(value_type = uuid::Uuid)]
    pub from_location_id: Uuid,
    /// When the tools are due back (RFC 3339).
    #[schema(value_type = String)]
    pub due_at: DateTime<Utc>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct CheckoutItemRequest {
    #[schema(value_type = uuid::Uuid)]
    pub stock_item_id: Uuid,
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub serial_id: Option<Uuid>,
}

/// POST /api/tool-checkouts/{id}/checkin.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CheckinRequest {
    /// Where the tools land; absent = the checkout's own location.
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub to_location_id: Option<Uuid>,
    /// Lines to return; empty/absent = every outstanding unit.
    #[serde(default)]
    pub lines: Vec<CheckinLineRequest>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct CheckinLineRequest {
    #[schema(value_type = uuid::Uuid)]
    pub line_id: Uuid,
    /// good | fair | damaged.
    #[serde(default)]
    pub return_condition: Option<String>,
    #[serde(default)]
    pub return_notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ToolCheckoutLineView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub stock_item_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub serial_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub movement_out_id: Uuid,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub movement_in_id: Option<Uuid>,
    #[schema(value_type = Option<String>)]
    pub returned_at: Option<DateTime<Utc>>,
    #[schema(nullable = true)]
    pub return_condition: Option<String>,
    #[schema(nullable = true)]
    pub return_notes: Option<String>,
}

impl From<ToolCheckoutLine> for ToolCheckoutLineView {
    fn from(l: ToolCheckoutLine) -> Self {
        Self {
            id: l.id,
            stock_item_id: l.stock_item_id,
            serial_id: l.serial_id,
            movement_out_id: l.movement_out_id,
            movement_in_id: l.movement_in_id,
            returned_at: l.returned_at,
            return_condition: l.return_condition,
            return_notes: l.return_notes,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ToolCheckoutView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub org_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub holder_user_id: Uuid,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub kit_id: Option<Uuid>,
    #[schema(value_type = uuid::Uuid)]
    pub from_location_id: Uuid,
    #[schema(value_type = String)]
    pub due_at: DateTime<Utc>,
    #[schema(nullable = true)]
    pub notes: Option<String>,
    pub status: String,
    #[schema(value_type = Option<String>)]
    pub returned_at: Option<DateTime<Utc>>,
    #[schema(nullable = true)]
    pub return_notes: Option<String>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub checked_out_by: Option<Uuid>,
    /// Derived: open past the due date.
    pub overdue: bool,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String)]
    pub updated_at: DateTime<Utc>,
}

impl ToolCheckoutView {
    fn view(c: ToolCheckout) -> Self {
        let overdue = c.is_overdue(Utc::now());
        Self {
            id: c.id,
            org_id: c.org_id,
            holder_user_id: c.holder_user_id,
            kit_id: c.kit_id,
            from_location_id: c.from_location_id,
            due_at: c.due_at,
            notes: c.notes,
            status: c.status,
            returned_at: c.returned_at,
            return_notes: c.return_notes,
            checked_out_by: c.checked_out_by,
            overdue,
            created_at: c.created_at,
            updated_at: c.updated_at,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ToolCheckoutMutationResponse {
    pub checkout: ToolCheckoutView,
    pub lines: Vec<ToolCheckoutLineView>,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ToolCheckoutDetailResponse {
    pub checkout: ToolCheckoutView,
    pub lines: Vec<ToolCheckoutLineView>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct CheckoutListQuery {
    /// open | returned (absent = both).
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub holder_user_id: Option<Uuid>,
    #[serde(default)]
    pub kit_id: Option<Uuid>,
    /// Only open checkouts past their due date.
    #[serde(default)]
    pub overdue_only: Option<bool>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// POST /api/tool-checkouts — instant-first (IF1/IF2).
#[utoipa::path(
    operation_id = "tool_checkouts_create",
    post,
    path = "/api/tool-checkouts",
    tag = "commercial",
    security(("bearer" = [])),
    request_body = CreateCheckoutRequest,
    responses(
        (status = 200, body = ToolCheckoutMutationResponse),
        (status = 409, description = "kit not ready / unit already out / not a tool"),
        (status = 404, description = "item, kit, or location not visible in scope")
    )
)]
pub async fn create_checkout(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Json(req): axum::extract::Json<CreateCheckoutRequest>,
) -> ApiResult<Json<ToolCheckoutMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let core_req = core_db::tools::CheckoutRequest {
        holder_user_id: req.holder_user_id,
        kit_id: req.kit_id,
        from_location_id: req.from_location_id,
        due_at: req.due_at,
        notes: req.notes.clone(),
        items: req
            .items
            .iter()
            .map(|i| core_db::tools::CheckoutTarget {
                stock_item_id: i.stock_item_id,
                serial_id: i.serial_id,
            })
            .collect(),
    };
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: CHECKOUT_ENTITY,
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let result = core_db::tools::create_checkout(&mut ctx.tx, org_id, user_id, &core_req)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        for leg in &result.legs {
            crate::routes::stock::emit_movement_side_effects(
                &mut ctx,
                org_id,
                &leg.movement,
                leg.low.as_ref(),
            )
            .await?;
        }
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: CHECKOUT_ENTITY,
                entity_id: result.checkout.id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            core_domain::events::types::TOOL_CHECKOUT_COMPLETED,
            CHECKOUT_ENTITY,
            Some(result.checkout.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ToolCheckoutCompleted {
                checkout: result.checkout.clone(),
                lines: result.lines.clone(),
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
                action: "tool.checkout".into(),
                entity_type: Some(CHECKOUT_ENTITY.into()),
                entity_id: Some(result.checkout.id.to_string()),
                detail: serde_json::json!({
                    "holder_user_id": result.checkout.holder_user_id,
                    "kit_id": result.checkout.kit_id,
                    "units": result.lines.len(),
                    "due_at": result.checkout.due_at,
                }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        let view = ToolCheckoutView::view(result.checkout.clone());
        let lines: Vec<ToolCheckoutLineView> = result
            .lines
            .into_iter()
            .map(ToolCheckoutLineView::from)
            .collect();
        let payload = serde_json::json!({ "checkout": &view, "lines": &lines });
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: CHECKOUT_ENTITY,
                entity_id: view.id,
                op: "checkout",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: view.updated_at,
                key: key.as_deref(),
                payload: &payload,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ToolCheckoutMutationResponse {
                checkout: view,
                lines,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// POST /api/tool-checkouts/{id}/checkin — instant-first (IF1/IF2).
#[utoipa::path(
    operation_id = "tool_checkouts_checkin",
    post,
    path = "/api/tool-checkouts/{id}/checkin",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = CheckinRequest,
    responses(
        (status = 200, body = ToolCheckoutMutationResponse),
        (status = 404, description = "not visible in scope"),
        (status = 409, description = "already returned / line not on this checkout")
    )
)]
pub async fn checkin(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<CheckinRequest>,
) -> ApiResult<Json<ToolCheckoutMutationResponse>> {
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
                entity: CHECKOUT_ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        // Default destination: where the tools came from.
        let (checkout_before, _lines) = core_db::tools::get_checkout(&mut ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        let to_location_id = req
            .to_location_id
            .unwrap_or(checkout_before.from_location_id);
        let targets: Vec<core_db::tools::CheckinTarget> = req
            .lines
            .iter()
            .map(|l| core_db::tools::CheckinTarget {
                line_id: l.line_id,
                return_condition: l.return_condition.clone(),
                return_notes: l.return_notes.clone(),
            })
            .collect();
        let result = core_db::tools::checkin(
            &mut ctx.tx,
            org_id,
            user_id,
            id,
            to_location_id,
            &targets,
            req.notes.as_deref(),
        )
        .await
        .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        for leg in &result.legs {
            crate::routes::stock::emit_movement_side_effects(
                &mut ctx,
                org_id,
                &leg.movement,
                leg.low.as_ref(),
            )
            .await?;
        }
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: CHECKOUT_ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            core_domain::events::types::TOOL_CHECKIN_COMPLETED,
            CHECKOUT_ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ToolCheckinCompleted {
                checkout: result.checkout.clone(),
                lines: result.lines.clone(),
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
                action: "tool.checkin".into(),
                entity_type: Some(CHECKOUT_ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "units_returned": result.legs.len(),
                    "to_location_id": to_location_id,
                    "status": result.checkout.status,
                }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        let view = ToolCheckoutView::view(result.checkout.clone());
        let lines: Vec<ToolCheckoutLineView> = result
            .lines
            .into_iter()
            .map(ToolCheckoutLineView::from)
            .collect();
        let payload = serde_json::json!({ "checkout": &view, "lines": &lines });
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: CHECKOUT_ENTITY,
                entity_id: id,
                op: "checkin",
                seq: write_seq,
                before: Some(serde_json::to_value(&checkout_before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: view.updated_at,
                key: key.as_deref(),
                payload: &payload,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ToolCheckoutMutationResponse {
                checkout: view,
                lines,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// GET /api/tool-checkouts.
#[utoipa::path(
    operation_id = "tool_checkouts_list",
    get,
    path = "/api/tool-checkouts",
    tag = "commercial",
    security(("bearer" = [])),
    params(CheckoutListQuery),
    responses((status = 200, body = [ToolCheckoutDetailResponse]))
)]
pub async fn list_checkouts(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Query(q): axum::extract::Query<CheckoutListQuery>,
) -> ApiResult<Json<Vec<ToolCheckoutDetailResponse>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    if let Some(status) = q.status.as_deref() {
        if status != "open" && status != "returned" {
            return Err(ApiError::Validation(
                "status must be open or returned".into(),
            ));
        }
    }
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let limit = q.limit.unwrap_or(LIST_LIMIT).min(LIST_LIMIT);
    let filter = core_db::tools::CheckoutFilter {
        status: q.status.as_deref(),
        holder_user_id: q.holder_user_id,
        kit_id: q.kit_id,
        overdue_only: q.overdue_only == Some(true),
        limit,
    };
    let checkouts = core_db::tools::list_checkouts(&mut tx, org_id, &filter).await?;
    let mut out = Vec::with_capacity(checkouts.len());
    for checkout in checkouts {
        let lines = core_db::tools::list_lines(&mut tx, org_id, checkout.id).await?;
        out.push(ToolCheckoutDetailResponse {
            checkout: ToolCheckoutView::view(checkout),
            lines: lines.into_iter().map(ToolCheckoutLineView::from).collect(),
        });
    }
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(out))
}

/// GET /api/tool-checkouts/{id}.
#[utoipa::path(
    operation_id = "tool_checkouts_get",
    get,
    path = "/api/tool-checkouts/{id}",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = ToolCheckoutDetailResponse),
        (status = 404, description = "not visible in scope")
    )
)]
pub async fn get_checkout(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<ToolCheckoutDetailResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let (checkout, lines) = core_db::tools::get_checkout(&mut tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(ToolCheckoutDetailResponse {
        checkout: ToolCheckoutView::view(checkout),
        lines: lines.into_iter().map(ToolCheckoutLineView::from).collect(),
    }))
}

// --- read models ------------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct ToolSerialsQuery {
    #[serde(default)]
    pub item_id: Option<Uuid>,
    #[serde(default)]
    pub location_id: Option<Uuid>,
    /// in_stock | out | checked_out | loaned.
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub holder_user_id: Option<Uuid>,
    /// `true` = tools only; `false` = devices only; absent = both.
    #[serde(default)]
    pub tool_only: Option<bool>,
    /// Only units past their checkout due date or loan expected return.
    #[serde(default)]
    pub overdue_only: Option<bool>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ToolSerialWhereaboutsView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub stock_item_id: Uuid,
    pub item_name: String,
    /// True when the item is a flagged tool (checkout-eligible).
    pub is_tool: bool,
    pub serial_number: String,
    pub status: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub location_id: Option<Uuid>,
    #[schema(nullable = true)]
    pub location_name: Option<String>,
    /// Open checkout holder, when the unit is checked out.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub holder_user_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub checkout_id: Option<Uuid>,
    #[schema(value_type = Option<String>)]
    pub due_at: Option<DateTime<Utc>>,
    /// Open loan client, when the unit is on a client loan.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub client_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub loan_id: Option<Uuid>,
    #[schema(value_type = Option<String>)]
    pub expected_return_at: Option<chrono::NaiveDate>,
    /// Derived: checked-out past due, or loaned past its expected return.
    pub overdue: bool,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String)]
    pub updated_at: DateTime<Utc>,
}

impl From<core_db::tools::ToolSerialWhereabouts> for ToolSerialWhereaboutsView {
    fn from(r: core_db::tools::ToolSerialWhereabouts) -> Self {
        let now = Utc::now();
        let overdue = r
            .due_at
            .map(|due| r.status == "checked_out" && due < now)
            .unwrap_or(false)
            || r.expected_return_at
                .map(|due| r.status == "loaned" && due < now.date_naive())
                .unwrap_or(false);
        Self {
            id: r.id,
            stock_item_id: r.stock_item_id,
            item_name: r.item_name,
            is_tool: r.is_tool,
            serial_number: r.serial_number,
            status: r.status,
            location_id: r.location_id,
            location_name: r.location_name,
            holder_user_id: r.holder_user_id,
            checkout_id: r.checkout_id,
            due_at: r.due_at,
            client_id: r.loan_client_id.or(r.serial_client_id),
            loan_id: r.loan_id,
            expected_return_at: r.expected_return_at,
            overdue,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

/// GET /api/stock-serials — WHERE IS IT NOW, per serialized unit (tools
/// and loaned devices; each row carries its live holder or client).
#[utoipa::path(
    operation_id = "stock_serials_whereabouts",
    get,
    path = "/api/stock-serials",
    tag = "commercial",
    security(("bearer" = [])),
    params(ToolSerialsQuery),
    responses((status = 200, body = [ToolSerialWhereaboutsView]))
)]
pub async fn tool_serials(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Query(q): axum::extract::Query<ToolSerialsQuery>,
) -> ApiResult<Json<Vec<ToolSerialWhereaboutsView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    if let Some(status) = q.status.as_deref() {
        if !["in_stock", "out", "checked_out", "loaned"].contains(&status) {
            return Err(ApiError::Validation(
                "status must be in_stock, out, checked_out, or loaned".into(),
            ));
        }
    }
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let filter = core_db::tools::WhereaboutsFilter {
        stock_item_id: q.item_id,
        location_id: q.location_id,
        status: q.status.as_deref(),
        holder_user_id: q.holder_user_id,
        is_tool: q.tool_only,
        overdue_only: q.overdue_only == Some(true),
        limit: q.limit.unwrap_or(LIST_LIMIT).min(LIST_LIMIT),
    };
    let rows = core_db::tools::serial_whereabouts(&mut tx, org_id, &filter).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(
        rows.into_iter()
            .map(ToolSerialWhereaboutsView::from)
            .collect(),
    ))
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct ToolAvailabilityQuery {
    #[serde(default)]
    pub location_id: Option<Uuid>,
    #[serde(default)]
    pub item_id: Option<Uuid>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ToolAvailabilityView {
    #[schema(value_type = uuid::Uuid)]
    pub stock_item_id: Uuid,
    pub item_name: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub location_id: Option<Uuid>,
    #[schema(nullable = true)]
    pub location_name: Option<String>,
    pub available_units: i64,
    pub checked_out_units: i64,
    pub loaned_units: i64,
    pub out_units: i64,
}

impl From<core_db::tools::ToolAvailabilityRow> for ToolAvailabilityView {
    fn from(r: core_db::tools::ToolAvailabilityRow) -> Self {
        Self {
            stock_item_id: r.stock_item_id,
            item_name: r.item_name,
            location_id: r.location_id,
            location_name: r.location_name,
            available_units: r.available_units,
            checked_out_units: r.checked_out_units,
            loaned_units: r.loaned_units,
            out_units: r.out_units,
        }
    }
}

/// GET /api/tools/availability — in-stock counts per tool item + location.
#[utoipa::path(
    operation_id = "tools_availability",
    get,
    path = "/api/tools/availability",
    tag = "commercial",
    security(("bearer" = [])),
    params(ToolAvailabilityQuery),
    responses((status = 200, body = [ToolAvailabilityView]))
)]
pub async fn tool_availability(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Query(q): axum::extract::Query<ToolAvailabilityQuery>,
) -> ApiResult<Json<Vec<ToolAvailabilityView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::tools::tool_availability(
        &mut tx,
        org_id,
        q.location_id,
        q.item_id,
        q.limit.unwrap_or(LIST_LIMIT).min(LIST_LIMIT),
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(
        rows.into_iter().map(ToolAvailabilityView::from).collect(),
    ))
}
