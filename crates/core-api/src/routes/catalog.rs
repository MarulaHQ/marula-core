//! Product/service catalog (TASKLIST Phase 3, Wave 12 lane 2; migration
//! 0060): org-scoped catalog item CRUD on the IF1–IF3 instant-first
//! envelope.
//!
//! Items carry name, optional SKU (org-unique, case-insensitive), unit of
//! measure, unit price, cost, taxable flag, category/group, and an active
//! flag. Archival is a flag, never a delete — quote lines snapshot their
//! catalog context at write time, so history survives. Every mutation
//! emits its contract event + audit row in the same commit.
//!
//! Non-purpose: no pricing tiers, no per-client price books, no FX
//! (single currency per item; ADR 0002 — Wave 12 lane 10 owns FX).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::routes::ApiGeneration;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    routing::{get, post},
    Json, Router,
};
use core_domain::entities::{Actor, AuditEvent, CatalogItem};
use core_domain::events::{types, CatalogItemArchived, CatalogItemCreated, CatalogItemUpdated};
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

const ENTITY: &str = "catalog_item";

/// Upper bound on list size (parity with the rest of the crate's lists).
const LIST_LIMIT: i64 = 500;

pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(&format!("{base}/catalog-items"), get(list).post(create))
        .route(
            &format!("{base}/catalog-items/{{id}}"),
            get(get_one).patch(update),
        )
        .route(
            &format!("{base}/catalog-items/{{id}}/archive"),
            post(archive),
        )
}

// --- wire structs ----------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct CatalogListQuery {
    /// `None` = all items (active + archived).
    pub active: Option<bool>,
    pub category: Option<String>,
    pub group_name: Option<String>,
    /// Substring search over name/sku/category/group.
    pub search: Option<String>,
}

/// POST /api/catalog-items body.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateCatalogItemRequest {
    pub name: String,
    pub sku: Option<String>,
    pub description: Option<String>,
    /// Unit of measure the price applies to (default `each`).
    #[serde(default = "default_unit")]
    pub unit: String,
    pub currency: String,
    #[serde(default)]
    pub unit_price_minor: i64,
    #[serde(default)]
    pub cost_minor: i64,
    #[serde(default = "default_true")]
    pub taxable: bool,
    pub category: Option<String>,
    pub group_name: Option<String>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

fn default_unit() -> String {
    "each".into()
}

fn default_true() -> bool {
    true
}

/// PATCH /api/catalog-items/{id} body. Absent fields keep their value;
/// `Some(None)` clears an optional field.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateCatalogItemRequest {
    pub name: Option<String>,
    /// Some → set (null clears); absent → no change.
    #[serde(default)]
    pub sku: Option<Option<String>>,
    #[serde(default)]
    pub description: Option<Option<String>>,
    pub unit: Option<String>,
    pub currency: Option<String>,
    pub unit_price_minor: Option<i64>,
    pub cost_minor: Option<i64>,
    pub taxable: Option<bool>,
    #[serde(default)]
    pub category: Option<Option<String>>,
    #[serde(default)]
    pub group_name: Option<Option<String>>,
    /// Reactivating an archived item is a plain update.
    pub active: Option<bool>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ArchiveCatalogItemRequest {
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// Instant-first mutation response (IF2) for catalog mutations.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CatalogItemMutationResponse {
    #[serde(flatten)]
    pub item: CatalogItemView,
    pub mutation: instant::MutationMeta,
}

/// RB2 (ADR 0004): a catalog item as the CALLER may see it. `unit_price`
/// is the commercial surface; `cost_minor` (and the gross margin derived
/// from it) is financial data — it serializes `null` for callers without
/// `financial:read` (dispatchers and technicians by default), which is
/// the "those values render blank" rule. The write trail keeps full
/// fidelity server-side; only client-facing views redact.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CatalogItemView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub org_id: Uuid,
    pub name: String,
    #[schema(nullable = true)]
    pub sku: Option<String>,
    #[schema(nullable = true)]
    pub description: Option<String>,
    pub unit: String,
    pub currency: String,
    pub unit_price_minor: i64,
    /// `null` without `financial:read` (RB2, ADR 0004).
    #[schema(nullable = true)]
    pub cost_minor: Option<i64>,
    pub taxable: bool,
    #[schema(nullable = true)]
    pub category: Option<String>,
    #[schema(nullable = true)]
    pub group_name: Option<String>,
    pub active: bool,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub created_by: Option<Uuid>,
    #[schema(value_type = String)]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[schema(value_type = String)]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl CatalogItemView {
    fn view(item: CatalogItem, financial: bool) -> Self {
        Self {
            id: item.id,
            org_id: item.org_id,
            name: item.name,
            sku: item.sku,
            description: item.description,
            unit: item.unit,
            currency: item.currency,
            unit_price_minor: item.unit_price_minor,
            cost_minor: financial.then_some(item.cost_minor),
            taxable: item.taxable,
            category: item.category,
            group_name: item.group_name,
            active: item.active,
            created_by: item.created_by,
            created_at: item.created_at,
            updated_at: item.updated_at,
        }
    }
}

// --- handlers ---------------------------------------------------------------

/// GET /api/catalog-items — newest/active first, filterable.
#[utoipa::path(
    operation_id = "catalog_items_list",
    get,
    path = "/api/catalog-items",
    tag = "commercial",
    security(("bearer" = [])),
    params(CatalogListQuery),
    responses((status = 200, body = [CatalogItemView]))
)]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<CatalogListQuery>,
) -> ApiResult<Json<Vec<CatalogItemView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let filters = core_db::catalog::CatalogItemFilters {
        active: q.active,
        category: q.category,
        group_name: q.group_name,
        search: q.search,
    };
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let financial =
        core_domain::policy::check(&authed.principal.permissions, Permission::ViewFinancials)
            .is_allowed();
    let items = core_db::catalog::list_catalog_items(
        &mut *tx,
        authed.principal.user.org_id,
        &filters,
        LIST_LIMIT,
    )
    .await?
    .into_iter()
    .map(|item| CatalogItemView::view(item, financial))
    .collect();
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(items))
}

/// GET /api/catalog-items/{id}.
#[utoipa::path(
    operation_id = "catalog_items_get",
    get,
    path = "/api/catalog-items/{id}",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = CatalogItemView),
        (status = 404, description = "not visible in scope")
    )
)]
pub async fn get_one(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<CatalogItemView>> {
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
    let financial =
        core_domain::policy::check(&authed.principal.permissions, Permission::ViewFinancials)
            .is_allowed();
    let item = core_db::catalog::get_catalog_item(&mut *tx, authed.principal.user.org_id, id)
        .await
        .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(CatalogItemView::view(item, financial)))
}

/// POST /api/catalog-items — create through the instant-first write path.
#[utoipa::path(
    operation_id = "catalog_items_create",
    post,
    path = "/api/catalog-items",
    tag = "commercial",
    security(("bearer" = [])),
    request_body = CreateCatalogItemRequest,
    responses(
        (status = 200, body = CatalogItemMutationResponse),
        (status = 409, description = "SKU already exists or idempotency key reused"),
        (status = 422, description = "validation failure")
    )
)]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateCatalogItemRequest>,
) -> ApiResult<Json<CatalogItemMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let item = CatalogItem {
        id: Uuid::now_v7(),
        org_id,
        name: req.name.clone(),
        sku: req.sku.clone(),
        description: req.description.clone(),
        unit: req.unit.clone(),
        currency: req.currency.clone(),
        unit_price_minor: req.unit_price_minor,
        cost_minor: req.cost_minor,
        taxable: req.taxable,
        category: req.category.clone(),
        group_name: req.group_name.clone(),
        active: true,
        created_by: Some(user_id),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    item.validate().map_err(ApiError::from)?;
    let financial =
        core_domain::policy::check(&authed.principal.permissions, Permission::ViewFinancials)
            .is_allowed();
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
        core_db::catalog::insert_catalog_item(&mut *ctx.tx, &item)
            .await
            .map_err(map_unique_conflict)?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: item.id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            types::CATALOG_ITEM_CREATED,
            ENTITY,
            Some(item.id.to_string().as_str()),
            &serde_json::to_value(CatalogItemCreated { item: item.clone() })
                .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "catalog.item_created".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(item.id.to_string()),
                detail: serde_json::json!({
                    "name": item.name,
                    "sku": item.sku,
                    "currency": item.currency,
                    "unit_price_minor": item.unit_price_minor,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: item.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&item).unwrap_or(Value::Null),
                updated_at: item.updated_at,
                key: key.as_deref(),
                payload: &item,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(CatalogItemMutationResponse {
                item: CatalogItemView::view(item, financial),
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/catalog-items/{id} — absent fields keep their value.
#[utoipa::path(
    operation_id = "catalog_items_update",
    patch,
    path = "/api/catalog-items/{id}",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = UpdateCatalogItemRequest,
    responses(
        (status = 200, body = CatalogItemMutationResponse),
        (status = 404, description = "not visible in scope"),
        (status = 409, description = "SKU already exists, version conflict, or idempotency key reused"),
        (status = 422, description = "validation failure")
    )
)]
pub async fn update(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateCatalogItemRequest>,
) -> ApiResult<Json<CatalogItemMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let financial =
        core_domain::policy::check(&authed.principal.permissions, Permission::ViewFinancials)
            .is_allowed();
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
        let before = core_db::catalog::get_catalog_item(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        let mut item = before.clone();
        if let Some(name) = &req.name {
            item.name = name.clone();
        }
        if let Some(sku) = &req.sku {
            item.sku = sku.clone();
        }
        if let Some(description) = &req.description {
            item.description = description.clone();
        }
        if let Some(unit) = &req.unit {
            item.unit = unit.clone();
        }
        if let Some(currency) = &req.currency {
            item.currency = currency.clone();
        }
        if let Some(unit_price_minor) = req.unit_price_minor {
            item.unit_price_minor = unit_price_minor;
        }
        if let Some(cost_minor) = req.cost_minor {
            item.cost_minor = cost_minor;
        }
        if let Some(taxable) = req.taxable {
            item.taxable = taxable;
        }
        if let Some(category) = &req.category {
            item.category = category.clone();
        }
        if let Some(group_name) = &req.group_name {
            item.group_name = group_name.clone();
        }
        if let Some(active) = req.active {
            item.active = active;
        }
        item.validate().map_err(ApiError::from)?;
        let after = core_db::catalog::update_catalog_item(&mut *ctx.tx, &item)
            .await
            .map_err(map_unique_conflict)?
            .ok_or(ApiError::NotFound)?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            types::CATALOG_ITEM_UPDATED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(CatalogItemUpdated {
                item: after.clone(),
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
                action: "catalog.item_updated".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "name": after.name,
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
                entity: ENTITY,
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
            MutationOutcome::Applied(CatalogItemMutationResponse {
                item: CatalogItemView::view(after, financial),
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/catalog-items/{id}/archive — flip `active` off (data, never
/// a delete). Already-archived items archive as a no-op replay of state.
#[utoipa::path(
    operation_id = "catalog_items_archive",
    post,
    path = "/api/catalog-items/{id}/archive",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = ArchiveCatalogItemRequest,
    responses(
        (status = 200, body = CatalogItemMutationResponse),
        (status = 404, description = "not visible in scope"),
        (status = 409, description = "version conflict or idempotency key reused")
    )
)]
pub async fn archive(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<ArchiveCatalogItemRequest>,
) -> ApiResult<Json<CatalogItemMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let financial =
        core_domain::policy::check(&authed.principal.permissions, Permission::ViewFinancials)
            .is_allowed();
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
        let before = core_db::catalog::get_catalog_item(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        let after = core_db::catalog::archive_catalog_item(&mut *ctx.tx, org_id, id)
            .await?
            .unwrap_or(before.clone());
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            types::CATALOG_ITEM_ARCHIVED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(CatalogItemArchived {
                item_id: id,
                name: after.name.clone(),
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
                action: "catalog.item_archived".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "name": after.name }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: id,
                op: "archive",
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
            MutationOutcome::Applied(CatalogItemMutationResponse {
                item: CatalogItemView::view(after, financial),
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// Maps the org-unique SKU/name/number violations to typed 409s.
pub(crate) fn map_unique_conflict(e: core_db::DbError) -> ApiError {
    match e {
        core_db::DbError::Sql(ref sql_err) if core_db::catalog::is_unique_conflict(sql_err) => {
            ApiError::Conflict("an item with this SKU already exists".into())
        }
        other => other.into(),
    }
}
