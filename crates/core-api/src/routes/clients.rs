//! Client hierarchy endpoints: company → site → contact → asset.
//! Mutations run in org-scoped transactions and emit their domain events
//! in the same commit (§4.2) — the search indexer depends on these.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use core_domain::events::types;
use core_domain::policy::{Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
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

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateClientRequest {
    pub name: String,
    pub notes: Option<String>,
    /// Instant-first autosave parameters (IF1): idempotency key. Absent in
    /// legacy clients — purely additive. (Creates carry no version.)
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// Instant-first mutation response (IF2): client fields at the top level
/// (unchanged shape) plus the additive `mutation` metadata.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ClientMutationResponse {
    #[serde(flatten)]
    pub client: core_domain::entities::Client,
    pub mutation: instant::MutationMeta,
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
#[utoipa::path(operation_id = "clients_list", get, path = "/api/clients", tag = "clients", security(("bearer" = [])),
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
#[utoipa::path(operation_id = "clients_get_one", get, path = "/api/clients/{id}", tag = "clients", security(("bearer" = [])),
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

/// POST /api/clients — instant-first (IF1/IF2): idempotency-keyed and
/// returns the read-your-write `mutation` metadata.
#[utoipa::path(operation_id = "clients_create", post, path = "/api/clients", tag = "clients", security(("bearer" = [])),
    request_body = CreateClientRequest,
    responses((status = 200, body = ClientMutationResponse),
              (status = 409, description = "idempotency key reused")))]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateClientRequest>,
) -> ApiResult<Json<ClientMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, client_edit_permission())?;
    if req.name.trim().is_empty() {
        return Err(ApiError::Validation("name is required".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "client",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let client = core_db::clients::create_client(
            &mut *ctx.tx,
            org_id,
            Uuid::now_v7(),
            req.name.trim(),
            req.notes.as_deref(),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "client",
                entity_id: client.id,
                version: None,
            },
        )
        .await?;
        ctx.emit(
            types::CLIENT_CREATED,
            "client",
            Some(client.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ClientCreated {
                client_id: client.id,
                name: client.name.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "client",
                entity_id: client.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&client).unwrap_or(Value::Null),
                updated_at: client.created_at,
                key: key.as_deref(),
                payload: &client,
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ClientMutationResponse { client, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// GET /api/clients/{id}/sites
#[utoipa::path(operation_id = "clients_list_sites", get, path = "/api/clients/{id}/sites", tag = "clients", security(("bearer" = [])),
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
#[utoipa::path(operation_id = "clients_create_site", post, path = "/api/clients/{id}/sites", tag = "clients", security(("bearer" = [])),
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
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let site = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let site = core_db::clients::create_site(
            &mut *ctx.tx,
            org_id,
            Uuid::now_v7(),
            id,
            &req.name,
            req.address.as_deref(),
            req.city.as_deref(),
            req.region.as_deref(),
            req.postal_code.as_deref(),
            req.country.as_deref(),
            req.latitude,
            req.longitude,
            req.timezone.as_deref(),
        )
        .await?;
        ctx.emit(
            types::SITE_CREATED,
            "site",
            Some(site.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::SiteCreated {
                site_id: site.id,
                client_id: site.client_id,
                name: site.name.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        Ok((ctx, site))
    })
    .await?;
    Ok(Json(site))
}

/// GET /api/clients/{id}/contacts
#[utoipa::path(operation_id = "clients_list_contacts", get, path = "/api/clients/{id}/contacts", tag = "clients", security(("bearer" = [])),
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
#[utoipa::path(operation_id = "clients_create_contact", post, path = "/api/clients/{id}/contacts", tag = "clients", security(("bearer" = [])),
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
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let contact = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let contact = core_db::clients::create_contact(
            &mut *ctx.tx,
            org_id,
            Uuid::now_v7(),
            id,
            None,
            &req.name,
            req.email.as_deref(),
            req.phone.as_deref(),
            None,
            false,
        )
        .await?;
        ctx.emit(
            types::CONTACT_CREATED,
            "contact",
            Some(contact.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ContactCreated {
                contact_id: contact.id,
                client_id: contact.client_id,
                name: contact.name.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        Ok((ctx, contact))
    })
    .await?;
    Ok(Json(contact))
}

/// GET /api/clients/{id}/assets
#[utoipa::path(operation_id = "clients_list_assets", get, path = "/api/clients/{id}/assets", tag = "clients", security(("bearer" = [])),
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
#[utoipa::path(operation_id = "clients_create_asset", post, path = "/api/clients/{id}/assets", tag = "clients", security(("bearer" = [])),
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
