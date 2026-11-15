//! Knowledge home extensions (client-first documentation, rack layouts,
//! network records, procedures and the Hudu-parity build-out).
//!
//! Registered from `routes::mod` (`knowledge_home::routes(gen)`). The existing
//! `kb`, `secrets`, `documents`, `passwords`, `config_items` and `assets`
//! modules keep their routes; this module is where the client-first home and
//! its new record types live.
//!
//! Reading order the surfaces follow: the client is the front door
//! (`GET /knowledge/home`), the client hub is a summary that links out
//! (`GET /knowledge/clients/{id}/hub`), and each record type has its own
//! resource below it. Completness is returned as a counted set of named legs,
//! never a bare percentage.
//!
//! Mutations run inside the org-scoped transaction (RLS set) and write an
//! `audit_events` row in the same transaction, so "who changed this" is a
//! query rather than a promise. They do not yet ride the instant-first
//! envelope — see the lane report's gap list.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::routes::ApiGeneration;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    routing::{delete, get, patch, post},
    Json, Router,
};
use chrono::{DateTime, NaiveDate, Utc};
use core_db::knowledge_home as kh;
use core_domain::entities::{Actor, AuditEvent};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

/* ------------------------------------------------------------------------- */
/* Permission shorthands                                                     */
/* ------------------------------------------------------------------------- */

fn kb(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Kb,
        action,
    }
}

fn secrets(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Secrets,
        action,
    }
}

fn trimmed(field: &str, value: &str) -> ApiResult<String> {
    let v = value.trim().to_string();
    if v.is_empty() {
        return Err(ApiError::Validation(format!("{field} is required")));
    }
    Ok(v)
}

fn one_of(field: &str, value: &str, allowed: &[&str]) -> ApiResult<()> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(ApiError::Validation(format!(
            "{field} must be one of {}",
            allowed.join(", ")
        )))
    }
}

/// Append-only audit row in the caller's transaction. The action vocabulary is
/// dotted and prefixed `knowledge.` so the admin audit query can select the
/// documentation story without a join.
#[allow(clippy::too_many_arguments)]
async fn audit(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    org_id: Uuid,
    user_id: Uuid,
    action: &str,
    entity_type: &str,
    entity_id: Uuid,
    detail: Value,
) -> ApiResult<()> {
    let event = AuditEvent {
        id: Uuid::now_v7(),
        org_id,
        actor: Actor::Human(user_id),
        action: action.to_string(),
        entity_type: Some(entity_type.to_string()),
        entity_id: Some(entity_id.to_string()),
        detail,
        ip: None,
        occurred_at: Utc::now(),
    };
    core_db::audit::insert(&mut **tx, &event)
        .await
        .map_err(ApiError::from)?;
    Ok(())
}

/* ------------------------------------------------------------------------- */
/* Home                                                                      */
/* ------------------------------------------------------------------------- */

#[derive(Debug, Serialize, ToSchema)]
pub struct CompletenessView {
    pub complete: i64,
    pub total: i64,
    pub gaps: Vec<String>,
}

impl From<kh::Completeness> for CompletenessView {
    fn from(c: kh::Completeness) -> Self {
        Self {
            complete: c.complete,
            total: c.total,
            gaps: c.gaps,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HomeClient {
    pub id: Uuid,
    pub name: String,
    pub sites_n: i64,
    pub assets_n: i64,
    pub documents_n: i64,
    pub articles_n: i64,
    pub secrets_n: i64,
    pub racks_n: i64,
    pub networks_n: i64,
    pub domains_n: i64,
    pub procedures_n: i64,
    pub stale_secrets_n: i64,
    pub expiring_n: i64,
    pub completeness: CompletenessView,
    pub last_activity: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HomeResponse {
    pub clients: Vec<HomeClient>,
    pub org: kh::OrgKnowledgeCounts,
    pub expiring_soon_n: i64,
}

/// GET /api/knowledge/home — the client directory that replaces the
/// article-first landing: search, the clients, org knowledge, recent movement.
#[utoipa::path(operation_id = "knowledge_home", get, path = "/api/knowledge/home", tag = "knowledge", security(("bearer" = [])),
    responses((status = 200, body = serde_json::Value)))]
pub async fn home(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<HomeResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    let facts = kh::list_client_facts(&mut *tx, org_id).await?;
    let names = kh::list_client_names(&mut *tx, org_id).await?;
    let org = kh::org_knowledge_counts(&mut *tx, org_id).await?;
    let expiring = kh::expirations(&mut *tx, org_id, None, 60).await?;
    tx.commit().await.map_err(ApiError::from)?;

    let by_id: std::collections::HashMap<Uuid, String> = names.into_iter().collect();
    let clients: Vec<HomeClient> = facts
        .into_iter()
        .map(|f| HomeClient {
            id: f.client_id,
            name: by_id.get(&f.client_id).cloned().unwrap_or_default(),
            sites_n: f.sites_n,
            assets_n: f.assets_n,
            documents_n: f.documents_n,
            articles_n: f.articles_n,
            secrets_n: f.secrets_n,
            racks_n: f.racks_n,
            networks_n: f.networks_n,
            domains_n: f.domains_n,
            procedures_n: f.procedures_n,
            stale_secrets_n: f.stale_secrets_n,
            expiring_n: f.expiring_n,
            completeness: f.completeness().into(),
            last_activity: f.last_activity(),
        })
        .collect();

    Ok(Json(HomeResponse {
        clients,
        org,
        expiring_soon_n: expiring.len() as i64,
    }))
}

/* ------------------------------------------------------------------------- */
/* Hub                                                                       */
/* ------------------------------------------------------------------------- */

#[derive(Debug, Serialize, ToSchema)]
pub struct HubHeader {
    pub client_id: Uuid,
    pub name: String,
    pub owner_name: Option<String>,
    pub review_note: Option<String>,
    pub completeness: CompletenessView,
    pub last_activity: Option<DateTime<Utc>>,
    pub pinned_document_ids: Vec<Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HubSections {
    pub articles_n: i64,
    pub documents_n: i64,
    pub assets_n: i64,
    pub secrets_n: i64,
    pub domains_n: i64,
    pub networks_n: i64,
    pub racks_n: i64,
    pub procedures_n: i64,
    pub sites_n: i64,
    pub contacts_n: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HubResponse {
    pub header: HubHeader,
    pub sections: HubSections,
    pub expirations: Vec<kh::ExpiryRow>,
    pub recent_changes: Vec<kh::RecentChange>,
    pub secrets_glance: kh::SecretsGlance,
}

/// GET /api/knowledge/clients/{client_id}/hub — the reading surface: header,
/// completeness with its reasons, section counts that link out, the expiries
/// that need attention, recent movement and a value-free vault glance.
#[utoipa::path(operation_id = "knowledge_client_hub", get, path = "/api/knowledge/clients/{client_id}/hub", tag = "knowledge", security(("bearer" = [])),
    params(("client_id" = Uuid, Path)),
    responses((status = 200, body = serde_json::Value), (status = 404, description = "client not in scope")))]
pub async fn client_hub(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(client_id): Path<Uuid>,
) -> ApiResult<Json<HubResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    let facts = kh::client_facts(&mut *tx, org_id, client_id).await?;
    let name = kh::client_name(&mut *tx, org_id, client_id).await?;
    let settings = kh::get_client_settings(&mut *tx, org_id, client_id).await?;
    let expirations = kh::expirations(&mut *tx, org_id, Some(client_id), 90).await?;
    let recent = kh::recent_changes(&mut *tx, org_id, client_id, 8).await?;
    let glance = kh::secrets_glance(&mut *tx, org_id, client_id).await?;
    tx.commit().await.map_err(ApiError::from)?;

    let (owner_name, review_note, pinned) = match settings {
        Some(s) => (s.owner_name, s.review_note, s.pinned_document_ids),
        None => (None, None, Vec::new()),
    };

    Ok(Json(HubResponse {
        header: HubHeader {
            client_id,
            name,
            owner_name,
            review_note,
            completeness: facts.completeness().into(),
            last_activity: facts.last_activity(),
            pinned_document_ids: pinned,
        },
        sections: HubSections {
            articles_n: facts.articles_n,
            documents_n: facts.documents_n,
            assets_n: facts.assets_n,
            secrets_n: facts.secrets_n,
            domains_n: facts.domains_n,
            networks_n: facts.networks_n,
            racks_n: facts.racks_n,
            procedures_n: facts.procedures_n,
            sites_n: facts.sites_n,
            contacts_n: facts.contacts_n,
        },
        expirations,
        recent_changes: recent,
        secrets_glance: glance,
    }))
}

/* ------------------------------------------------------------------------- */
/* Expiries and search                                                       */
/* ------------------------------------------------------------------------- */

#[derive(Debug, Deserialize)]
pub struct ExpiryQuery {
    pub client_id: Option<Uuid>,
    /// Days ahead to look. Default 90, capped at 730 so one call cannot ask
    /// for a decade of history.
    pub days: Option<i32>,
}

/// GET /api/knowledge/expirations — one rollup across domains, certificates,
/// document reviews, asset warranties and credential rotation.
#[utoipa::path(operation_id = "knowledge_expirations", get, path = "/api/knowledge/expirations", tag = "knowledge", security(("bearer" = [])), 
    params(("client_id" = Option<Uuid>, Query), ("days" = Option<i32>, Query)),
    responses((status = 200, body = [kh::ExpiryRow])))]
pub async fn expirations(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ExpiryQuery>,
) -> ApiResult<Json<Vec<kh::ExpiryRow>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let days = q.days.unwrap_or(90).clamp(1, 730);
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = kh::expirations(&mut *tx, org_id, q.client_id, days).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub limit: Option<i64>,
}

/// GET /api/knowledge/search — every documentation store, one ranked list.
/// A secret appears as its record only; no value and no sealed field is read.
#[utoipa::path(operation_id = "knowledge_search", get, path = "/api/knowledge/search", tag = "knowledge", security(("bearer" = [])), 
    params(("q" = String, Query), ("limit" = Option<i64>, Query)),
    responses((status = 200, body = [kh::KnowledgeSearchHit])))]
pub async fn search(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<SearchQuery>,
) -> ApiResult<Json<Vec<kh::KnowledgeSearchHit>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let needle = q.q.trim();
    if needle.len() < 2 {
        return Ok(Json(Vec::new()));
    }
    // Escape LIKE metacharacters so a search for "100%" is a search, not a
    // wildcard.
    let escaped = needle
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let pattern = format!("%{escaped}%");
    let limit = q.limit.unwrap_or(20).clamp(1, 50);
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = kh::search(&mut *tx, org_id, &pattern, limit).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/* ------------------------------------------------------------------------- */
/* Racks                                                                     */
/* ------------------------------------------------------------------------- */

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateRackRequest {
    pub client_id: Uuid,
    pub site_id: Option<Uuid>,
    pub name: String,
    pub code: Option<String>,
    #[serde(default = "default_units")]
    pub units: i16,
    pub width_mm: Option<i32>,
    pub depth_mm: Option<i32>,
    pub power_capacity_kw: Option<f64>,
    pub power_used_kw: Option<f64>,
    pub cooling: Option<String>,
    pub location_note: Option<String>,
    pub notes: Option<String>,
    pub fields: Option<Value>,
}

fn default_units() -> i16 {
    42
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateRackRequest {
    pub site_id: Option<Uuid>,
    pub name: String,
    pub code: Option<String>,
    #[serde(default = "default_units")]
    pub units: i16,
    pub width_mm: Option<i32>,
    pub depth_mm: Option<i32>,
    pub power_capacity_kw: Option<f64>,
    pub power_used_kw: Option<f64>,
    pub cooling: Option<String>,
    pub location_note: Option<String>,
    pub notes: Option<String>,
    pub fields: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct ClientScopedQuery {
    pub client_id: Uuid,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RackDetailResponse {
    #[serde(flatten)]
    pub rack: kh::RackRecord,
    pub devices: Vec<kh::RackDeviceRecord>,
    pub links: Vec<kh::LinkRecord>,
    pub used_units: i16,
}

/// GET /api/knowledge/racks?client_id=
#[utoipa::path(operation_id = "knowledge_list_racks", get, path = "/api/knowledge/racks?client_id=", tag = "knowledge", security(("bearer" = [])), 
    params(("client_id" = Uuid, Query)),
    responses((status = 200, body = [kh::RackRecord])))]
pub async fn list_racks(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ClientScopedQuery>,
) -> ApiResult<Json<Vec<kh::RackRecord>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = kh::list_racks(&mut *tx, org_id, q.client_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// GET /api/knowledge/racks/{id} — the elevation and its placements.
#[utoipa::path(operation_id = "knowledge_get_rack", get, path = "/api/knowledge/racks/{id}", tag = "knowledge", security(("bearer" = [])), 
    params(("id" = Uuid, Path)),
    responses((status = 200, body = RackDetailResponse)))]
pub async fn get_rack(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<RackDetailResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rack = kh::get_rack(&mut *tx, org_id, id).await?;
    let devices = kh::list_rack_devices(&mut *tx, org_id, id).await?;
    let links = kh::links_for(&mut *tx, org_id, "rack", id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    let used_units: i16 = devices
        .iter()
        .filter(|d| d.face == "front")
        .map(|d| d.unit_height)
        .sum();
    Ok(Json(RackDetailResponse {
        rack,
        devices,
        links,
        used_units,
    }))
}

/// POST /api/knowledge/racks
#[utoipa::path(operation_id = "knowledge_create_rack", post, path = "/api/knowledge/racks", tag = "knowledge", security(("bearer" = [])), 
    request_body = CreateRackRequest,
    responses((status = 200, body = kh::RackRecord)))]
pub async fn create_rack(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateRackRequest>,
) -> ApiResult<Json<kh::RackRecord>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let name = trimmed("name", &req.name)?;
    if !(4..=58).contains(&req.units) {
        return Err(ApiError::Validation("units must be 4 to 58".into()));
    }
    let fields = req
        .fields
        .unwrap_or_else(|| Value::Object(Default::default()));
    let id = Uuid::now_v7();
    let rack = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let rack = kh::insert_rack(
            &mut *ctx.tx,
            org_id,
            id,
            req.client_id,
            req.site_id,
            &name,
            req.code.as_deref(),
            req.units,
            req.width_mm,
            req.depth_mm,
            req.power_capacity_kw,
            req.power_used_kw,
            req.cooling.as_deref(),
            req.location_note.as_deref(),
            req.notes.as_deref(),
            &fields,
            user_id,
        )
        .await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.rack.created",
            "knowledge_rack",
            id,
            serde_json::json!({ "client_id": req.client_id, "name": name }),
        )
        .await?;
        Ok((ctx, rack))
    })
    .await?;
    Ok(Json(rack))
}

/// PATCH /api/knowledge/racks/{id}
#[utoipa::path(operation_id = "knowledge_update_rack", patch, path = "/api/knowledge/racks/{id}", tag = "knowledge", security(("bearer" = [])), 
    params(("id" = Uuid, Path)),
    request_body = UpdateRackRequest,
    responses((status = 200, body = kh::RackRecord)))]
pub async fn update_rack(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateRackRequest>,
) -> ApiResult<Json<kh::RackRecord>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let name = trimmed("name", &req.name)?;
    if !(4..=58).contains(&req.units) {
        return Err(ApiError::Validation("units must be 4 to 58".into()));
    }
    let fields = req
        .fields
        .unwrap_or_else(|| Value::Object(Default::default()));
    let rack = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let rack = kh::update_rack(
            &mut *ctx.tx,
            org_id,
            id,
            req.site_id,
            &name,
            req.code.as_deref(),
            req.units,
            req.width_mm,
            req.depth_mm,
            req.power_capacity_kw,
            req.power_used_kw,
            req.cooling.as_deref(),
            req.location_note.as_deref(),
            req.notes.as_deref(),
            &fields,
            user_id,
        )
        .await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.rack.updated",
            "knowledge_rack",
            id,
            serde_json::json!({ "name": name }),
        )
        .await?;
        Ok((ctx, rack))
    })
    .await?;
    Ok(Json(rack))
}

/// DELETE /api/knowledge/racks/{id}
#[utoipa::path(operation_id = "knowledge_delete_rack", delete, path = "/api/knowledge/racks/{id}", tag = "knowledge", security(("bearer" = [])), 
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Object)))]
pub async fn delete_rack(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        kh::delete_rack(&mut *ctx.tx, org_id, id).await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.rack.deleted",
            "knowledge_rack",
            id,
            Value::Null,
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(Json(serde_json::json!({ "deleted": id })))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RackDeviceRequest {
    pub asset_id: Option<Uuid>,
    pub name: String,
    pub device_type: String,
    pub face: String,
    pub unit_start: i16,
    #[serde(default = "default_height")]
    pub unit_height: i16,
    pub model: Option<String>,
    pub serial: Option<String>,
    #[serde(default = "default_device_status")]
    pub status: String,
    pub power_notes: Option<String>,
    pub network_notes: Option<String>,
}

fn default_height() -> i16 {
    1
}

fn default_device_status() -> String {
    "online".into()
}

/// POST /api/knowledge/racks/{id}/devices — place a device in the elevation.
#[utoipa::path(operation_id = "knowledge_add_rack_device", post, path = "/api/knowledge/racks/{id}/devices", tag = "knowledge", security(("bearer" = [])), 
    params(("id" = Uuid, Path)),
    request_body = RackDeviceRequest,
    responses((status = 200, body = kh::RackDeviceRecord)))]
pub async fn add_rack_device(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<RackDeviceRequest>,
) -> ApiResult<Json<kh::RackDeviceRecord>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let name = trimmed("name", &req.name)?;
    one_of(
        "device_type",
        &req.device_type,
        &["network", "compute", "storage", "power", "patch", "other"],
    )?;
    one_of("face", &req.face, &["front", "rear"])?;
    one_of(
        "status",
        &req.status,
        &["online", "attention", "offline", "passive"],
    )?;
    if req.unit_start < 1 || !(1..=12).contains(&req.unit_height) {
        return Err(ApiError::Validation(
            "unit_start must be 1 or more and unit_height 1 to 12".into(),
        ));
    }
    let new_id = Uuid::now_v7();
    let device = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let rack = kh::get_rack(&mut *ctx.tx, org_id, id).await?;
        let top = req.unit_start + req.unit_height - 1;
        if top > rack.units {
            return Err(ApiError::Validation(format!(
                "U{} to U{} does not fit a {}U rack",
                req.unit_start, top, rack.units
            )));
        }
        if kh::rack_slot_taken(
            &mut *ctx.tx,
            org_id,
            id,
            &req.face,
            req.unit_start,
            req.unit_height,
            None,
        )
        .await?
        {
            return Err(ApiError::Conflict(
                "another device already occupies that part of the rack".into(),
            ));
        }
        let device = kh::insert_rack_device(
            &mut *ctx.tx,
            org_id,
            new_id,
            id,
            req.asset_id,
            &name,
            &req.device_type,
            &req.face,
            req.unit_start,
            req.unit_height,
            req.model.as_deref(),
            req.serial.as_deref(),
            &req.status,
            req.power_notes.as_deref(),
            req.network_notes.as_deref(),
        )
        .await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.rack_device.created",
            "knowledge_rack_device",
            new_id,
            serde_json::json!({ "rack_id": id, "unit_start": req.unit_start }),
        )
        .await?;
        Ok((ctx, device))
    })
    .await?;
    Ok(Json(device))
}

/// PATCH /api/knowledge/racks/{id}/devices/{device_id}
#[utoipa::path(operation_id = "knowledge_update_rack_device", patch, path = "/api/knowledge/racks/{id}/devices/{device_id}", tag = "knowledge", security(("bearer" = [])), 
    request_body = RackDeviceRequest,
    responses((status = 200, body = kh::RackDeviceRecord)))]
pub async fn update_rack_device(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, device_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<RackDeviceRequest>,
) -> ApiResult<Json<kh::RackDeviceRecord>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let name = trimmed("name", &req.name)?;
    one_of(
        "device_type",
        &req.device_type,
        &["network", "compute", "storage", "power", "patch", "other"],
    )?;
    one_of("face", &req.face, &["front", "rear"])?;
    one_of(
        "status",
        &req.status,
        &["online", "attention", "offline", "passive"],
    )?;
    let device = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let rack = kh::get_rack(&mut *ctx.tx, org_id, id).await?;
        let top = req.unit_start + req.unit_height - 1;
        if top > rack.units {
            return Err(ApiError::Validation(format!(
                "U{} to U{} does not fit a {}U rack",
                req.unit_start, top, rack.units
            )));
        }
        if kh::rack_slot_taken(
            &mut *ctx.tx,
            org_id,
            id,
            &req.face,
            req.unit_start,
            req.unit_height,
            Some(device_id),
        )
        .await?
        {
            return Err(ApiError::Conflict(
                "another device already occupies that part of the rack".into(),
            ));
        }
        let device = kh::update_rack_device(
            &mut *ctx.tx,
            org_id,
            device_id,
            req.asset_id,
            &name,
            &req.device_type,
            &req.face,
            req.unit_start,
            req.unit_height,
            req.model.as_deref(),
            req.serial.as_deref(),
            &req.status,
            req.power_notes.as_deref(),
            req.network_notes.as_deref(),
        )
        .await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.rack_device.updated",
            "knowledge_rack_device",
            device_id,
            serde_json::json!({ "rack_id": id }),
        )
        .await?;
        Ok((ctx, device))
    })
    .await?;
    Ok(Json(device))
}

/// DELETE /api/knowledge/racks/{id}/devices/{device_id}
#[utoipa::path(operation_id = "knowledge_delete_rack_device", delete, path = "/api/knowledge/racks/{id}/devices/{device_id}", tag = "knowledge", security(("bearer" = [])), 
    responses((status = 200, body = Object)))]
pub async fn delete_rack_device(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, device_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        kh::delete_rack_device(&mut *ctx.tx, org_id, device_id).await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.rack_device.deleted",
            "knowledge_rack_device",
            device_id,
            serde_json::json!({ "rack_id": id }),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(Json(serde_json::json!({ "deleted": device_id })))
}

/* ------------------------------------------------------------------------- */
/* Networks                                                                  */
/* ------------------------------------------------------------------------- */

#[derive(Debug, Deserialize, ToSchema)]
pub struct NetworkRequest {
    pub client_id: Option<Uuid>,
    pub site_id: Option<Uuid>,
    pub name: String,
    #[serde(default = "default_network_kind")]
    pub kind: String,
    pub gateway: Option<String>,
    pub dns: Option<String>,
    pub vlan_zone: Option<String>,
    pub notes: Option<String>,
    pub fields: Option<Value>,
}

fn default_network_kind() -> String {
    "lan".into()
}

#[derive(Debug, Serialize, ToSchema)]
pub struct NetworkDetailResponse {
    #[serde(flatten)]
    pub network: kh::NetworkRecord,
    pub subnets: Vec<kh::SubnetRecord>,
    pub reservations: Vec<kh::IpReservationRecord>,
    pub links: Vec<kh::LinkRecord>,
}

/// GET /api/knowledge/networks?client_id=
#[utoipa::path(operation_id = "knowledge_list_networks", get, path = "/api/knowledge/networks?client_id=", tag = "knowledge", security(("bearer" = [])), 
    params(("client_id" = Uuid, Query)),
    responses((status = 200, body = [kh::NetworkRecord])))]
pub async fn list_networks(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ClientScopedQuery>,
) -> ApiResult<Json<Vec<kh::NetworkRecord>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = kh::list_networks(&mut *tx, org_id, q.client_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// GET /api/knowledge/networks/{id}
#[utoipa::path(operation_id = "knowledge_get_network", get, path = "/api/knowledge/networks/{id}", tag = "knowledge", security(("bearer" = [])), 
    params(("id" = Uuid, Path)),
    responses((status = 200, body = NetworkDetailResponse)))]
pub async fn get_network(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<NetworkDetailResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let network = kh::get_network(&mut *tx, org_id, id).await?;
    let subnets = kh::list_subnets(&mut *tx, org_id, id).await?;
    let mut reservations = Vec::new();
    for subnet in &subnets {
        reservations.extend(kh::list_reservations(&mut *tx, org_id, subnet.id).await?);
    }
    let links = kh::links_for(&mut *tx, org_id, "network", id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(NetworkDetailResponse {
        network,
        subnets,
        reservations,
        links,
    }))
}

/// POST /api/knowledge/networks
#[utoipa::path(operation_id = "knowledge_create_network", post, path = "/api/knowledge/networks", tag = "knowledge", security(("bearer" = [])), 
    request_body = NetworkRequest,
    responses((status = 200, body = kh::NetworkRecord)))]
pub async fn create_network(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<NetworkRequest>,
) -> ApiResult<Json<kh::NetworkRecord>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let client_id = req
        .client_id
        .ok_or_else(|| ApiError::Validation("client_id is required".into()))?;
    let name = trimmed("name", &req.name)?;
    one_of(
        "kind",
        &req.kind,
        &["lan", "wan", "wifi", "vpn", "voice", "guest", "other"],
    )?;
    let fields = req
        .fields
        .unwrap_or_else(|| Value::Object(Default::default()));
    let id = Uuid::now_v7();
    let network = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let network = kh::insert_network(
            &mut *ctx.tx,
            org_id,
            id,
            client_id,
            req.site_id,
            &name,
            &req.kind,
            req.gateway.as_deref(),
            req.dns.as_deref(),
            req.vlan_zone.as_deref(),
            req.notes.as_deref(),
            &fields,
            user_id,
        )
        .await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.network.created",
            "knowledge_network",
            id,
            serde_json::json!({ "client_id": client_id, "name": name }),
        )
        .await?;
        Ok((ctx, network))
    })
    .await?;
    Ok(Json(network))
}

/// PATCH /api/knowledge/networks/{id}
#[utoipa::path(operation_id = "knowledge_update_network", patch, path = "/api/knowledge/networks/{id}", tag = "knowledge", security(("bearer" = [])), 
    params(("id" = Uuid, Path)),
    request_body = NetworkRequest,
    responses((status = 200, body = kh::NetworkRecord)))]
pub async fn update_network(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<NetworkRequest>,
) -> ApiResult<Json<kh::NetworkRecord>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let name = trimmed("name", &req.name)?;
    one_of(
        "kind",
        &req.kind,
        &["lan", "wan", "wifi", "vpn", "voice", "guest", "other"],
    )?;
    let fields = req
        .fields
        .unwrap_or_else(|| Value::Object(Default::default()));
    let network = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let network = kh::update_network(
            &mut *ctx.tx,
            org_id,
            id,
            req.site_id,
            &name,
            &req.kind,
            req.gateway.as_deref(),
            req.dns.as_deref(),
            req.vlan_zone.as_deref(),
            req.notes.as_deref(),
            &fields,
            user_id,
        )
        .await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.network.updated",
            "knowledge_network",
            id,
            serde_json::json!({ "name": name }),
        )
        .await?;
        Ok((ctx, network))
    })
    .await?;
    Ok(Json(network))
}

/// DELETE /api/knowledge/networks/{id}
#[utoipa::path(operation_id = "knowledge_delete_network", delete, path = "/api/knowledge/networks/{id}", tag = "knowledge", security(("bearer" = [])), 
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Object)))]
pub async fn delete_network(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        kh::delete_network(&mut *ctx.tx, org_id, id).await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.network.deleted",
            "knowledge_network",
            id,
            Value::Null,
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(Json(serde_json::json!({ "deleted": id })))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SubnetRequest {
    pub cidr: String,
    pub vlan_id: Option<i32>,
    pub name: Option<String>,
    pub gateway: Option<String>,
    pub used_for: Option<String>,
    #[serde(default = "default_true")]
    pub dhcp: bool,
    pub notes: Option<String>,
}

fn default_true() -> bool {
    true
}

/// POST /api/knowledge/networks/{id}/subnets
#[utoipa::path(operation_id = "knowledge_add_subnet", post, path = "/api/knowledge/networks/{id}/subnets", tag = "knowledge", security(("bearer" = [])), 
    params(("id" = Uuid, Path)),
    request_body = SubnetRequest,
    responses((status = 200, body = kh::SubnetRecord)))]
pub async fn add_subnet(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<SubnetRequest>,
) -> ApiResult<Json<kh::SubnetRecord>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let cidr = trimmed("cidr", &req.cidr)?;
    if let Some(vlan) = req.vlan_id {
        if !(1..=4094).contains(&vlan) {
            return Err(ApiError::Validation("vlan_id must be 1 to 4094".into()));
        }
    }
    let new_id = Uuid::now_v7();
    let subnet = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let network = kh::get_network(&mut *ctx.tx, org_id, id).await?;
        let subnet = kh::insert_subnet(
            &mut *ctx.tx,
            org_id,
            new_id,
            network.client_id,
            id,
            &cidr,
            req.vlan_id,
            req.name.as_deref(),
            req.gateway.as_deref(),
            req.used_for.as_deref(),
            req.dhcp,
            req.notes.as_deref(),
        )
        .await
        .map_err(|e| match e {
            core_db::DbError::Sql(sqlx::Error::Database(db)) if db.constraint().is_some() => {
                ApiError::Validation(format!("{cidr} is not a valid subnet"))
            }
            other => other.into(),
        })?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.subnet.created",
            "knowledge_subnet",
            new_id,
            serde_json::json!({ "network_id": id, "cidr": cidr }),
        )
        .await?;
        Ok((ctx, subnet))
    })
    .await?;
    Ok(Json(subnet))
}

/// DELETE /api/knowledge/subnets/{id}
#[utoipa::path(operation_id = "knowledge_delete_subnet", delete, path = "/api/knowledge/subnets/{id}", tag = "knowledge", security(("bearer" = [])), 
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Object)))]
pub async fn delete_subnet(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        kh::delete_subnet(&mut *ctx.tx, org_id, id).await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.subnet.deleted",
            "knowledge_subnet",
            id,
            Value::Null,
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(Json(serde_json::json!({ "deleted": id })))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReservationRequest {
    pub address: String,
    pub hostname: Option<String>,
    pub asset_id: Option<Uuid>,
    #[serde(default = "default_reservation_status")]
    pub status: String,
    pub notes: Option<String>,
}

fn default_reservation_status() -> String {
    "assigned".into()
}

/// POST /api/knowledge/subnets/{id}/reservations
#[utoipa::path(operation_id = "knowledge_add_reservation", post, path = "/api/knowledge/subnets/{id}/reservations", tag = "knowledge", security(("bearer" = [])), 
    params(("id" = Uuid, Path)),
    request_body = ReservationRequest,
    responses((status = 200, body = kh::IpReservationRecord)))]
pub async fn add_reservation(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<ReservationRequest>,
) -> ApiResult<Json<kh::IpReservationRecord>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let address = trimmed("address", &req.address)?;
    one_of(
        "status",
        &req.status,
        &["assigned", "reserved", "deprecated"],
    )?;
    let new_id = Uuid::now_v7();
    let reservation = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let client_id: Uuid = sqlx::query_scalar(
            "SELECT client_id FROM knowledge_subnets WHERE org_id = $1 AND id = $2",
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&mut *ctx.tx)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
        let reservation = kh::insert_reservation(
            &mut *ctx.tx,
            org_id,
            new_id,
            client_id,
            id,
            &address,
            req.hostname.as_deref(),
            req.asset_id,
            &req.status,
            req.notes.as_deref(),
        )
        .await
        .map_err(|e| match e {
            core_db::DbError::Sql(sqlx::Error::Database(db)) if db.constraint().is_some() => {
                ApiError::Validation(format!("{address} is not a valid address"))
            }
            other => other.into(),
        })?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.ip_reservation.created",
            "knowledge_ip_reservation",
            new_id,
            serde_json::json!({ "subnet_id": id, "address": address }),
        )
        .await?;
        Ok((ctx, reservation))
    })
    .await?;
    Ok(Json(reservation))
}

/// DELETE /api/knowledge/reservations/{id}
#[utoipa::path(operation_id = "knowledge_delete_reservation", delete, path = "/api/knowledge/reservations/{id}", tag = "knowledge", security(("bearer" = [])), 
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Object)))]
pub async fn delete_reservation(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        kh::delete_reservation(&mut *ctx.tx, org_id, id).await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.ip_reservation.deleted",
            "knowledge_ip_reservation",
            id,
            Value::Null,
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(Json(serde_json::json!({ "deleted": id })))
}

/* ------------------------------------------------------------------------- */
/* Domains                                                                   */
/* ------------------------------------------------------------------------- */

#[derive(Debug, Deserialize, ToSchema)]
pub struct DomainRequest {
    pub client_id: Option<Uuid>,
    pub domain: String,
    #[serde(default = "default_domain_kind")]
    pub kind: String,
    pub registrar: Option<String>,
    pub registrar_expiry: Option<NaiveDate>,
    pub ssl_expiry: Option<NaiveDate>,
    pub ssl_issuer: Option<String>,
    pub dns_provider: Option<String>,
    pub hosting: Option<String>,
    #[serde(default = "default_true")]
    pub manages_dns: bool,
    #[serde(default)]
    pub monitoring: bool,
    pub status_url: Option<String>,
    pub notes: Option<String>,
    pub fields: Option<Value>,
}

fn default_domain_kind() -> String {
    "domain".into()
}

#[derive(Debug, Deserialize)]
pub struct DomainListQuery {
    pub client_id: Option<Uuid>,
}

/// GET /api/knowledge/domains?client_id= — org-wide when no client is named.
#[utoipa::path(operation_id = "knowledge_list_domains", get, path = "/api/knowledge/domains?client_id=", tag = "knowledge", security(("bearer" = [])), 
    params(("client_id" = Option<Uuid>, Query)),
    responses((status = 200, body = [kh::DomainRecord])))]
pub async fn list_domains(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<DomainListQuery>,
) -> ApiResult<Json<Vec<kh::DomainRecord>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = match q.client_id {
        Some(client_id) => kh::list_domains(&mut *tx, org_id, client_id).await?,
        None => kh::list_domains_all(&mut *tx, org_id).await?,
    };
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// POST /api/knowledge/domains
#[utoipa::path(operation_id = "knowledge_create_domain", post, path = "/api/knowledge/domains", tag = "knowledge", security(("bearer" = [])), 
    request_body = DomainRequest,
    responses((status = 200, body = kh::DomainRecord)))]
pub async fn create_domain(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<DomainRequest>,
) -> ApiResult<Json<kh::DomainRecord>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let client_id = req
        .client_id
        .ok_or_else(|| ApiError::Validation("client_id is required".into()))?;
    let domain = trimmed("domain", &req.domain)?;
    one_of("kind", &req.kind, &["domain", "website"])?;
    let fields = req
        .fields
        .unwrap_or_else(|| Value::Object(Default::default()));
    let id = Uuid::now_v7();
    let record = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let record = kh::insert_domain(
            &mut *ctx.tx,
            org_id,
            id,
            client_id,
            &domain,
            &req.kind,
            req.registrar.as_deref(),
            req.registrar_expiry,
            req.ssl_expiry,
            req.ssl_issuer.as_deref(),
            req.dns_provider.as_deref(),
            req.hosting.as_deref(),
            req.manages_dns,
            req.monitoring,
            req.status_url.as_deref(),
            req.notes.as_deref(),
            &fields,
            user_id,
        )
        .await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.domain.created",
            "knowledge_domain",
            id,
            serde_json::json!({ "client_id": client_id, "domain": domain }),
        )
        .await?;
        Ok((ctx, record))
    })
    .await?;
    Ok(Json(record))
}

/// PATCH /api/knowledge/domains/{id}
#[utoipa::path(operation_id = "knowledge_update_domain", patch, path = "/api/knowledge/domains/{id}", tag = "knowledge", security(("bearer" = [])), 
    params(("id" = Uuid, Path)),
    request_body = DomainRequest,
    responses((status = 200, body = kh::DomainRecord)))]
pub async fn update_domain(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<DomainRequest>,
) -> ApiResult<Json<kh::DomainRecord>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let domain = trimmed("domain", &req.domain)?;
    one_of("kind", &req.kind, &["domain", "website"])?;
    let fields = req
        .fields
        .unwrap_or_else(|| Value::Object(Default::default()));
    let record = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let record = kh::update_domain(
            &mut *ctx.tx,
            org_id,
            id,
            &domain,
            &req.kind,
            req.registrar.as_deref(),
            req.registrar_expiry,
            req.ssl_expiry,
            req.ssl_issuer.as_deref(),
            req.dns_provider.as_deref(),
            req.hosting.as_deref(),
            req.manages_dns,
            req.monitoring,
            req.status_url.as_deref(),
            req.notes.as_deref(),
            &fields,
            user_id,
        )
        .await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.domain.updated",
            "knowledge_domain",
            id,
            serde_json::json!({ "domain": domain }),
        )
        .await?;
        Ok((ctx, record))
    })
    .await?;
    Ok(Json(record))
}

/// DELETE /api/knowledge/domains/{id}
#[utoipa::path(operation_id = "knowledge_delete_domain", delete, path = "/api/knowledge/domains/{id}", tag = "knowledge", security(("bearer" = [])), 
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Object)))]
pub async fn delete_domain(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        kh::delete_domain(&mut *ctx.tx, org_id, id).await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.domain.deleted",
            "knowledge_domain",
            id,
            Value::Null,
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(Json(serde_json::json!({ "deleted": id })))
}

/* ------------------------------------------------------------------------- */
/* Procedures                                                                */
/* ------------------------------------------------------------------------- */

#[derive(Debug, Deserialize, ToSchema)]
pub struct ProcedureStepInput {
    pub title: String,
    pub body: Option<String>,
    #[serde(default = "default_true")]
    pub required: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ProcedureRequest {
    pub client_id: Option<Uuid>,
    pub title: String,
    pub summary: Option<String>,
    #[serde(default)]
    pub body: String,
    pub owner_user_id: Option<Uuid>,
    #[serde(default = "default_true")]
    pub is_checklist: bool,
    #[serde(default)]
    pub client_visible: bool,
    pub fields: Option<Value>,
    #[serde(default)]
    pub steps: Vec<ProcedureStepInput>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProcedureDetailResponse {
    #[serde(flatten)]
    pub procedure: kh::ProcedureRecord,
    pub steps: Vec<kh::ProcedureStepRecord>,
    pub links: Vec<kh::LinkRecord>,
}

/// GET /api/knowledge/procedures?client_id=
#[utoipa::path(operation_id = "knowledge_list_procedures", get, path = "/api/knowledge/procedures?client_id=", tag = "knowledge", security(("bearer" = [])), 
    params(("client_id" = Uuid, Query)),
    responses((status = 200, body = [kh::ProcedureRecord])))]
pub async fn list_procedures(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ClientScopedQuery>,
) -> ApiResult<Json<Vec<kh::ProcedureRecord>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = kh::list_procedures(&mut *tx, org_id, q.client_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// GET /api/knowledge/procedures/{id}
#[utoipa::path(operation_id = "knowledge_get_procedure", get, path = "/api/knowledge/procedures/{id}", tag = "knowledge", security(("bearer" = [])), 
    params(("id" = Uuid, Path)),
    responses((status = 200, body = ProcedureDetailResponse)))]
pub async fn get_procedure(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ProcedureDetailResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let procedure = kh::get_procedure(&mut *tx, org_id, id).await?;
    let steps = kh::list_procedure_steps(&mut *tx, org_id, id).await?;
    let links = kh::links_for(&mut *tx, org_id, "procedure", id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(ProcedureDetailResponse {
        procedure,
        steps,
        links,
    }))
}

fn step_tuples(steps: &[ProcedureStepInput]) -> Vec<(String, Option<String>, bool)> {
    steps
        .iter()
        .map(|s| (s.title.trim().to_string(), s.body.clone(), s.required))
        .collect()
}

/// POST /api/knowledge/procedures
#[utoipa::path(operation_id = "knowledge_create_procedure", post, path = "/api/knowledge/procedures", tag = "knowledge", security(("bearer" = [])), 
    request_body = ProcedureRequest,
    responses((status = 200, body = ProcedureDetailResponse)))]
pub async fn create_procedure(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<ProcedureRequest>,
) -> ApiResult<Json<ProcedureDetailResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let client_id = req
        .client_id
        .ok_or_else(|| ApiError::Validation("client_id is required".into()))?;
    let title = trimmed("title", &req.title)?;
    let fields = req
        .fields
        .unwrap_or_else(|| Value::Object(Default::default()));
    let steps = step_tuples(&req.steps);
    let id = Uuid::now_v7();
    let detail = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let procedure = kh::insert_procedure(
            &mut *ctx.tx,
            org_id,
            id,
            client_id,
            &title,
            req.summary.as_deref(),
            &req.body,
            req.owner_user_id,
            req.is_checklist,
            req.client_visible,
            &fields,
            user_id,
        )
        .await?;
        let steps = kh::replace_procedure_steps(&mut *ctx.tx, org_id, id, &steps).await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.procedure.created",
            "knowledge_procedure",
            id,
            serde_json::json!({ "client_id": client_id, "title": title }),
        )
        .await?;
        Ok((ctx, (procedure, steps)))
    })
    .await?;
    Ok(Json(ProcedureDetailResponse {
        procedure: detail.0,
        steps: detail.1,
        links: Vec::new(),
    }))
}

/// PATCH /api/knowledge/procedures/{id}
#[utoipa::path(operation_id = "knowledge_update_procedure", patch, path = "/api/knowledge/procedures/{id}", tag = "knowledge", security(("bearer" = [])), 
    params(("id" = Uuid, Path)),
    request_body = ProcedureRequest,
    responses((status = 200, body = ProcedureDetailResponse)))]
pub async fn update_procedure(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<ProcedureRequest>,
) -> ApiResult<Json<ProcedureDetailResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let title = trimmed("title", &req.title)?;
    let fields = req
        .fields
        .unwrap_or_else(|| Value::Object(Default::default()));
    let steps = step_tuples(&req.steps);
    let detail = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let procedure = kh::update_procedure(
            &mut *ctx.tx,
            org_id,
            id,
            &title,
            req.summary.as_deref(),
            &req.body,
            req.owner_user_id,
            req.is_checklist,
            req.client_visible,
            &fields,
            user_id,
        )
        .await?;
        let steps = kh::replace_procedure_steps(&mut *ctx.tx, org_id, id, &steps).await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.procedure.updated",
            "knowledge_procedure",
            id,
            serde_json::json!({ "title": title, "version": procedure.version }),
        )
        .await?;
        Ok((ctx, (procedure, steps)))
    })
    .await?;
    Ok(Json(ProcedureDetailResponse {
        procedure: detail.0,
        steps: detail.1,
        links: Vec::new(),
    }))
}

/// DELETE /api/knowledge/procedures/{id}
#[utoipa::path(operation_id = "knowledge_delete_procedure", delete, path = "/api/knowledge/procedures/{id}", tag = "knowledge", security(("bearer" = [])), 
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Object)))]
pub async fn delete_procedure(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        kh::delete_procedure(&mut *ctx.tx, org_id, id).await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.procedure.deleted",
            "knowledge_procedure",
            id,
            Value::Null,
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(Json(serde_json::json!({ "deleted": id })))
}

/* ------------------------------------------------------------------------- */
/* Links, hub settings, document visibility                                  */
/* ------------------------------------------------------------------------- */

#[derive(Debug, Deserialize)]
pub struct LinkQuery {
    pub kind: String,
    pub id: Uuid,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateKnowledgeLinkRequest {
    pub client_id: Option<Uuid>,
    pub from_kind: String,
    pub from_id: Uuid,
    pub to_kind: String,
    pub to_id: Uuid,
    pub label: Option<String>,
}

const LINK_KINDS: &[&str] = &[
    "rack",
    "rack_device",
    "network",
    "subnet",
    "domain",
    "procedure",
    "document",
    "kb_article",
    "asset",
    "secret",
    "ticket",
    "site",
];

/// GET /api/knowledge/links?kind=&id=
#[utoipa::path(operation_id = "knowledge_list_links", get, path = "/api/knowledge/links?kind=&id=", tag = "knowledge", security(("bearer" = [])), 
    params(("kind" = String, Query), ("id" = Uuid, Query)),
    responses((status = 200, body = [kh::LinkRecord])))]
pub async fn list_links(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<LinkQuery>,
) -> ApiResult<Json<Vec<kh::LinkRecord>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::View))?;
    one_of("kind", &q.kind, LINK_KINDS)?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = kh::links_for(&mut *tx, org_id, &q.kind, q.id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// POST /api/knowledge/links
#[utoipa::path(operation_id = "knowledge_create_link", post, path = "/api/knowledge/links", tag = "knowledge", security(("bearer" = [])), 
    request_body = CreateKnowledgeLinkRequest,
    responses((status = 200, body = kh::LinkRecord)))]
pub async fn create_link(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateKnowledgeLinkRequest>,
) -> ApiResult<Json<kh::LinkRecord>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Edit))?;
    one_of("from_kind", &req.from_kind, LINK_KINDS)?;
    one_of("to_kind", &req.to_kind, LINK_KINDS)?;
    if req.from_kind == req.to_kind && req.from_id == req.to_id {
        return Err(ApiError::Validation(
            "a record cannot link to itself".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let id = Uuid::now_v7();
    let link = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let link = kh::insert_link(
            &mut *ctx.tx,
            org_id,
            id,
            req.client_id,
            &req.from_kind,
            req.from_id,
            &req.to_kind,
            req.to_id,
            req.label.as_deref(),
            user_id,
        )
        .await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.link.created",
            "knowledge_link",
            id,
            serde_json::json!({
                "from_kind": req.from_kind, "from_id": req.from_id,
                "to_kind": req.to_kind, "to_id": req.to_id
            }),
        )
        .await?;
        Ok((ctx, link))
    })
    .await?;
    Ok(Json(link))
}

/// DELETE /api/knowledge/links/{id}
#[utoipa::path(operation_id = "knowledge_delete_link", delete, path = "/api/knowledge/links/{id}", tag = "knowledge", security(("bearer" = [])), 
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Object)))]
pub async fn delete_link(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        kh::delete_link(&mut *ctx.tx, org_id, id).await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.link.deleted",
            "knowledge_link",
            id,
            Value::Null,
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(Json(serde_json::json!({ "deleted": id })))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ClientSettingsRequest {
    pub owner_user_id: Option<Uuid>,
    #[serde(default)]
    pub pinned_document_ids: Vec<Uuid>,
    pub review_note: Option<String>,
}

/// GET /api/knowledge/clients/{client_id}/settings
#[utoipa::path(operation_id = "knowledge_get_client_settings", get, path = "/api/knowledge/clients/{client_id}/settings", tag = "knowledge", security(("bearer" = [])), 
    params(("client_id" = Uuid, Path)),
    responses((status = 200, body = kh::ClientSettingsRecord)))]
pub async fn get_client_settings(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(client_id): Path<Uuid>,
) -> ApiResult<Json<Option<kh::ClientSettingsRecord>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let row = kh::get_client_settings(&mut *tx, org_id, client_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(row))
}

/// PUT /api/knowledge/clients/{client_id}/settings — owner, pinned documents
/// and the review note the next technician reads first.
#[utoipa::path(operation_id = "knowledge_put_client_settings", put, path = "/api/knowledge/clients/{client_id}/settings", tag = "knowledge", security(("bearer" = [])), 
    params(("client_id" = Uuid, Path)),
    request_body = ClientSettingsRequest,
    responses((status = 200, body = kh::ClientSettingsRecord)))]
pub async fn put_client_settings(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(client_id): Path<Uuid>,
    Json(req): Json<ClientSettingsRequest>,
) -> ApiResult<Json<kh::ClientSettingsRecord>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let row = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let row = kh::upsert_client_settings(
            &mut *ctx.tx,
            org_id,
            client_id,
            req.owner_user_id,
            &req.pinned_document_ids,
            req.review_note.as_deref(),
            user_id,
        )
        .await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.client_settings.updated",
            "knowledge_client_settings",
            client_id,
            serde_json::json!({ "owner_user_id": req.owner_user_id }),
        )
        .await?;
        Ok((ctx, row))
    })
    .await?;
    Ok(Json(row))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DocumentVisibilityRequest {
    pub client_visible: bool,
}

/// PUT /api/knowledge/documents/{id}/visibility — the staff-side switch that
/// decides whether the client portal may show this document.
#[utoipa::path(operation_id = "knowledge_put_document_visibility", put, path = "/api/knowledge/documents/{id}/visibility", tag = "knowledge", security(("bearer" = [])), 
    params(("id" = Uuid, Path)),
    request_body = DocumentVisibilityRequest,
    responses((status = 200, body = Object)))]
pub async fn put_document_visibility(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<DocumentVisibilityRequest>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let (client_id, visible) = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let (client_id, _) = kh::document_visibility(&mut *ctx.tx, org_id, id).await?;
        kh::set_document_visibility(&mut *ctx.tx, org_id, id, req.client_visible, user_id).await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.document.visibility_changed",
            "document",
            id,
            serde_json::json!({ "client_visible": req.client_visible }),
        )
        .await?;
        Ok((ctx, (client_id, req.client_visible)))
    })
    .await?;
    Ok(Json(serde_json::json!({
        "document_id": id,
        "client_id": client_id,
        "client_visible": visible
    })))
}

/* ------------------------------------------------------------------------- */
/* Templates                                                                 */
/* ------------------------------------------------------------------------- */

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateKnowledgeTemplateRequest {
    pub client_id: Option<Uuid>,
    pub kind: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub body: String,
    pub fields: Option<Value>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateKnowledgeTemplateRequest {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub body: String,
    pub fields: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct TemplateListQuery {
    pub kind: Option<String>,
}

/// GET /api/knowledge/templates?kind=
#[utoipa::path(operation_id = "knowledge_list_templates", get, path = "/api/knowledge/templates?kind=", tag = "knowledge", security(("bearer" = [])), 
    params(("kind" = Option<String>, Query)),
    responses((status = 200, body = [kh::TemplateRecord])))]
pub async fn list_templates(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<TemplateListQuery>,
) -> ApiResult<Json<Vec<kh::TemplateRecord>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::View))?;
    if let Some(kind) = q.kind.as_deref() {
        one_of("kind", kind, &["article", "document", "procedure"])?;
    }
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = kh::list_templates(&mut *tx, org_id, q.kind.as_deref()).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// POST /api/knowledge/templates
#[utoipa::path(operation_id = "knowledge_create_template", post, path = "/api/knowledge/templates", tag = "knowledge", security(("bearer" = [])), 
    request_body = CreateKnowledgeTemplateRequest,
    responses((status = 200, body = kh::TemplateRecord)))]
pub async fn create_template(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateKnowledgeTemplateRequest>,
) -> ApiResult<Json<kh::TemplateRecord>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Create))?;
    one_of("kind", &req.kind, &["article", "document", "procedure"])?;
    let name = trimmed("name", &req.name)?;
    let fields = req.fields.unwrap_or_else(|| Value::Array(Vec::new()));
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let id = Uuid::now_v7();
    let record = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let record = kh::insert_template(
            &mut *ctx.tx,
            org_id,
            id,
            req.client_id,
            &req.kind,
            &name,
            req.description.as_deref(),
            &req.body,
            &fields,
            user_id,
        )
        .await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.template.created",
            "knowledge_template",
            id,
            serde_json::json!({ "kind": req.kind, "name": name }),
        )
        .await?;
        Ok((ctx, record))
    })
    .await?;
    Ok(Json(record))
}

/// PATCH /api/knowledge/templates/{id} — snapshots the previous version.
#[utoipa::path(operation_id = "knowledge_update_template", patch, path = "/api/knowledge/templates/{id}", tag = "knowledge", security(("bearer" = [])), 
    params(("id" = Uuid, Path)),
    request_body = UpdateKnowledgeTemplateRequest,
    responses((status = 200, body = kh::TemplateRecord)))]
pub async fn update_template(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateKnowledgeTemplateRequest>,
) -> ApiResult<Json<kh::TemplateRecord>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Edit))?;
    let name = trimmed("name", &req.name)?;
    let fields = req.fields.unwrap_or_else(|| Value::Array(Vec::new()));
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let record = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let record = kh::update_template(
            &mut *ctx.tx,
            org_id,
            id,
            &name,
            req.description.as_deref(),
            &req.body,
            &fields,
            user_id,
        )
        .await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.template.updated",
            "knowledge_template",
            id,
            serde_json::json!({ "name": name, "version": record.version }),
        )
        .await?;
        Ok((ctx, record))
    })
    .await?;
    Ok(Json(record))
}

/* ------------------------------------------------------------------------- */
/* Portal document list (staff preview)                                      */
/* ------------------------------------------------------------------------- */

#[derive(Debug, Deserialize)]
pub struct PortalDocsQuery {
    pub client_id: Uuid,
}

/// GET /api/knowledge/portal/documents?client_id= — exactly what the client
/// portal may show, so staff can preview the client's view without signing in
/// as them.
#[utoipa::path(operation_id = "knowledge_portal_documents", get, path = "/api/knowledge/portal/documents?client_id=", tag = "portal", 
    params(("client_id" = Uuid, Query)),
    responses((status = 200, body = [kh::PortalDocRow])))]
pub async fn portal_documents(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<PortalDocsQuery>,
) -> ApiResult<Json<Vec<kh::PortalDocRow>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = kh::portal_documents(&mut *tx, org_id, q.client_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/* ------------------------------------------------------------------------- */
/* Portal document list (client session)                                     */
/* ------------------------------------------------------------------------- */

/// GET /api/portal/knowledge/documents — the portal's own view of the
/// documents this client may read. Portal-authenticated (not staff), and
/// gated on the same `ViewKb` action the portal's article list uses, so a
/// portal role that cannot read help pages cannot read documents either.
#[utoipa::path(operation_id = "knowledge_portal_knowledge_documents", get, path = "/api/portal/knowledge/documents", tag = "portal", 
    responses((status = 200, body = [kh::PortalDocRow])))]
pub async fn portal_knowledge_documents(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<kh::PortalDocRow>>> {
    let authed = extensions
        .get::<crate::auth::PortalAuthed>()
        .cloned()
        .ok_or_else(|| ApiError::Unauthorized("not authenticated".into()))?;
    core_domain::policy::check_portal(authed.user.role, core_domain::policy::PortalAction::ViewKb)
        .expect()
        .map_err(ApiError::Forbidden)?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, authed.user.org_id, authed.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = kh::portal_documents(&mut *tx, authed.user.org_id, authed.user.client_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/* ------------------------------------------------------------------------- */
/* Two-step codes on a credential                                            */
/* ------------------------------------------------------------------------- */

#[derive(Debug, Deserialize, ToSchema)]
pub struct AttachTotpRequest {
    /// Base32 seed from the authenticator. Spaces and dashes are ignored.
    pub seed_base32: String,
    #[serde(default = "default_digits")]
    pub digits: i16,
    #[serde(default = "default_period")]
    pub period_seconds: i16,
    #[serde(default = "default_algorithm")]
    pub algorithm: String,
    pub issuer: Option<String>,
    pub account: Option<String>,
    /// The code the authenticator shows now. Required: a seed that has never
    /// produced a code is a seed nobody checked.
    pub verify_code: String,
}

fn default_digits() -> i16 {
    6
}
fn default_period() -> i16 {
    30
}
fn default_algorithm() -> String {
    "SHA1".into()
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TotpStateResponse {
    pub record_id: Uuid,
    pub configured: bool,
    pub digits: Option<i16>,
    pub period_seconds: Option<i16>,
    pub algorithm: Option<String>,
    pub issuer: Option<String>,
    pub account: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TotpCodeResponse {
    pub code: String,
    pub digits: i16,
    pub period_seconds: i16,
    pub seconds_remaining: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub otpauth_url: Option<String>,
}

fn totp_aad(org_id: Uuid, record_id: Uuid) -> Vec<u8> {
    format!("knowledge-totp:{org_id}:{record_id}").into_bytes()
}

fn build_totp(
    seed: &[u8],
    digits: i16,
    period: i16,
    algorithm: &str,
    issuer: &str,
    account: &str,
) -> Result<totp_rs::Totp, ApiError> {
    let alg = match algorithm {
        "SHA1" => totp_rs::Algorithm::SHA1,
        "SHA256" => totp_rs::Algorithm::SHA256,
        "SHA512" => totp_rs::Algorithm::SHA512,
        other => {
            return Err(ApiError::Validation(format!(
                "algorithm must be SHA1, SHA256 or SHA512, not {other}"
            )))
        }
    };
    totp_rs::Builder::new()
        .with_algorithm(alg)
        .with_digits(u8::try_from(digits).unwrap_or(6))
        .with_skew(1)
        .with_step_duration(u64::try_from(period).unwrap_or(30))
        .with_secret(seed.to_vec())
        .with_issuer(Some(issuer))
        .with_account_name(account)
        .build()
        .map_err(|e| ApiError::Validation(format!("seed rejected: {e}")))
}

/// GET /api/knowledge/secrets/{record_id}/totp — configuration only. The seed
/// and any code are unavailable on this route.
#[utoipa::path(operation_id = "knowledge_get_totp_state", get, path = "/api/knowledge/secrets/{record_id}/totp", tag = "knowledge", security(("bearer" = [])), 
    params(("record_id" = Uuid, Path)),
    responses((status = 200, body = TotpStateResponse)))]
pub async fn get_totp_state(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(record_id): Path<Uuid>,
) -> ApiResult<Json<TotpStateResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let row = kh::get_secret_totp(&mut *tx, org_id, record_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(match row {
        Some(r) => TotpStateResponse {
            record_id,
            configured: true,
            digits: Some(r.digits),
            period_seconds: Some(r.period_seconds),
            algorithm: Some(r.algorithm),
            issuer: r.issuer,
            account: r.account,
            created_at: Some(r.created_at),
        },
        None => TotpStateResponse {
            record_id,
            configured: false,
            digits: None,
            period_seconds: None,
            algorithm: None,
            issuer: None,
            account: None,
            created_at: None,
        },
    }))
}

/// POST /api/knowledge/secrets/{record_id}/totp — verify the code the
/// authenticator shows, then seal the seed. Saving needs the same reveal
/// permission as reading the password (`secrets:edit`).
#[utoipa::path(operation_id = "knowledge_attach_totp", post, path = "/api/knowledge/secrets/{record_id}/totp", tag = "knowledge", security(("bearer" = [])), 
    params(("record_id" = Uuid, Path)),
    request_body = AttachTotpRequest,
    responses((status = 200, body = TotpStateResponse)))]
pub async fn attach_totp(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(record_id): Path<Uuid>,
    Json(req): Json<AttachTotpRequest>,
) -> ApiResult<Json<TotpStateResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::Edit))?;
    one_of("algorithm", &req.algorithm, &["SHA1", "SHA256", "SHA512"])?;
    if !matches!(req.digits, 6 | 8) {
        return Err(ApiError::Validation("digits must be 6 or 8".into()));
    }
    if !(15..=120).contains(&req.period_seconds) {
        return Err(ApiError::Validation(
            "period_seconds must be 15 to 120".into(),
        ));
    }
    // Normalise the seed the way an authenticator's display does: no spaces,
    // no dashes, uppercase.
    let cleaned: String = req
        .seed_base32
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect::<String>()
        .to_ascii_uppercase();
    let seed = data_encoding::BASE32_NOPAD
        .decode(cleaned.as_bytes())
        .map_err(|_| ApiError::Validation("seed_base32 is not valid base32".into()))?;
    if seed.len() < 10 {
        return Err(ApiError::Validation(
            "the seed is too short to be a TOTP secret".into(),
        ));
    }
    let issuer = req.issuer.clone().unwrap_or_else(|| "Marula".into());
    let account = req.account.clone().unwrap_or_else(|| record_id.to_string());
    let totp = build_totp(
        &seed,
        req.digits,
        req.period_seconds,
        &req.algorithm,
        &issuer,
        &account,
    )?;
    let now = Utc::now().timestamp();
    let expected = totp.generate(u64::try_from(now).unwrap_or(0)).to_string();
    if expected.trim() != req.verify_code.trim() {
        return Err(ApiError::Validation(
            "that code does not match the seed; check the authenticator's clock".into(),
        ));
    }

    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let secret_key = state.config.secret_key.clone();
    let digits = req.digits;
    let period = req.period_seconds;
    let algorithm = req.algorithm.clone();
    let algorithm_c = algorithm.clone();
    let issuer_c = issuer.clone();
    let account_c = account.clone();
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        // The record must exist in scope before a seed can attach to it.
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM secret_records WHERE org_id = $1 AND id = $2)",
        )
        .bind(org_id)
        .bind(record_id)
        .fetch_one(&mut *ctx.tx)
        .await
        .map_err(ApiError::from)?;
        if !exists {
            return Err(ApiError::NotFound);
        }
        let sealed = crate::crypto::seal_with_aad(&secret_key, &seed, &totp_aad(org_id, record_id))
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("totp seal failed: {e}")))?;
        kh::upsert_secret_totp(
            &mut *ctx.tx,
            org_id,
            record_id,
            &sealed,
            digits,
            period,
            &algorithm_c,
            Some(&issuer_c),
            Some(&account_c),
            user_id,
        )
        .await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.secret_totp.attached",
            "secret_record",
            record_id,
            serde_json::json!({ "digits": digits, "period": period }),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;

    Ok(Json(TotpStateResponse {
        record_id,
        configured: true,
        digits: Some(digits),
        period_seconds: Some(period),
        algorithm: Some(algorithm),
        issuer: Some(issuer),
        account: Some(account),
        created_at: Some(Utc::now()),
    }))
}

/// POST /api/knowledge/secrets/{record_id}/totp/code — the audited code read.
/// Sealed seed in, six or eight digits out; nothing else ever leaves.
#[utoipa::path(operation_id = "knowledge_read_totp_code", post, path = "/api/knowledge/secrets/{record_id}/totp/code", tag = "knowledge", security(("bearer" = [])), 
    params(("record_id" = Uuid, Path)),
    responses((status = 200, body = TotpCodeResponse)))]
pub async fn read_totp_code(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(record_id): Path<Uuid>,
) -> ApiResult<Json<TotpCodeResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let secret_key = state.config.secret_key.clone();
    let payload = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let row = kh::get_secret_totp(&mut *ctx.tx, org_id, record_id)
            .await?
            .ok_or_else(|| ApiError::NotFound as ApiError)?;
        let seed =
            crate::crypto::open_with_aad(&secret_key, &row.seed_enc, &totp_aad(org_id, record_id))
                .map_err(|e| ApiError::Internal(anyhow::anyhow!("totp unseal failed: {e}")))?;
        let issuer = row.issuer.clone().unwrap_or_else(|| "Marula".into());
        let account = row.account.clone().unwrap_or_else(|| record_id.to_string());
        let totp = build_totp(
            &seed,
            row.digits,
            row.period_seconds,
            &row.algorithm,
            &issuer,
            &account,
        )?;
        let now = Utc::now().timestamp();
        let code = totp.generate(u64::try_from(now).unwrap_or(0)).to_string();
        let remaining =
            i64::from(row.period_seconds) - (now.rem_euclid(i64::from(row.period_seconds)));
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.secret_totp.read",
            "secret_record",
            record_id,
            serde_json::json!({ "digits": row.digits }),
        )
        .await?;
        Ok((
            ctx,
            TotpCodeResponse {
                code,
                digits: row.digits,
                period_seconds: row.period_seconds,
                seconds_remaining: remaining.max(0),
                otpauth_url: None,
            },
        ))
    })
    .await?;
    Ok(Json(payload))
}

/// DELETE /api/knowledge/secrets/{record_id}/totp
#[utoipa::path(operation_id = "knowledge_delete_totp", delete, path = "/api/knowledge/secrets/{record_id}/totp", tag = "knowledge", security(("bearer" = [])), 
    params(("record_id" = Uuid, Path)),
    responses((status = 200, body = Object)))]
pub async fn delete_totp(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(record_id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        kh::delete_secret_totp(&mut *ctx.tx, org_id, record_id).await?;
        audit(
            &mut ctx.tx,
            org_id,
            user_id,
            "knowledge.secret_totp.removed",
            "secret_record",
            record_id,
            Value::Null,
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(Json(serde_json::json!({ "removed": record_id })))
}

/* ------------------------------------------------------------------------- */
/* Router                                                                    */
/* ------------------------------------------------------------------------- */

pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    let k = |tail: &str| format!("{base}/knowledge{tail}");
    Router::new()
        // The client-first home and its hub.
        .route(&k("/home"), get(home))
        .route(&k("/clients/{client_id}/hub"), get(client_hub))
        .route(
            &k("/clients/{client_id}/settings"),
            get(get_client_settings).put(put_client_settings),
        )
        .route(&k("/expirations"), get(expirations))
        .route(&k("/search"), get(search))
        // Racks and placements.
        .route(&k("/racks"), get(list_racks).post(create_rack))
        .route(
            &k("/racks/{id}"),
            get(get_rack).patch(update_rack).delete(delete_rack),
        )
        .route(&k("/racks/{id}/devices"), post(add_rack_device))
        .route(
            &k("/racks/{id}/devices/{device_id}"),
            patch(update_rack_device).delete(delete_rack_device),
        )
        // Networks, subnets and address reservations.
        .route(&k("/networks"), get(list_networks).post(create_network))
        .route(
            &k("/networks/{id}"),
            get(get_network)
                .patch(update_network)
                .delete(delete_network),
        )
        .route(&k("/networks/{id}/subnets"), post(add_subnet))
        .route(
            &format!("{base}/knowledge/subnets/{{id}}"),
            delete(delete_subnet),
        )
        .route(&k("/subnets/{id}/reservations"), post(add_reservation))
        .route(
            &format!("{base}/knowledge/reservations/{{id}}"),
            delete(delete_reservation),
        )
        // Domains and websites.
        .route(&k("/domains"), get(list_domains).post(create_domain))
        .route(
            &k("/domains/{id}"),
            patch(update_domain).delete(delete_domain),
        )
        // Procedures and their ordered steps.
        .route(
            &k("/procedures"),
            get(list_procedures).post(create_procedure),
        )
        .route(
            &k("/procedures/{id}"),
            get(get_procedure)
                .patch(update_procedure)
                .delete(delete_procedure),
        )
        // The generic knowledge edge, hub settings, portal visibility.
        .route(&k("/links"), get(list_links).post(create_link))
        .route(
            &format!("{base}/knowledge/links/{{id}}"),
            delete(delete_link),
        )
        .route(
            &k("/documents/{id}/visibility"),
            axum::routing::put(put_document_visibility),
        )
        .route(&k("/portal/documents"), get(portal_documents))
        // Templates.
        .route(&k("/templates"), get(list_templates).post(create_template))
        .route(&k("/templates/{id}"), patch(update_template))
        // Two-step codes on a credential.
        .route(
            &k("/secrets/{record_id}/totp"),
            get(get_totp_state).post(attach_totp).delete(delete_totp),
        )
        .route(&k("/secrets/{record_id}/totp/code"), post(read_totp_code))
        // The portal's own document list (portal session, not staff auth).
        .route(
            &format!("{base}/portal/knowledge/documents"),
            get(portal_knowledge_documents),
        )
}
