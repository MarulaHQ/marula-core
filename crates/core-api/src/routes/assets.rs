//! Asset endpoints (Wave 11 lane 5, migration 0054): typed asset
//! instances on the reconciled docs-platform. Types ride
//! `document_layouts` (asset-shaped system layouts are the taxonomy;
//! asset-type CRUD = document-layout CRUD), instances stay in
//! `device_assets` with `layout_id` + schema-validated `fields`.
//!
//! External-DMS parity (binding): every endpoint is plain
//! resource-oriented REST on the v1 envelope; mutations ride the
//! instant-first envelope (IF1–IF3); lifecycle events register as
//! `asset.*` so the MCP/connector seam can mirror an external DMS.
//!
//! Integration link status is DATA, not live sync: connector
//! installations are exposed read-only (linked/unlinked + last-touch
//! timestamps); no sync runs here.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::routes::documents::{link_target_exists, links_for, validate_fields};
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use core_db::assets::AssetLifecyclePatch;
use core_db::document_layouts::get_layout;
use core_domain::entities::{DeviceAsset, LinkKind};
use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use utoipa::ToSchema;
use uuid::Uuid;

fn assets(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Clients,
        action,
    }
}

fn vault(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Passwords,
        action,
    }
}

/// Instant-first mutation response (IF2): asset fields at the top level
/// plus the additive `mutation` metadata.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AssetMutationResponse {
    #[serde(flatten)]
    pub asset: DeviceAsset,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateAssetRequest {
    pub client_id: Uuid,
    #[serde(default)]
    pub site_id: Option<Uuid>,
    /// Human asset tag, e.g. "EXCH-01".
    pub tag: String,
    pub name: String,
    /// Free-text connector-compat column (typed classification rides
    /// `layout_id`).
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default = "default_status")]
    pub status: String,
    /// Typed layout; when set, `fields` is validated against its schema.
    #[serde(default)]
    pub layout_id: Option<Uuid>,
    #[serde(default)]
    pub fields: Option<Value>,
    /// Inventory lifecycle inputs (rental tracking + depreciation,
    /// migration 0061); all optional, absent = SQL default.
    #[serde(default)]
    pub lifecycle: Option<AssetLifecycleInput>,
    /// Instant-first mutation parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// Rental tracking + depreciation inputs on an asset (money is integer
/// minor units + currency code per ADR 0002; no conversion happens here).
#[derive(Debug, Default, Clone, Deserialize, Serialize, ToSchema)]
pub struct AssetLifecycleInput {
    /// owned | leased | rented
    #[serde(default)]
    pub ownership: Option<String>,
    #[serde(default)]
    pub rental_vendor: Option<String>,
    #[serde(default)]
    pub rental_start: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub rental_end: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub rental_cost_minor: Option<i64>,
    #[serde(default)]
    pub rental_currency: Option<String>,
    /// monthly | quarterly | annual
    #[serde(default)]
    pub rental_cycle: Option<String>,
    #[serde(default)]
    pub purchase_date: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub purchase_cost_minor: Option<i64>,
    #[serde(default)]
    pub purchase_currency: Option<String>,
    /// none | straight_line | declining_balance
    #[serde(default)]
    pub depreciation_method: Option<String>,
    #[serde(default)]
    pub useful_life_months: Option<i32>,
    #[serde(default)]
    pub salvage_value_minor: Option<i64>,
    /// basis points for declining_balance (1..9999)
    #[serde(default)]
    pub depreciation_rate_bps: Option<i32>,
}

impl From<AssetLifecycleInput> for AssetLifecyclePatch {
    fn from(v: AssetLifecycleInput) -> Self {
        AssetLifecyclePatch {
            ownership: v.ownership,
            rental_vendor: v.rental_vendor,
            rental_start: v.rental_start,
            rental_end: v.rental_end,
            rental_cost_minor: v.rental_cost_minor,
            rental_currency: v.rental_currency,
            rental_cycle: v.rental_cycle,
            purchase_date: v.purchase_date,
            purchase_cost_minor: v.purchase_cost_minor,
            purchase_currency: v.purchase_currency,
            purchase_fx_rate_used: None,
            purchase_fx_rate_date: None,
            purchase_fx_source: None,
            base_purchase_cost_minor: None,
            depreciation_method: v.depreciation_method,
            useful_life_months: v.useful_life_months,
            salvage_value_minor: v.salvage_value_minor,
            depreciation_rate_bps: v.depreciation_rate_bps,
        }
    }
}

fn default_kind() -> String {
    "device".into()
}

fn default_status() -> String {
    "active".into()
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateAssetRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub site_id: Option<Uuid>,
    #[serde(default)]
    pub layout_id: Option<Uuid>,
    #[serde(default)]
    pub fields: Option<Value>,
    /// Inventory lifecycle inputs (rental tracking + depreciation);
    /// absent = keep (patch semantics).
    #[serde(default)]
    pub lifecycle: Option<AssetLifecycleInput>,
    /// Instant-first mutation parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListAssetsQuery {
    #[serde(default)]
    pub client_id: Option<Uuid>,
    #[serde(default)]
    pub layout_id: Option<Uuid>,
    /// Substring over tag, name, and custom fields.
    #[serde(default)]
    pub q: Option<String>,
}

/// GET /api/assets — org-wide, filterable (resource REST for external-DMS
/// parity; the client-scoped tab narrows with `?client_id=`).
#[utoipa::path(operation_id = "assets_list", get, path = "/api/assets", tag = "assets", security(("bearer" = [])),
    params(ListAssetsQuery),
    responses((status = 200, body = [DeviceAsset])))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ListAssetsQuery>,
) -> ApiResult<Json<Vec<DeviceAsset>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, assets(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows =
        core_db::assets::list_assets(&mut *tx, org_id, q.client_id, q.layout_id, q.q.as_deref())
            .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// GET /api/assets/{id}
#[utoipa::path(operation_id = "assets_get", get, path = "/api/assets/{id}", tag = "assets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = DeviceAsset),
              (status = 404, description = "asset not visible in scope")))]
pub async fn get_one(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<DeviceAsset>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, assets(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let asset = core_db::assets::get_asset(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(asset))
}

/// POST /api/assets — instant-first (IF1/IF2).
#[utoipa::path(operation_id = "assets_create", post, path = "/api/assets", tag = "assets", security(("bearer" = [])),
    request_body = CreateAssetRequest,
    responses((status = 200, body = AssetMutationResponse),
              (status = 409, description = "idempotency key reused"),
              (status = 422, description = "fields rejected by the layout schema")))]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateAssetRequest>,
) -> ApiResult<Json<AssetMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, assets(Action::Create))?;
    let tag = req.tag.trim().to_string();
    if tag.is_empty() {
        return Err(ApiError::Validation("tag is required".into()));
    }
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::Validation("name is required".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    // Layout + fields validated OUTSIDE the org tx: a bad layout id or
    // schema-mismatched fields must not consume a mutation claim.
    let fields = req
        .fields
        .clone()
        .unwrap_or_else(|| Value::Object(Default::default()));
    if let Some(layout_id) = req.layout_id {
        let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
        core_db::set_org_and_user(&mut tx, org_id, user_id)
            .await
            .map_err(ApiError::from)?;
        let layout = get_layout(&mut *tx, org_id, layout_id).await?;
        tx.commit().await.map_err(ApiError::from)?;
        validate_fields(&layout, &fields)?;
    }

    let (client_id, site_id, kind, status, layout_id) = (
        req.client_id,
        req.site_id,
        req.kind,
        req.status,
        req.layout_id,
    );
    let req_lifecycle = req.lifecycle.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "asset",
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
        let mut asset = core_db::assets::create_asset(
            &mut *ctx.tx,
            org_id,
            id,
            client_id,
            site_id,
            &tag,
            &name,
            &kind,
            &status,
            layout_id,
            &fields,
        )
        .await?;
        if let Some(lifecycle) = &req_lifecycle {
            asset = core_db::assets::update_asset_lifecycle(
                &mut *ctx.tx,
                org_id,
                id,
                &AssetLifecyclePatch::from(lifecycle.clone()),
            )
            .await?;
        }
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "asset",
                entity_id: id,
                version: None,
            },
        )
        .await?;
        ctx.emit(
            types::ASSET_CREATED,
            "asset",
            Some(id.to_string().as_str()),
            &serde_json::json!({ "asset_id": id, "client_id": client_id }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "asset",
                entity_id: id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&asset).unwrap_or(Value::Null),
                updated_at: asset.created_at,
                key: key.as_deref(),
                payload: &asset,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(AssetMutationResponse { asset, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/assets/{id} — instant-first (IF1/IF2); absent = keep. A
/// layout change re-validates the effective fields against the NEW schema.
#[utoipa::path(operation_id = "assets_update", patch, path = "/api/assets/{id}", tag = "assets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = UpdateAssetRequest,
    responses((status = 200, body = AssetMutationResponse),
              (status = 404, description = "asset not visible in scope"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "fields rejected by the layout schema")))]
pub async fn update(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateAssetRequest>,
) -> ApiResult<Json<AssetMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, assets(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let version = req.instant.version;

    let pool = state.pool.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "asset",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::assets::get_asset(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;

        // Effective layout + fields validation (same rule as documents:
        // a layout change re-validates the kept fields vs the new schema).
        let next_layout_id = match &req.layout_id {
            Some(new_layout) => {
                let layout = get_layout(&mut *ctx.tx, org_id, *new_layout)
                    .await
                    .map_err(|e| instant::mutation_scope_err(e.into()))?;
                let effective_fields = req.fields.clone().unwrap_or_else(|| before.fields.clone());
                validate_fields(&layout, &effective_fields)?;
                Some(Some(*new_layout))
            }
            None => {
                if let Some(f) = &req.fields {
                    if let Some(current_layout) = before.layout_id {
                        let layout = get_layout(&mut *ctx.tx, org_id, current_layout)
                            .await
                            .map_err(|e| instant::mutation_scope_err(e.into()))?;
                        validate_fields(&layout, f)?;
                    }
                }
                None
            }
        };

        let asset = core_db::assets::update_asset(
            &mut *ctx.tx,
            org_id,
            id,
            req.name.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            req.status.as_deref(),
            req.site_id.map(Some),
            next_layout_id,
            req.fields.as_ref(),
        )
        .await?;
        let asset = match &req.lifecycle {
            Some(lifecycle) => {
                let mut patch = AssetLifecyclePatch::from(lifecycle.clone());
                // ADR 0002 lock moment (inventory receipt): a purchase cost
                // written in a non-base currency locks its conversion
                // snapshot; the snapshot columns are then immutable (DB
                // trigger). The transaction currency stays the fact.
                if let (Some(cost), Some(cur)) = (
                    lifecycle.purchase_cost_minor,
                    lifecycle.purchase_currency.as_deref(),
                ) {
                    let snap = crate::routes::fx::lock_snapshot(
                        &pool,
                        org_id,
                        cost,
                        cur,
                        lifecycle
                            .purchase_date
                            .unwrap_or_else(|| chrono::Utc::now().date_naive()),
                    )
                    .await?;
                    if let Some(s) = snap {
                        patch.purchase_fx_rate_used = Some(s.rate_used);
                        patch.purchase_fx_rate_date = Some(s.rate_date);
                        patch.purchase_fx_source = Some(s.source);
                        patch.base_purchase_cost_minor = Some(s.base_amount_minor);
                    }
                }
                core_db::assets::update_asset_lifecycle(&mut *ctx.tx, org_id, id, &patch).await?
            }
            None => asset,
        };
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "asset",
                entity_id: id,
                version,
            },
        )
        .await?;
        ctx.emit(
            types::ASSET_UPDATED,
            "asset",
            Some(id.to_string().as_str()),
            &serde_json::json!({ "asset_id": id }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "asset",
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&asset).unwrap_or(Value::Null),
                updated_at: asset.updated_at,
                key: key.as_deref(),
                payload: &asset,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(AssetMutationResponse { asset, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/assets/{id} — the audit trail keeps the events; link rows
/// survive and surface as broken targets (docs-platform semantics: never
/// cascade-deleted). Mirrors the vault's delete shape (`deleted: true`),
/// which intentionally rides no instant-first envelope.
#[utoipa::path(operation_id = "assets_delete", delete, path = "/api/assets/{id}", tag = "assets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "deleted"),
              (status = 404, description = "asset not visible in scope")))]
pub async fn remove(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, assets(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::assets::get_asset(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        core_db::assets::delete_asset(&mut *ctx.tx, org_id, id).await?;
        ctx.emit(
            types::ASSET_DELETED,
            "asset",
            Some(id.to_string().as_str()),
            &serde_json::json!({ "asset_id": id }),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;

    Ok(Json(serde_json::json!({ "deleted": true })))
}

// ---- cross-entity links (asset kind, 0054) --------------------------------

/// GET /api/assets/{id}/links — the related-items panel.
#[utoipa::path(operation_id = "assets_list_links", get, path = "/api/assets/{id}/links", tag = "assets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [core_domain::entities::ResolvedLink])))]
pub async fn list_links(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<core_domain::entities::ResolvedLink>>> {
    links_for(&state, &extensions, LinkKind::Asset, id).await
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateAssetLinkRequest {
    /// One of: document | password | client | ticket | kb_article | asset.
    pub to_kind: String,
    pub to_id: Uuid,
}

/// POST /api/assets/{id}/links
#[utoipa::path(operation_id = "assets_add_link", post, path = "/api/assets/{id}/links", tag = "assets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = CreateAssetLinkRequest,
    responses((status = 200, body = [core_domain::entities::ResolvedLink]),
              (status = 404, description = "asset or link target not visible in scope"),
              (status = 409, description = "link already exists")))]
pub async fn add_link(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateAssetLinkRequest>,
) -> ApiResult<Json<Vec<core_domain::entities::ResolvedLink>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, assets(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let to_kind = LinkKind::parse(&req.to_kind)
        .ok_or_else(|| ApiError::Validation(format!("unknown link kind '{}'", req.to_kind)))?;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::assets::get_asset(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        link_target_exists(&mut ctx.tx, org_id, to_kind, req.to_id).await?;
        let link = core_db::document_links::create_link(
            &mut *ctx.tx,
            org_id,
            Uuid::now_v7(),
            LinkKind::Asset,
            id,
            to_kind,
            req.to_id,
            user_id,
        )
        .await?;
        ctx.emit(
            types::LINK_ADDED,
            "link",
            Some(link.id.to_string().as_str()),
            &serde_json::json!({
                "from_kind": "asset", "from_id": id,
                "to_kind": to_kind.as_str(), "to_id": req.to_id
            }),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    links_for(&state, &extensions, LinkKind::Asset, id).await
}

/// GET /api/assets/{id}/vault — the asset's vault entries: password rows
/// LINKED to this asset (password→asset). Strict posture: metadata only
/// (title/username/url/expiry) under `passwords:view`; secrets exist only
/// behind the audited `/passwords/{id}/reveal` endpoint, which requires
/// `passwords:edit` — the UI must render role-gated reveal, never a dead
/// button. No ciphertext ever appears in this response.
#[utoipa::path(operation_id = "assets_list_vault", get, path = "/api/assets/{id}/vault", tag = "assets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [VaultEntryView]),
              (status = 404, description = "asset not visible in scope")))]
pub async fn list_vault(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<VaultEntryView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, vault(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    // Scope check on the asset itself (typed 404, not an empty list).
    core_db::assets::get_asset(&mut *tx, org_id, id).await?;
    let rows = sqlx::query(
        r#"SELECT p.id, p.title, p.username, p.url, p.expires_at
           FROM document_links l
           JOIN passwords p ON p.org_id = l.org_id AND p.id = l.from_id
           WHERE l.org_id = $1 AND l.from_kind = 'password' AND l.to_kind = 'asset' AND l.to_id = $2
           ORDER BY p.title"#,
    )
    .bind(org_id)
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(
        rows.into_iter()
            .map(|r| VaultEntryView {
                id: r.get("id"),
                title: r.get("title"),
                username: r.get("username"),
                url: r.get("url"),
                expires_at: r.get("expires_at"),
            })
            .collect(),
    ))
}

/// A vault entry as the asset detail page sees it: metadata only —
/// reveal rides the audited `/passwords/{id}/reveal` endpoint.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct VaultEntryView {
    #[serde(rename = "id")]
    pub id: Uuid,
    pub title: String,
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

// ---- integration link status (data, not sync) ------------------------------

/// GET /api/assets/integration-links — per-connector link state as data
/// (QBO/Xero and friends): linked/unlinked + last-touch timestamps. No
/// live sync runs here; the `ConfigSourceLink` seam (ADR 0003) attaches
/// per-asset state when it lands.
#[utoipa::path(operation_id = "assets_integration_links", get, path = "/api/assets/integration-links", tag = "assets", security(("bearer" = [])),
    responses((status = 200, body = [IntegrationLinkStatus])))]
pub async fn integration_links(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<IntegrationLinkStatus>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, assets(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::webhooks::list_connector_installations(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(
        rows.into_iter()
            .map(|r| IntegrationLinkStatus {
                connector_type: r.connector_type,
                linked_at: r.created_at,
                updated_at: r.updated_at,
            })
            .collect(),
    ))
}

/// Per-connector link state as data (linked/unlinked + last touch).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct IntegrationLinkStatus {
    pub connector_type: String,
    pub linked_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
