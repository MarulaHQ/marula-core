//! Client hierarchy endpoints: company → site → contact → asset.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use core_domain::policy::{Module, Permission};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

fn client_view_permission() -> Permission {
    Permission::Matrix {
        module: Module::Clients,
        action: core_domain::policy::Action::View,
    }
}

fn client_edit_permission() -> Permission {
    Permission::Matrix {
        module: Module::Clients,
        action: core_domain::policy::Action::Create,
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateClientRequest {
    pub name: String,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSiteRequest {
    pub name: String,
    pub address: Option<String>,
    pub city: Option<String>,
    pub region: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub timezone: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateContactRequest {
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAssetRequest {
    pub tag: String,
    pub name: String,
    pub kind: Option<String>,
}

/// GET /api/clients
#[utoipa::path(get, path = "/api/clients", tag = "clients", security(("bearer" = [])),
    responses((status = 200, body = [core_domain::entities::Client])))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<core_domain::entities::Client>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, client_view_permission())?;
    let clients = core_db::clients::list_clients(&state.pool, authed.principal.user.org_id).await?;
    Ok(Json(clients))
}

/// GET /api/clients/{id}
#[utoipa::path(get, path = "/api/clients/{id}", tag = "clients", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = core_domain::entities::Client)))]
pub async fn get_one(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<core_domain::entities::Client>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, client_view_permission())?;
    let client =
        core_db::clients::get_client(&state.pool, authed.principal.user.org_id, id).await?;
    // Per-client visibility rule: restricted_to lists allowed users; admins bypass.
    if !client.restricted_to.is_empty()
        && !client.restricted_to.contains(&authed.principal.user.id)
        && !authed.principal.is_owner_or_admin()
    {
        return Err(ApiError::NotFound);
    }
    Ok(Json(client))
}

/// POST /api/clients
#[utoipa::path(post, path = "/api/clients", tag = "clients", security(("bearer" = [])),
    request_body = CreateClientRequest,
    responses((status = 200, body = core_domain::entities::Client)))]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateClientRequest>,
) -> ApiResult<Json<core_domain::entities::Client>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, client_edit_permission())?;
    if req.name.trim().is_empty() {
        return Err(ApiError::Validation("name is required".into()));
    }
    let client = core_db::clients::create_client(
        &state.pool,
        authed.principal.user.org_id,
        Uuid::now_v7(),
        req.name.trim(),
        req.notes.as_deref(),
    )
    .await?;
    Ok(Json(client))
}

/// GET /api/clients/{id}/sites
#[utoipa::path(get, path = "/api/clients/{id}/sites", tag = "clients", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [core_domain::entities::Site])))]
pub async fn list_sites(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<core_domain::entities::Site>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, client_view_permission())?;
    let sites = core_db::clients::list_sites(&state.pool, authed.principal.user.org_id, id).await?;
    Ok(Json(sites))
}

/// POST /api/clients/{id}/sites
#[utoipa::path(post, path = "/api/clients/{id}/sites", tag = "clients", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = CreateSiteRequest,
    responses((status = 200, body = core_domain::entities::Site)))]
pub async fn create_site(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateSiteRequest>,
) -> ApiResult<Json<core_domain::entities::Site>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, client_edit_permission())?;
    let site = core_db::clients::create_site(
        &state.pool,
        authed.principal.user.org_id,
        Uuid::now_v7(),
        id,
        &req.name,
    )
    .await?;
    Ok(Json(site))
}

/// GET /api/clients/{id}/contacts
#[utoipa::path(get, path = "/api/clients/{id}/contacts", tag = "clients", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [core_domain::entities::Contact])))]
pub async fn list_contacts(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<core_domain::entities::Contact>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, client_view_permission())?;
    let contacts =
        core_db::clients::list_contacts(&state.pool, authed.principal.user.org_id, id).await?;
    Ok(Json(contacts))
}

/// POST /api/clients/{id}/contacts
#[utoipa::path(post, path = "/api/clients/{id}/contacts", tag = "clients", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = CreateContactRequest,
    responses((status = 200, body = core_domain::entities::Contact)))]
pub async fn create_contact(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateContactRequest>,
) -> ApiResult<Json<core_domain::entities::Contact>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, client_edit_permission())?;
    let contact = core_db::clients::create_contact(
        &state.pool,
        authed.principal.user.org_id,
        Uuid::now_v7(),
        id,
        &req.name,
        req.email.as_deref(),
        req.phone.as_deref(),
    )
    .await?;
    Ok(Json(contact))
}

/// GET /api/clients/{id}/assets
#[utoipa::path(get, path = "/api/clients/{id}/assets", tag = "clients", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [core_domain::entities::DeviceAsset])))]
pub async fn list_assets(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<core_domain::entities::DeviceAsset>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, client_view_permission())?;
    let assets =
        core_db::clients::list_assets(&state.pool, authed.principal.user.org_id, id).await?;
    Ok(Json(assets))
}

/// POST /api/clients/{id}/assets
#[utoipa::path(post, path = "/api/clients/{id}/assets", tag = "clients", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = CreateAssetRequest,
    responses((status = 200, body = core_domain::entities::DeviceAsset)))]
pub async fn create_asset(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateAssetRequest>,
) -> ApiResult<Json<core_domain::entities::DeviceAsset>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, client_edit_permission())?;
    let asset = core_db::clients::create_asset(
        &state.pool,
        authed.principal.user.org_id,
        Uuid::now_v7(),
        id,
        &req.tag,
        &req.name,
        req.kind.as_deref().unwrap_or("device"),
    )
    .await?;
    Ok(Json(asset))
}
