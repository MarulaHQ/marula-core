//! Stock + consumables API (Wave 32 lane 1, `p32-stock-consumables`).
//!
//! The IF1–IF3 instant-first envelope over `stock_items` /
//! `stock_locations` / `stock_levels` / `stock_serials`, the append-only
//! `stock_movements` ledger, and the two named operational read models:
//!
//!   * `GET /api/stock/on-hand` — stock on hand (level rows + thresholds,
//!     the fresh instant-first read).
//!   * `GET /api/stock/consumption-velocity` — per-item outbound rate
//!     over a lookback window, computed by the core from its ledger.
//!
//! Mutations are the ledger legs: receive / consume / adjust / transfer /
//! ship-out. Every mutation appends exactly ONE ledger row and applies
//! its deterministic level/serial consequence in the same commit, emits
//! `stock.movement_recorded` (+ `stock.level_low` when the transition
//! into low happened) through the transactional outbox, and writes its
//! audit row. Low-stock alerts ride the notification center (`stock_low`
//! kind rows, same commit).
//!
//! ADR 0002 inventory-receipt FX lock: a `receive` whose cost currency
//! ≠ org base freezes the FX snapshot on the ledger row at receipt time
//! (routes/fx.rs `lock_snapshot`); display conversions happen only at
//! read time; this layer never re-derives money after the lock.
//!
//! Serialized vs consumable: `kind = serialized` items move per physical
//! unit (receive upserts one `stock_serials` row per serial number, one
//! `qty_milli=1000` ledger row each; OUT legs require the serial and flip
//! its status). That shape is the seam Wave 32 lane 3 (tools + client
//! loans) consumes.
//!
//! Non-purpose: NO purchase orders (a later lane's entities; no PO
//! coupling here), NO UI (lane 4), no config-items coupling (a stock item
//! may later instantiate a DeviceAsset when installed — link, don't
//! merge).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::routes::ApiGeneration;
use crate::state::AppState;
use axum::http::Extensions;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use core_domain::entities::{Actor, AuditEvent, StockItem, StockLocation};
use core_domain::policy::{check, Action, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

const ENTITY: &str = "stock_item";
const LIST_LIMIT: i64 = 500;

fn commercial(action: Action) -> Permission {
    Permission::Matrix {
        module: core_domain::policy::Module::Commercial,
        action,
    }
}

fn financial_read(authed: &crate::auth::AuthedUser) -> bool {
    check(&authed.principal.permissions, Permission::ViewFinancials).is_allowed()
}

/// Maps the DB SKU/name-unique violation to a typed 409.
fn sku_conflict(e: core_db::DbError) -> ApiError {
    let unique = matches!(
        &e,
        core_db::DbError::Sql(sqlx::Error::Database(db_err))
            if db_err.is_unique_violation()
    );
    if unique {
        ApiError::Conflict("that name/SKU already exists".into())
    } else {
        e.into()
    }
}

pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(
            &format!("{base}/stock-items"),
            get(list_items).post(create_item),
        )
        .route(
            &format!("{base}/stock-items/{{id}}"),
            get(get_item).patch(update_item),
        )
        .route(
            &format!("{base}/stock-items/{{id}}/archive"),
            post(archive_item),
        )
        .route(
            &format!("{base}/stock-items/{{id}}/movements"),
            get(item_movements),
        )
        .route(
            &format!("{base}/stock-items/{{id}}/serials"),
            get(item_serials),
        )
        .route(
            &format!("{base}/stock-locations"),
            get(list_locations).post(create_location),
        )
        .route(
            &format!("{base}/stock-locations/{{id}}"),
            get(get_location).patch(update_location),
        )
        .route(
            &format!("{base}/stock/levels/thresholds"),
            post(set_thresholds_route),
        )
        .route(&format!("{base}/stock/receive"), post(receive))
        .route(&format!("{base}/stock/consume"), post(consume))
        .route(&format!("{base}/stock/adjust"), post(adjust))
        .route(&format!("{base}/stock/transfer"), post(transfer))
        .route(&format!("{base}/stock/ship-out"), post(ship_out))
        .route(&format!("{base}/stock/on-hand"), get(on_hand))
        .route(
            &format!("{base}/stock/consumption-velocity"),
            get(consumption_velocity_route),
        )
}

// --- wire structs -----------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct StockItemListQuery {
    /// `None` = all items (active + archived).
    pub active: Option<bool>,
    pub search: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// POST /api/stock-items.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateStockItemRequest {
    pub name: String,
    pub sku: Option<String>,
    /// Soft catalog link when a catalog line exists; absent = free-form
    /// part (name/part_number carry the identity).
    #[schema(value_type = Option<uuid::Uuid>)]
    pub catalog_item_id: Option<Uuid>,
    pub part_number: Option<String>,
    #[serde(default = "default_consumable")]
    pub kind: String,
    #[serde(default = "default_each")]
    pub unit: String,
    pub currency: String,
    /// Optional opening cost reference at create (display only; locked
    /// costs land with receive legs).
    #[serde(default)]
    pub last_unit_cost_minor: Option<i64>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

fn default_consumable() -> String {
    "consumable".into()
}

fn default_each() -> String {
    "each".into()
}

/// PATCH /api/stock-items/{id}. Absent fields keep their value;
/// `Some(None)` clears an optional field.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateStockItemRequest {
    pub name: Option<String>,
    #[serde(default)]
    pub sku: Option<Option<String>>,
    #[serde(default)]
    pub catalog_item_id: Option<Uuid>,
    #[serde(default)]
    pub part_number: Option<Option<String>>,
    pub kind: Option<String>,
    pub unit: Option<String>,
    /// Reactivating an archived item is a plain update.
    pub active: Option<bool>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ArchiveStockItemRequest {
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// Instant-first mutation response (IF2).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct StockItemMutationResponse {
    #[serde(flatten)]
    pub item: StockItemView,
    pub mutation: instant::MutationMeta,
}

/// The stock item as the CALLER may see it. `last_unit_cost_minor` is
/// financial data — `null` without `financial:read` (RB2, ADR 0004).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StockItemView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub org_id: Uuid,
    pub name: String,
    #[schema(nullable = true)]
    pub sku: Option<String>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub catalog_item_id: Option<Uuid>,
    #[schema(nullable = true)]
    pub part_number: Option<String>,
    pub kind: String,
    pub unit: String,
    pub currency: String,
    /// `null` without `financial:read` (RB2).
    #[schema(nullable = true)]
    pub last_unit_cost_minor: Option<i64>,
    pub active: bool,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub created_by: Option<Uuid>,
    #[schema(value_type = String)]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[schema(value_type = String)]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl StockItemView {
    fn view(item: StockItem, financial: bool) -> Self {
        Self {
            id: item.id,
            org_id: item.org_id,
            name: item.name,
            sku: item.sku,
            catalog_item_id: item.catalog_item_id,
            part_number: item.part_number,
            kind: item.kind,
            unit: item.unit,
            currency: item.currency,
            last_unit_cost_minor: financial.then_some(item.last_unit_cost_minor).flatten(),
            active: item.active,
            created_by: item.created_by,
            created_at: item.created_at,
            updated_at: item.updated_at,
        }
    }
}

/// POST /api/stock-locations.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateStockLocationRequest {
    pub name: String,
    pub kind: String,
    pub code: Option<String>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub parent_id: Option<Uuid>,
    /// Whose van/truck (optional).
    #[schema(value_type = Option<uuid::Uuid>)]
    pub assigned_user_id: Option<Uuid>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateStockLocationRequest {
    pub name: Option<String>,
    pub kind: Option<String>,
    #[serde(default)]
    pub code: Option<Option<String>>,
    #[serde(default)]
    pub assigned_user_id: Option<Uuid>,
    pub active: Option<bool>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct LocationListQuery {
    pub kind: Option<String>,
    #[serde(default)]
    pub active_only: Option<bool>,
}

/// Location view.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct StockLocationView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub org_id: Uuid,
    pub name: String,
    pub kind: String,
    #[schema(nullable = true)]
    pub code: Option<String>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub parent_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub assigned_user_id: Option<Uuid>,
    pub active: bool,
    #[schema(value_type = String)]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[schema(value_type = String)]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<StockLocation> for StockLocationView {
    fn from(l: StockLocation) -> Self {
        Self {
            id: l.id,
            org_id: l.org_id,
            name: l.name,
            kind: l.kind,
            code: l.code,
            parent_id: l.parent_id,
            assigned_user_id: l.assigned_user_id,
            active: l.active,
            created_at: l.created_at,
            updated_at: l.updated_at,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct StockLocationMutationResponse {
    #[serde(flatten)]
    pub location: StockLocationView,
    pub mutation: instant::MutationMeta,
}

/// POST /api/stock/{receive|consume|adjust|transfer|ship-out} bodies.
/// All quantities are `milli` thousandths of the item's unit (parity with
/// quote lines); signed legs are normalized per kind below.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ReceiveRequest {
    #[schema(value_type = uuid::Uuid)]
    pub item_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub location_id: Uuid,
    /// MILLI of the item's unit; serialized items move 1000/serial.
    pub qty_milli: i64,
    /// Unit cost in the item's cost currency (locked at receipt on the
    /// ledger row; ADR 0002 inventory-receipt lock moment).
    #[serde(default)]
    pub unit_cost_minor: Option<i64>,
    /// Serialized receives carry one entry per physical unit.
    pub serial_numbers: Option<Vec<String>>,
    /// Optional receiving note.
    pub note: Option<String>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ConsumeRequest {
    #[schema(value_type = uuid::Uuid)]
    pub item_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub location_id: Uuid,
    /// MILLI to consume (positive; the ledger stores it negative).
    pub qty_milli: i64,
    /// The on-site job consuming the parts (traceability anchor).
    #[schema(value_type = Option<uuid::Uuid>)]
    pub ticket_id: Option<Uuid>,
    /// Serialized consumption moves ONE unit.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub serial_id: Option<Uuid>,
    pub note: Option<String>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct AdjustRequest {
    #[schema(value_type = uuid::Uuid)]
    pub item_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub location_id: Uuid,
    /// SIGNED milli delta (negative = shrink). The counter adjustment is
    /// the audit trail, not a rewrite.
    pub qty_milli: i64,
    pub note: Option<String>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct TransferRequest {
    #[schema(value_type = uuid::Uuid)]
    pub item_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub from_location_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub to_location_id: Uuid,
    pub qty_milli: i64,
    /// Serialized transfer moves ONE unit.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub serial_id: Option<Uuid>,
    pub note: Option<String>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ShipOutRequest {
    #[schema(value_type = uuid::Uuid)]
    pub item_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub location_id: Uuid,
    pub qty_milli: i64,
    /// Ship-to client (fleet/loan delivery anchor).
    #[schema(value_type = Option<uuid::Uuid>)]
    pub client_id: Option<Uuid>,
    /// The on-site job the shipment belongs to.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub ticket_id: Option<Uuid>,
    /// Optional carrier + tracking ref (deeplinked by later lanes).
    pub carrier: Option<String>,
    pub tracking_ref: Option<String>,
    /// Serialized shipment moves ONE unit.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub serial_id: Option<Uuid>,
    pub note: Option<String>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// Movement view (the traceability row the reads render).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StockMovementView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub stock_item_id: Uuid,
    pub kind: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub from_location_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub to_location_id: Option<Uuid>,
    pub qty_milli: i64,
    /// `null` without `financial:read` (RB2).
    #[schema(nullable = true)]
    pub cost_minor: Option<i64>,
    #[schema(value_type = Option<String>)]
    pub currency: Option<String>,
    /// ADR 0002 receipt FX lock fields (`null` for in-base movements).
    #[schema(value_type = Option<String>)]
    pub fx_rate_used: Option<String>,
    #[schema(value_type = Option<String>)]
    pub fx_rate_date: Option<String>,
    #[schema(value_type = Option<String>)]
    pub fx_source: Option<String>,
    #[schema(nullable = true)]
    pub base_cost_minor: Option<i64>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub serial_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub client_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub ticket_id: Option<Uuid>,
    #[schema(nullable = true)]
    pub carrier: Option<String>,
    #[schema(nullable = true)]
    pub tracking_ref: Option<String>,
    #[schema(nullable = true)]
    pub note: Option<String>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub created_by: Option<Uuid>,
    #[schema(value_type = String)]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

fn mv_view(m: core_domain::entities::StockMovement, financial: bool) -> StockMovementView {
    StockMovementView {
        id: m.id,
        stock_item_id: m.stock_item_id,
        kind: m.kind,
        from_location_id: m.from_location_id,
        to_location_id: m.to_location_id,
        qty_milli: m.qty_milli,
        cost_minor: financial.then_some(m.cost_minor).flatten(),
        currency: None,
        fx_rate_used: m.fx_rate_used,
        fx_rate_date: m.fx_rate_date.map(|d| d.to_string()),
        fx_source: m.fx_source,
        base_cost_minor: financial.then_some(m.base_cost_minor).flatten(),
        serial_id: m.serial_id,
        client_id: m.client_id,
        ticket_id: m.ticket_id,
        carrier: m.carrier,
        tracking_ref: m.tracking_ref,
        note: m.note,
        created_by: m.created_by,
        created_at: m.created_at,
    }
}

/// Instant-first mutation response for ledger legs.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MovementMutationResponse {
    pub movement: StockMovementView,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ThresholdsRequest {
    #[schema(value_type = uuid::Uuid)]
    pub item_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub location_id: Uuid,
    #[schema(value_type = Option<i64>)]
    pub min_qty_milli: Option<i64>,
    #[schema(value_type = Option<i64>)]
    pub max_qty_milli: Option<i64>,
    #[schema(value_type = Option<i64>)]
    pub reorder_point_milli: Option<i64>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// The stock-on-hand read model row (`qty leaking thresholds`).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OnHandRow {
    #[schema(value_type = uuid::Uuid)]
    pub stock_item_id: Uuid,
    pub item_name: String,
    pub item_kind: String,
    pub unit: String,
    #[schema(value_type = uuid::Uuid)]
    pub stock_location_id: Uuid,
    pub location_name: String,
    pub location_kind: String,
    pub qty_milli: i64,
    #[schema(value_type = Option<i64>)]
    pub min_qty_milli: Option<i64>,
    #[schema(value_type = Option<i64>)]
    pub max_qty_milli: Option<i64>,
    #[schema(value_type = Option<i64>)]
    pub reorder_point_milli: Option<i64>,
    /// The instant-first low flag the UI + automations key on.
    pub low: bool,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct OnHandQuery {
    #[serde(default)]
    pub item_id: Option<Uuid>,
    #[serde(default)]
    pub location_id: Option<Uuid>,
    /// true → only rows at/below the reorder point.
    #[serde(default)]
    pub low_only: Option<bool>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct VelocityQuery {
    /// Lookback window in days (default 30; outbound legs summed).
    #[serde(default)]
    pub window_days: Option<i64>,
    #[serde(default)]
    pub item_id: Option<Uuid>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct VelocityMutationless {
    #[schema(value_type = uuid::Uuid)]
    pub stock_item_id: Uuid,
    pub item_name: String,
    pub unit: String,
    pub consumed_milli: i64,
    /// Average daily outbound rate in milli over the window.
    pub avg_daily_milli: i64,
}

// --- locations ---------------------------------------------------------------

/// GET /api/stock-locations.
#[utoipa::path(
    operation_id = "stock_locations_list",
    get,
    path = "/api/stock-locations",
    tag = "commercial",
    security(("bearer" = [])),
    params(LocationListQuery),
    responses((status = 200, body = [StockLocationView]))
)]
pub async fn list_locations(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Query(q): axum::extract::Query<LocationListQuery>,
) -> ApiResult<Json<Vec<StockLocationView>>> {
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
    let locations = core_db::stock::list_stock_locations(
        &mut tx,
        authed.principal.user.org_id,
        q.kind.as_deref(),
        q.active_only.unwrap_or(false),
        LIST_LIMIT,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(
        locations.into_iter().map(StockLocationView::from).collect(),
    ))
}

/// GET /api/stock-locations/{id}.
#[utoipa::path(
    operation_id = "stock_locations_get",
    get,
    path = "/api/stock-locations/{id}",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = StockLocationView),
        (status = 404, description = "not visible in scope")
    )
)]
pub async fn get_location(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<StockLocationView>> {
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
    let location =
        core_db::stock::get_stock_location(&mut *tx, authed.principal.user.org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(StockLocationView::from(location)))
}

/// POST /api/stock-locations.
#[utoipa::path(
    operation_id = "stock_locations_create",
    post,
    path = "/api/stock-locations",
    tag = "commercial",
    security(("bearer" = [])),
    request_body = CreateStockLocationRequest,
    responses(
        (status = 200, body = StockLocationMutationResponse),
        (status = 409, description = "idempotency key reused or name conflicts"),
        (status = 422, description = "validation failure")
    )
)]
pub async fn create_location(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Json(req): axum::extract::Json<CreateStockLocationRequest>,
) -> ApiResult<Json<StockLocationMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    const L_ENTITY: &str = "stock_location";
    let location = StockLocation {
        id: Uuid::now_v7(),
        org_id,
        name: req.name.clone(),
        kind: req.kind.clone(),
        code: req.code.clone(),
        parent_id: req.parent_id,
        assigned_user_id: req.assigned_user_id,
        active: true,
        created_by: Some(user_id),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    location.validate().map_err(ApiError::from)?;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: L_ENTITY,
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        core_db::stock::insert_stock_location(&mut *ctx.tx, &location)
            .await
            .map_err(sku_conflict)?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: L_ENTITY,
                entity_id: location.id,
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
                action: "stock.location_created".into(),
                entity_type: Some(L_ENTITY.into()),
                entity_id: Some(location.id.to_string()),
                detail: serde_json::json!({ "name": location.name, "kind": location.kind }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: L_ENTITY,
                entity_id: location.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&location).unwrap_or(Value::Null),
                updated_at: location.updated_at,
                key: key.as_deref(),
                payload: &location,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(StockLocationMutationResponse {
                location: StockLocationView::from(location),
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/stock-locations/{id}.
#[utoipa::path(
    operation_id = "stock_locations_update",
    patch,
    path = "/api/stock-locations/{id}",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = UpdateStockLocationRequest,
    responses(
        (status = 200, body = StockLocationMutationResponse),
        (status = 404, description = "not visible in scope"),
        (status = 422, description = "validation failure")
    )
)]
pub async fn update_location(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<UpdateStockLocationRequest>,
) -> ApiResult<Json<StockLocationMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    const L_ENTITY: &str = "stock_location";
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: L_ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::stock::get_stock_location(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        let mut location = before.clone();
        if let Some(name) = &req.name {
            location.name = name.clone();
        }
        if let Some(kind) = &req.kind {
            location.kind = kind.clone();
        }
        if let Some(code) = &req.code {
            location.code = code.clone();
        }
        if let Some(assigned_user_id) = req.assigned_user_id {
            location.assigned_user_id = Some(assigned_user_id);
        }
        if let Some(active) = req.active {
            location.active = active;
        }
        location.updated_at = Utc::now();
        location.validate().map_err(ApiError::from)?;
        core_db::stock::update_stock_location(&mut *ctx.tx, &location).await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: L_ENTITY,
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
                action: "stock.location_updated".into(),
                entity_type: Some(L_ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "name": location.name, "active": location.active }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: L_ENTITY,
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&location).unwrap_or(Value::Null),
                updated_at: location.updated_at,
                key: key.as_deref(),
                payload: &location,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(StockLocationMutationResponse {
                location: StockLocationView::from(location),
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- items / locations handlers --------------------------------------------

/// GET /api/stock-items.
#[utoipa::path(
    operation_id = "stock_items_list",
    get,
    path = "/api/stock-items",
    tag = "commercial",
    security(("bearer" = [])),
    params(StockItemListQuery),
    responses((status = 200, body = [StockItemView]))
)]
pub async fn list_items(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Query(q): axum::extract::Query<StockItemListQuery>,
) -> ApiResult<Json<Vec<StockItemView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let only_active = q.active == Some(true);
    let limit = q.limit.unwrap_or(LIST_LIMIT).min(LIST_LIMIT);

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;

    let items = core_db::stock::list_stock_items(
        &mut tx,
        authed.principal.user.org_id,
        q.search.as_deref(),
        only_active,
        limit,
    )
    .await?;
    let financial = financial_read(&authed);
    let views: Vec<StockItemView> = items
        .into_iter()
        .map(|item| StockItemView::view(item, financial))
        .collect();
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(views))
}

/// GET /api/stock-items/{id}.
#[utoipa::path(
    operation_id = "stock_items_get",
    get,
    path = "/api/stock-items/{id}",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = StockItemView),
        (status = 404, description = "not visible in scope")
    )
)]
pub async fn get_item(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<StockItemView>> {
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
    let item = core_db::stock::get_stock_item(&mut *tx, authed.principal.user.org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(StockItemView::view(item, financial_read(&authed))))
}

/// POST /api/stock-items — create through the instant-first write path.
#[utoipa::path(
    operation_id = "stock_items_create",
    post,
    path = "/api/stock-items",
    tag = "commercial",
    security(("bearer" = [])),
    request_body = CreateStockItemRequest,
    responses(
        (status = 200, body = StockItemMutationResponse),
        (status = 409, description = "SKU already exists or idempotency key reused"),
        (status = 422, description = "validation failure")
    )
)]
pub async fn create_item(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Json(req): axum::extract::Json<CreateStockItemRequest>,
) -> ApiResult<Json<StockItemMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let financial = financial_read(&authed);
    let item = StockItem {
        id: Uuid::now_v7(),
        org_id,
        name: req.name.clone(),
        sku: req.sku.clone(),
        catalog_item_id: req.catalog_item_id,
        part_number: req.part_number.clone(),
        kind: req.kind.clone(),
        unit: req.unit.clone(),
        currency: req.currency.clone(),
        last_unit_cost_minor: None,
        active: true,
        created_by: Some(user_id),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    item.validate().map_err(ApiError::from)?;

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
        core_db::stock::insert_stock_item(&mut *ctx.tx, &item)
            .await
            .map_err(sku_conflict)?;
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
            core_domain::events::types::STOCK_ITEM_CREATED,
            ENTITY,
            Some(item.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::StockItemCreated { item: item.clone() })
                .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "stock.item_created".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(item.id.to_string()),
                detail: serde_json::json!({
                    "name": item.name,
                    "sku": item.sku,
                    "kind": item.kind,
                    "unit": item.unit,
                }),
                ip: None,
                occurred_at: Utc::now(),
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
            MutationOutcome::Applied(StockItemMutationResponse {
                item: StockItemView::view(item, financial),
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/stock-items/{id} — absent fields keep their value.
#[utoipa::path(
    operation_id = "stock_items_update",
    patch,
    path = "/api/stock-items/{id}",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = UpdateStockItemRequest,
    responses(
        (status = 200, body = StockItemMutationResponse),
        (status = 404, description = "not visible in scope"),
        (status = 409, description = "SKU already exists, version conflict, or idempotency key reused"),
        (status = 422, description = "validation failure")
    )
)]
pub async fn update_item(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<UpdateStockItemRequest>,
) -> ApiResult<Json<StockItemMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let financial = financial_read(&authed);
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
        let before = core_db::stock::get_stock_item(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        let mut item = before.clone();
        if let Some(name) = &req.name {
            item.name = name.clone();
        }
        if let Some(sku) = &req.sku {
            item.sku = sku.clone();
        }
        if let Some(catalog_item_id) = req.catalog_item_id {
            item.catalog_item_id = Some(catalog_item_id);
        }
        if let Some(part_number) = &req.part_number {
            item.part_number = part_number.clone();
        }
        if let Some(kind) = &req.kind {
            item.kind = kind.clone();
        }
        if let Some(unit) = &req.unit {
            item.unit = unit.clone();
        }
        if let Some(active) = req.active {
            item.active = active;
        }
        item.updated_at = Utc::now();
        item.validate().map_err(ApiError::from)?;
        core_db::stock::update_stock_item(&mut *ctx.tx, &item)
            .await
            .map_err(sku_conflict)?;
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
            core_domain::events::types::STOCK_ITEM_UPDATED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::StockItemUpdated { item: item.clone() })
                .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "stock.item_updated".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "name": item.name,
                    "active": item.active,
                }),
                ip: None,
                occurred_at: Utc::now(),
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
            MutationOutcome::Applied(StockItemMutationResponse {
                item: StockItemView::view(item, financial),
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/stock-items/{id}/archive — archival is a flag.
#[utoipa::path(
    operation_id = "stock_items_archive",
    post,
    path = "/api/stock-items/{id}/archive",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = ArchiveStockItemRequest,
    responses(
        (status = 200, body = StockItemMutationResponse),
        (status = 404, description = "not visible in scope")
    )
)]
pub async fn archive_item(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<ArchiveStockItemRequest>,
) -> ApiResult<Json<StockItemMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let financial = financial_read(&authed);
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
        let before = core_db::stock::get_stock_item(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        core_db::stock::archive_stock_item(&mut *ctx.tx, org_id, id).await?;
        let mut after = before.clone();
        after.active = false;
        after.updated_at = Utc::now();
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
            core_domain::events::types::STOCK_ITEM_ARCHIVED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::StockItemArchived {
                item_id: id,
                name: before.name.clone(),
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
                action: "stock.item_archived".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "name": before.name }),
                ip: None,
                occurred_at: Utc::now(),
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
            MutationOutcome::Applied(StockItemMutationResponse {
                item: StockItemView::view(after, financial),
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- the ledger legs -----------------------------------------------------------

/// Shared leg body: claim → item load → ledger append + consequence →
/// low-stock notification + events + audit → instant record. One commit
/// per call; the traceability row is written exactly once.
async fn ledger_apply(
    state: AppState,
    extensions: Extensions,
    op: &'static str,
    fingerprint: Value,
    key: Option<String>,
    m: core_domain::entities::StockMovement,
    serial_number: Option<String>,
) -> ApiResult<Json<MovementMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let financial = financial_read(&authed);

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "stock_movement",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }

        let item = core_db::stock::get_stock_item(&mut *ctx.tx, org_id, m.stock_item_id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        if !item.active {
            return Err(ApiError::Conflict(format!(
                "stock item {} is archived",
                m.stock_item_id
            )));
        }
        let mut movement = m;
        movement.currency = Some(item.currency.clone());
        movement.created_by = Some(user_id);
        movement.created_at = Utc::now();

        // ADR 0002 inventory-receipt FX lock: only a RECEIVE whose cost
        // currency differs from the org base freezes the snapshot; the
        // ledger row carries it forever after (read-time conversion only).
        if movement.kind == "receive" {
            if let Some(cost) = movement.cost_minor {
                let today = movement.created_at.date_naive();
                let currency = item.currency.clone();
                movement.currency = Some(currency.clone());
                let locked =
                    crate::routes::fx::lock_snapshot(&mut ctx.tx, org_id, cost, &currency, today)
                        .await?;
                if let Some(snap) = locked {
                    movement.fx_rate_used = Some(snap.rate_used);
                    movement.fx_rate_date = Some(snap.rate_date);
                    movement.fx_source = Some(snap.source);
                    movement.base_cost_minor = Some(snap.base_amount_minor);
                }
            }
        }

        let input = core_db::stock::LedgerInput {
            movement: movement.clone(),
            item_name: item.name.clone(),
            item_kind: item.kind.clone(),
            serial_number,
        };
        let ledger_out = core_db::stock::record_movement(&mut ctx.tx, &input).await?;
        let movement = ledger_out.movement;

        if let Some(low) = ledger_out.low {
            let recipients =
                core_db::stock::low_stock_recipients(&mut ctx.tx, org_id, low.stock_location_id)
                    .await?;
            for recipient in recipients {
                core_db::notifications::insert(
                    &mut *ctx.tx,
                    Uuid::now_v7(),
                    org_id,
                    recipient,
                    core_domain::entities::NotificationKind::StockLow.as_str(),
                    movement.ticket_id,
                    &format!("Low stock: {}", low.item_name),
                    Some(
                        format!(
                            "The level at {} fell to {} (reorder point {}).",
                            low.stock_location_id, low.qty_milli, low.reorder_point_milli
                        )
                        .as_str(),
                    ),
                )
                .await?;
            }
            ctx.emit(
                core_domain::events::types::STOCK_LEVEL_LOW,
                "stock_level",
                Some(low.stock_item_id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::StockLevelLow {
                    org_id,
                    stock_item_id: low.stock_item_id,
                    stock_location_id: low.stock_location_id,
                    qty_milli: low.qty_milli,
                    reorder_point_milli: low.reorder_point_milli,
                    item_name: low.item_name.clone(),
                    ticket_id: movement.ticket_id,
                    occurred_at: movement.created_at,
                })
                .unwrap_or_else(|_| serde_json::json!({})),
            )
            .await?;
        }

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "stock_movement",
                entity_id: movement.id,
                version: None,
            },
        )
        .await?;

        ctx.emit(
            core_domain::events::types::STOCK_MOVEMENT_RECORDED,
            "stock_movement",
            Some(movement.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::StockMovementRecorded {
                movement: movement.clone(),
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
                action: format!("stock.{op}"),
                entity_type: Some("stock_movement".into()),
                entity_id: Some(movement.id.to_string()),
                detail: serde_json::json!({
                    "item_id": movement.stock_item_id,
                    "kind": movement.kind,
                    "qty_milli": movement.qty_milli,
                }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;

        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "stock_movement",
                entity_id: movement.id,
                op,
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&movement).unwrap_or(Value::Null),
                updated_at: movement.created_at,
                key: key.as_deref(),
                payload: &movement,
                provenance: Default::default(),
            },
        )
        .await?;

        Ok((
            ctx,
            MutationOutcome::Applied(MovementMutationResponse {
                movement: mv_view(movement, financial),
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

async fn item_kind(
    state: &AppState,
    extensions: &Extensions,
    item_id: Uuid,
) -> ApiResult<(String, String)> {
    let authed = authed::<crate::auth::AuthedUser>(extensions)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    let item = core_db::stock::get_stock_item(&mut *tx, org_id, item_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok((item.kind, item.currency))
}

fn require_pos(qty: i64, what: &str) -> ApiResult<()> {
    if qty <= 0 {
        return Err(ApiError::Validation(format!("{what} must be positive")));
    }
    Ok(())
}

fn as_serial_number(req_serial_numbers: Option<Vec<String>>) -> Option<String> {
    req_serial_numbers
        .and_then(|v| v.into_iter().next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// POST /api/stock/receive — a positive leg into ONE location. Serialized
/// items move one physical unit per ledger row (exactly one serial).
#[utoipa::path(
    operation_id = "stock_receive",
    post,
    path = "/api/stock/receive",
    tag = "commercial",
    security(("bearer" = [])),
    request_body = ReceiveRequest,
    responses(
        (status = 200, body = MovementMutationResponse),
        (status = 404, description = "item or location not visible in scope"),
        (status = 409, description = "idempotency key reused, item archived, or invalid movement"),
        (status = 422, description = "validation failure")
    )
)]
pub async fn receive(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Json(req): axum::extract::Json<ReceiveRequest>,
) -> ApiResult<Json<MovementMutationResponse>> {
    require_pos(req.qty_milli, "receive quantity")?;
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let (kind, _currency) = item_kind(&state, &extensions, req.item_id).await?;
    let serialized = kind == "serialized";

    let qty_milli;
    let mut serial_number: Option<String> = None;
    if serialized {
        serial_number = Some(as_serial_number(req.serial_numbers.clone()).ok_or_else(|| {
            ApiError::Validation("a serialized receive must carry one serial".into())
        })?);
        if req.serial_numbers.as_ref().map_or(0, Vec::len) != 1 {
            return Err(ApiError::Validation(
                "pass exactly one serial per serialized receive".into(),
            ));
        }
        qty_milli = 1000;
    } else {
        if req.serial_numbers.is_some() {
            return Err(ApiError::Validation(
                "a consumable receive must not carry serials".into(),
            ));
        }
        qty_milli = req.qty_milli;
    }
    if let Some(cost) = req.unit_cost_minor {
        if cost < 0 {
            return Err(ApiError::Validation(
                "unit cost must not be negative".into(),
            ));
        }
    }

    let movement = core_domain::entities::StockMovement {
        id: Uuid::now_v7(),
        org_id,
        stock_item_id: req.item_id,
        kind: "receive".into(),
        from_location_id: None,
        to_location_id: Some(req.location_id),
        qty_milli,
        cost_minor: req.unit_cost_minor,
        currency: None,
        fx_rate_used: None,
        fx_rate_date: None,
        fx_source: None,
        base_cost_minor: None,
        serial_id: None,
        client_id: None,
        ticket_id: None,
        carrier: None,
        tracking_ref: None,
        note: req.note.clone(),
        created_by: None,
        created_at: Utc::now(),
    };
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    ledger_apply(
        state,
        extensions,
        "receive",
        fingerprint,
        req.instant.idempotency_key.clone(),
        movement,
        serial_number,
    )
    .await
}

fn require_serialized_consistent(
    serialized: bool,
    serial_id: &Option<Uuid>,
    what: &str,
) -> ApiResult<()> {
    if serialized != serial_id.is_some() {
        return Err(ApiError::Validation(if serialized {
            format!("a serialized {what} must carry the unit's serial_id")
        } else {
            format!("a consumable {what} must not carry a serial_id")
        }));
    }
    Ok(())
}

fn authed_principal(extensions: &Extensions) -> ApiResult<(core_domain::ids::OrgId, Uuid, bool)> {
    let authed = authed::<crate::auth::AuthedUser>(extensions)?;
    let financial = financial_read(&authed);
    Ok((
        authed.principal.user.org_id,
        authed.principal.user.id,
        financial,
    ))
}

/// POST /api/stock/consume — the OUT leg charging a ticket's job.
#[utoipa::path(
    operation_id = "stock_consume",
    post,
    path = "/api/stock/consume",
    tag = "commercial",
    security(("bearer" = [])),
    request_body = ConsumeRequest,
    responses(
        (status = 200, body = MovementMutationResponse),
        (status = 409, description = "insufficient stock or invalid movement"),
        (status = 422, description = "validation failure (qty_milli must be positive)")
    )
)]
pub async fn consume(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Json(req): axum::extract::Json<ConsumeRequest>,
) -> ApiResult<Json<MovementMutationResponse>> {
    require_pos(req.qty_milli, "consume quantity")?;
    let (org_id, _user_id, _financial) = authed_principal(&extensions)?;
    let (kind, _currency) = item_kind(&state, &extensions, req.item_id).await?;
    let serialized = kind == "serialized";
    require_serialized_consistent(serialized, &req.serial_id, "consume")?;
    let qty_milli = if serialized { 1000 } else { req.qty_milli };

    let movement = core_domain::entities::StockMovement {
        id: Uuid::now_v7(),
        org_id,
        stock_item_id: req.item_id,
        kind: "consume".into(),
        from_location_id: Some(req.location_id),
        to_location_id: None,
        qty_milli: -qty_milli,
        cost_minor: None,
        currency: None,
        fx_rate_used: None,
        fx_rate_date: None,
        fx_source: None,
        base_cost_minor: None,
        serial_id: req.serial_id,
        client_id: None,
        ticket_id: req.ticket_id,
        carrier: None,
        tracking_ref: None,
        note: req.note.clone(),
        created_by: None,
        created_at: Utc::now(),
    };
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    ledger_apply(
        state,
        extensions,
        "consume",
        fingerprint,
        req.instant.idempotency_key.clone(),
        movement,
        None,
    )
    .await
}

/// POST /api/stock/adjust — the signed counter-fix (NO serialized items;
/// move the unit instead).
#[utoipa::path(
    operation_id = "stock_adjust",
    post,
    path = "/api/stock/adjust",
    tag = "commercial",
    security(("bearer" = [])),
    request_body = AdjustRequest,
    responses(
        (status = 200, body = MovementMutationResponse),
        (status = 409, description = "insufficient stock or serialized item"),
        (status = 422, description = "validation failure (delta must be non-zero)")
    )
)]
pub async fn adjust(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Json(req): axum::extract::Json<AdjustRequest>,
) -> ApiResult<Json<MovementMutationResponse>> {
    if req.qty_milli == 0 {
        return Err(ApiError::Validation("adjust delta must be non-zero".into()));
    }
    let (org_id, _user_id, _financial) = authed_principal(&extensions)?;
    let (kind, _currency) = item_kind(&state, &extensions, req.item_id).await?;
    if kind == "serialized" {
        return Err(ApiError::Conflict(
            "adjust is not a serialized movement; move the unit instead".into(),
        ));
    }

    let movement = core_domain::entities::StockMovement {
        id: Uuid::now_v7(),
        org_id,
        stock_item_id: req.item_id,
        kind: "adjust".into(),
        from_location_id: if req.qty_milli < 0 {
            Some(req.location_id)
        } else {
            None
        },
        to_location_id: if req.qty_milli > 0 {
            Some(req.location_id)
        } else {
            None
        },
        qty_milli: req.qty_milli,
        cost_minor: None,
        currency: None,
        fx_rate_used: None,
        fx_rate_date: None,
        fx_source: None,
        base_cost_minor: None,
        serial_id: None,
        client_id: None,
        ticket_id: None,
        carrier: None,
        tracking_ref: None,
        note: req.note.clone(),
        created_by: None,
        created_at: Utc::now(),
    };
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    ledger_apply(
        state,
        extensions,
        "adjust",
        fingerprint,
        req.instant.idempotency_key.clone(),
        movement,
        None,
    )
    .await
}

/// POST /api/stock/transfer — shrink one location, grow another.
#[utoipa::path(
    operation_id = "stock_transfer",
    post,
    path = "/api/stock/transfer",
    tag = "commercial",
    security(("bearer" = [])),
    request_body = TransferRequest,
    responses(
        (status = 200, body = MovementMutationResponse),
        (status = 409, description = "insufficient stock or invalid movement"),
        (status = 422, description = "validation failure")
    )
)]
pub async fn transfer(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Json(req): axum::extract::Json<TransferRequest>,
) -> ApiResult<Json<MovementMutationResponse>> {
    require_pos(req.qty_milli, "transfer quantity")?;
    let (org_id, _user_id, _financial) = authed_principal(&extensions)?;
    let (kind, _currency) = item_kind(&state, &extensions, req.item_id).await?;
    let serialized = kind == "serialized";
    require_serialized_consistent(serialized, &req.serial_id, "transfer")?;
    let qty_milli = if serialized { 1000 } else { req.qty_milli };

    let movement = core_domain::entities::StockMovement {
        id: Uuid::now_v7(),
        org_id,
        stock_item_id: req.item_id,
        kind: "transfer".into(),
        from_location_id: Some(req.from_location_id),
        to_location_id: Some(req.to_location_id),
        qty_milli,
        cost_minor: None,
        currency: None,
        fx_rate_used: None,
        fx_rate_date: None,
        fx_source: None,
        base_cost_minor: None,
        serial_id: req.serial_id,
        client_id: None,
        ticket_id: None,
        carrier: None,
        tracking_ref: None,
        note: req.note.clone(),
        created_by: None,
        created_at: Utc::now(),
    };
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    ledger_apply(
        state,
        extensions,
        "transfer",
        fingerprint,
        req.instant.idempotency_key.clone(),
        movement,
        None,
    )
    .await
}

/// POST /api/stock/ship-out — OUT to a client or an on-site job, with
/// optional carrier + tracking ref.
#[utoipa::path(
    operation_id = "stock_ship_out",
    post,
    path = "/api/stock/ship-out",
    tag = "commercial",
    security(("bearer" = [])),
    request_body = ShipOutRequest,
    responses(
        (status = 200, body = MovementMutationResponse),
        (status = 409, description = "insufficient stock or invalid movement"),
        (status = 422, description = "validation failure")
    )
)]
pub async fn ship_out(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Json(req): axum::extract::Json<ShipOutRequest>,
) -> ApiResult<Json<MovementMutationResponse>> {
    require_pos(req.qty_milli, "ship-out quantity")?;
    if req.client_id.is_none() && req.ticket_id.is_none() {
        return Err(ApiError::Validation(
            "a ship-out needs a client or a ticket destination".into(),
        ));
    }
    let (org_id, _user_id, _financial) = authed_principal(&extensions)?;
    let (kind, _currency) = item_kind(&state, &extensions, req.item_id).await?;
    let serialized = kind == "serialized";
    require_serialized_consistent(serialized, &req.serial_id, "ship-out")?;
    let qty_milli = if serialized { 1000 } else { req.qty_milli };

    // Fingerprint BEFORE the request is consumed by movement below.
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let movement = core_domain::entities::StockMovement {
        id: Uuid::now_v7(),
        org_id,
        stock_item_id: req.item_id,
        kind: "ship_out".into(),
        from_location_id: Some(req.location_id),
        to_location_id: None,
        qty_milli: -qty_milli,
        cost_minor: None,
        currency: None,
        fx_rate_used: None,
        fx_rate_date: None,
        fx_source: None,
        base_cost_minor: None,
        serial_id: req.serial_id,
        client_id: req.client_id,
        ticket_id: req.ticket_id,
        carrier: req.carrier,
        tracking_ref: req.tracking_ref,
        note: req.note.clone(),
        created_by: None,
        created_at: Utc::now(),
    };
    ledger_apply(
        state,
        extensions,
        "ship_out",
        fingerprint,
        key,
        movement,
        None,
    )
    .await
}

/// POST /api/stock/levels/thresholds — min/max/reorder per (item,
/// location); NULL clears. Creates the level row on first touch, without
/// moving stock.
#[utoipa::path(
    operation_id = "stock_thresholds_set",
    post,
    path = "/api/stock/levels/thresholds",
    tag = "commercial",
    security(("bearer" = [])),
    request_body = ThresholdsRequest,
    responses((status = 200, body = StockLevelView))
)]
pub async fn set_thresholds_route(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Json(req): axum::extract::Json<ThresholdsRequest>,
) -> ApiResult<Json<StockLevelView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    core_db::stock::get_stock_item(&mut *tx, org_id, req.item_id).await?;
    core_db::stock::get_stock_location(&mut *tx, org_id, req.location_id).await?;
    let level = core_db::stock::set_thresholds(
        &mut *tx,
        org_id,
        req.item_id,
        req.location_id,
        req.min_qty_milli,
        req.max_qty_milli,
        req.reorder_point_milli,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(StockLevelView::from(level)))
}

/// POST thresholds response: the fresh threshold row.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct StockLevelView {
    #[schema(value_type = uuid::Uuid)]
    pub stock_item_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub stock_location_id: Uuid,
    pub qty_milli: i64,
    #[schema(value_type = Option<i64>)]
    pub min_qty_milli: Option<i64>,
    #[schema(value_type = Option<i64>)]
    pub max_qty_milli: Option<i64>,
    #[schema(value_type = Option<i64>)]
    pub reorder_point_milli: Option<i64>,
    #[schema(value_type = String)]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<core_domain::entities::StockLevel> for StockLevelView {
    fn from(l: core_domain::entities::StockLevel) -> Self {
        Self {
            stock_item_id: l.stock_item_id,
            stock_location_id: l.stock_location_id,
            qty_milli: l.qty_milli,
            min_qty_milli: l.min_qty_milli,
            max_qty_milli: l.max_qty_milli,
            reorder_point_milli: l.reorder_point_milli,
            updated_at: l.updated_at,
        }
    }
}

/// GET /api/stock/on-hand — the stock-on-hand read model.
#[utoipa::path(
    operation_id = "stock_on_hand",
    get,
    path = "/api/stock/on-hand",
    tag = "commercial",
    security(("bearer" = [])),
    params(OnHandQuery),
    responses((status = 200, body = [OnHandRow]))
)]
pub async fn on_hand(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Query(q): axum::extract::Query<OnHandQuery>,
) -> ApiResult<Json<Vec<OnHandRow>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let org_id = authed.principal.user.org_id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let levels = core_db::stock::list_levels(
        &mut tx,
        org_id,
        q.item_id,
        q.location_id,
        q.limit.unwrap_or(LIST_LIMIT).min(LIST_LIMIT),
    )
    .await?;

    let mut items: std::collections::HashMap<Uuid, (String, String, String)> =
        std::collections::HashMap::new();
    let mut locations: std::collections::HashMap<Uuid, (String, String)> =
        std::collections::HashMap::new();
    let mut rows = Vec::with_capacity(levels.len());
    for level in levels {
        let (item_name, item_kind, unit) = match items.get(&level.stock_item_id) {
            Some(hit) => hit.clone(),
            None => {
                let item =
                    core_db::stock::get_stock_item(&mut *tx, org_id, level.stock_item_id).await?;
                let hit = (item.name, item.kind, item.unit);
                items.insert(level.stock_item_id, hit.clone());
                hit
            }
        };
        let (location_name, location_kind) = match locations.get(&level.stock_location_id) {
            Some(hit) => hit.clone(),
            None => {
                let loc =
                    core_db::stock::get_stock_location(&mut *tx, org_id, level.stock_location_id)
                        .await?;
                let hit = (loc.name, loc.kind);
                locations.insert(level.stock_location_id, hit.clone());
                hit
            }
        };
        let low = match level.reorder_point_milli {
            Some(point) => level.qty_milli <= point,
            None => false,
        };
        if low && !q.low_only.unwrap_or(false) {
            // keep
        }
        if !q.low_only.unwrap_or(false) || low {
            rows.push(OnHandRow {
                stock_item_id: level.stock_item_id,
                item_name,
                item_kind,
                unit,
                stock_location_id: level.stock_location_id,
                location_name,
                location_kind,
                qty_milli: level.qty_milli,
                min_qty_milli: level.min_qty_milli,
                max_qty_milli: level.max_qty_milli,
                reorder_point_milli: level.reorder_point_milli,
                low,
            });
        }
    }
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// GET /api/stock/consumption-velocity — the outbound-rate read model.
#[utoipa::path(
    operation_id = "stock_consumption_velocity",
    get,
    path = "/api/stock/consumption-velocity",
    tag = "commercial",
    security(("bearer" = [])),
    params(VelocityQuery),
    responses((status = 200, body = [VelocityMutationless]))
)]
pub async fn consumption_velocity_route(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Query(q): axum::extract::Query<VelocityQuery>,
) -> ApiResult<Json<Vec<VelocityMutationless>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let window_days = q.window_days.unwrap_or(30).clamp(1, 365);

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::stock::consumption_velocity(
        &mut tx,
        org_id,
        window_days.clamp(1, 365) as i32,
        q.limit.unwrap_or(LIST_LIMIT).min(LIST_LIMIT),
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(
        rows.into_iter()
            .map(|r| VelocityMutationless {
                stock_item_id: r.stock_item_id,
                item_name: r.item_name,
                unit: r.unit,
                consumed_milli: r.consumed_milli,
                avg_daily_milli: r.avg_daily_milli,
            })
            .collect(),
    ))
}

// --- traceability + serials reads -------------------------------------------

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct MovementListQuery {
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct SerialListQuery {
    #[serde(default)]
    pub location_id: Option<Uuid>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// GET /api/stock-items/{id}/movements — THE traceability ledger read.
#[utoipa::path(
    operation_id = "stock_item_movements",
    get,
    path = "/api/stock-items/{id}/movements",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path), MovementListQuery),
    responses((status = 200, body = [StockMovementView]))
)]
pub async fn item_movements(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<MovementListQuery>,
) -> ApiResult<Json<Vec<StockMovementView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let financial = financial_read(&authed);
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let movements = core_db::stock::list_movements(
        &mut tx,
        authed.principal.user.org_id,
        id,
        q.limit.unwrap_or(LIST_LIMIT).min(LIST_LIMIT),
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(
        movements
            .into_iter()
            .map(|mv| mv_view(mv, financial))
            .collect(),
    ))
}

/// GET /api/stock-items/{id}/serials — serialized WHERE-IT-IS-NOW.
#[utoipa::path(
    operation_id = "stock_item_serials",
    get,
    path = "/api/stock-items/{id}/serials",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path), SerialListQuery),
    responses((status = 200, body = [StockSerialView]))
)]
pub async fn item_serials(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<SerialListQuery>,
) -> ApiResult<Json<Vec<StockSerialView>>> {
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
    let serials = core_db::stock::list_serials(
        &mut tx,
        authed.principal.user.org_id,
        id,
        q.location_id,
        q.status.as_deref(),
        q.limit.unwrap_or(LIST_LIMIT).min(LIST_LIMIT),
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(
        serials.into_iter().map(StockSerialView::from).collect(),
    ))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct StockSerialView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub stock_item_id: Uuid,
    pub serial_number: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub location_id: Option<Uuid>,
    pub status: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub ticket_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub client_id: Option<Uuid>,
    #[schema(value_type = String)]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<core_domain::entities::StockSerial> for StockSerialView {
    fn from(s: core_domain::entities::StockSerial) -> Self {
        Self {
            id: s.id,
            stock_item_id: s.stock_item_id,
            serial_number: s.serial_number,
            location_id: s.location_id,
            status: s.status,
            ticket_id: s.ticket_id,
            client_id: s.client_id,
            updated_at: s.updated_at,
        }
    }
}
