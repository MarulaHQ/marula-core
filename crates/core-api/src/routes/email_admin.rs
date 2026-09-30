//! Email administration: inbound routing, per-board outbound identities,
//! transport configuration (credentials sealed in the generic sealed slot),
//! the outbound delivery ledger view, and the test-send probe.
//!
//! Purpose: give operators one admin surface for the whole mail edge —
//! where inbound mail routes (org catch-all + per-board addresses), who
//! outbound mail comes from (org identity + per-board overrides), how it
//! leaves the building (none/postmark/smtp with credentials sealed at rest),
//! what happened to each message (queued/sending/sent/dead with attempts),
//! and a probe that proves the configured transport renders and sends.
//!
//! Doctrine notes:
//! - Non-secret configuration rides the typed settings registry; the two
//!   transport credentials (Postmark server token, SMTP password) go
//!   through the SEALED store (`byok_secrets`, reserved `mail.*` provider
//!   ids) — never env-only, never returned, never logged.
//! - Per-board identities are org-scope settings under board-qualified keys
//!   (`email.board.<id>.<field>`); the settings model holds them, so no
//!   schema change was needed.
//! - Inbound addresses are globally unique by index: they are how an
//!   inbound webhook resolves WHICH org and board receive a message.
//!   Collisions inside the org are refused here with typed 409s; collisions
//!   across orgs are refused by the database unique indexes.
//!
//! Non-purpose: no mailbox provisioning, no DNS automation, no provider
//! webhook wiring — operators point their mail provider at Marula and
//! configure addresses here to match (see docs/site/how-to/admin-email.md).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::Extensions;
use axum::Json;
use core_domain::entities::{Actor, AuditEvent};
use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use core_domain::settings as registry;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

/// Reserved sealed-slot provider ids for mail transport credentials.
pub const SEALED_POSTMARK_PROVIDER: &str = "mail.postmark";
pub const SEALED_SMTP_PROVIDER: &str = "mail.smtp";

fn admin(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action,
    }
}

fn audit_event(
    org_id: Uuid,
    actor_id: Uuid,
    entity_id: String,
    detail: serde_json::Value,
) -> AuditEvent {
    AuditEvent {
        id: Uuid::now_v7(),
        org_id,
        actor: Actor::Human(actor_id),
        action: "email.settings_updated".to_string(),
        entity_type: Some("email_settings".to_string()),
        entity_id: Some(entity_id),
        detail,
        ip: None,
        occurred_at: chrono::Utc::now(),
    }
}

// --- shared view plumbing ----------------------------------------------------

/// Stored override or registry default — the same defaults-first read the
/// settings hub performs, minus the declaration listing.
async fn org_value_or_default(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    key: &str,
) -> ApiResult<serde_json::Value> {
    if let Some(value) = core_db::settings::get_org_value(&mut *conn, org_id, key).await? {
        return Ok(value);
    }
    Ok(registry::default_value(key)
        .map(|v| v.to_json())
        .unwrap_or(serde_json::Value::Null))
}

/// Non-empty string extraction from a stored JSON value.
fn as_text(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
}

fn as_i64(value: &serde_json::Value, fallback: i64) -> i64 {
    value.as_i64().unwrap_or(fallback)
}

fn is_mailbox(addr: &str) -> bool {
    let trimmed = addr.trim();
    !trimmed.is_empty() && trimmed.contains('@') && !trimmed.contains(char::is_whitespace)
}

/// Validates one address against the registry's mailbox shape (the same
/// rule the settings declarations describe).
fn require_mailbox(field: &str, addr: &str) -> ApiResult<String> {
    let trimmed = addr.trim().to_ascii_lowercase();
    if !is_mailbox(&trimmed) {
        return Err(ApiError::Validation(format!(
            "{field} must be a mailbox address"
        )));
    }
    Ok(trimmed)
}

// --- inbound routing ---------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct InboundSettingsResponse {
    pub catchall_address: Option<String>,
    pub catchall_board_id: Option<Uuid>,
    pub enabled: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PutInboundSettingsRequest {
    pub catchall_address: String,
    pub catchall_board_id: Uuid,
    /// Enables/disables catch-all routing; absent keeps the stored value.
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PutBoardAddressRequest {
    /// `None` clears the address.
    pub address: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BoardAddressResponse {
    pub board_id: Uuid,
    pub inbound_address: Option<String>,
}

/// GET /api/email/inbound-settings — current org catch-all, if configured.
#[utoipa::path(operation_id = "email_inbound_settings_get", get,
    path = "/api/email/inbound-settings", tag = "email",
    security(("bearer" = [])),
    responses((status = 200, body = InboundSettingsResponse)))]
pub async fn get_inbound_settings(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<InboundSettingsResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    let org = authed.principal.user.org_id;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let settings = core_db::emails::get_inbound_settings(&mut *tx, org).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(InboundSettingsResponse {
        catchall_address: settings.as_ref().map(|s| s.catchall_address.clone()),
        catchall_board_id: settings.as_ref().map(|s| s.catchall_board_id),
        enabled: settings.as_ref().is_some_and(|s| s.enabled),
    }))
}

/// PUT /api/email/inbound-settings — upsert the org catch-all route.
#[utoipa::path(operation_id = "email_inbound_settings_put", put,
    path = "/api/email/inbound-settings", tag = "email",
    security(("bearer" = [])), request_body = PutInboundSettingsRequest,
    responses((status = 200, body = InboundSettingsResponse),
              (status = 409, description = "address already routed")))]
pub async fn put_inbound_settings(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<PutInboundSettingsRequest>,
) -> ApiResult<Json<InboundSettingsResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org = authed.principal.user.org_id;
    let user = authed.principal.user.id;
    let address = require_mailbox("catchall_address", &req.catchall_address)?;
    let result = crate::org_tx::with_org_tx(&state, org, user, |mut ctx| async move {
        // Board must exist inside THIS org before it can receive mail.
        core_db::boards::get_board(&mut *ctx.tx, org, req.catchall_board_id)
            .await
            .map_err(|_| ApiError::Validation("catchall_board_id not found in org".into()))?;
        // Collision validation: an address routed to a board, or to this
        // org's other routing slot, cannot silently become a catch-all.
        if let Some(owner) = inbound_address_owner(&mut ctx.tx, org, &address, None, true).await? {
            return Err(ApiError::Conflict(format!(
                "address already in use by {owner}"
            )));
        }
        core_db::emails::upsert_inbound_settings(&mut ctx.tx, org, &address, req.catchall_board_id)
            .await?;
        if let Some(enabled) = req.enabled {
            core_db::emails::set_inbound_enabled(&mut ctx.tx, org, enabled).await?;
        }
        core_db::audit::insert(
            &mut *ctx.tx,
            &audit_event(
                org,
                user,
                "inbound:catchall".into(),
                json!({ "address": address, "board_id": req.catchall_board_id, "enabled": req.enabled }),
            ),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await;
    map_inbound_write(result)?;
    get_inbound_settings(State(state), extensions).await
}

/// PUT /api/boards/{id}/inbound-address — set/clear a board-level address.
#[utoipa::path(operation_id = "email_board_inbound_address_put", put,
    path = "/api/boards/{id}/inbound-address", tag = "email",
    security(("bearer" = [])), request_body = PutBoardAddressRequest,
    params(("id" = Uuid, Path, description = "Board id")),
    responses((status = 200, body = BoardAddressResponse),
              (status = 404, description = "board not found"),
              (status = 409, description = "address already routed")))]
pub async fn put_board_inbound_address(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(board_id): Path<Uuid>,
    Json(req): Json<PutBoardAddressRequest>,
) -> ApiResult<Json<BoardAddressResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org = authed.principal.user.org_id;
    let user = authed.principal.user.id;
    let address = match req.address.as_deref() {
        Some(raw) if !raw.trim().is_empty() => Some(require_mailbox("address", raw)?),
        // Blank and null both clear; a stored empty string would be a trap.
        _ => None,
    };
    let clear = address.is_none();
    let address = address.unwrap_or_default();
    // The tx closure is `async move`, so it takes its own copy while the
    // response below still reports the stored address (compile fix, Wave 34
    // lane 6: the fast-forwarded lane-8 commit used `address` after the move).
    let address_in_tx = address.clone();
    let result = crate::org_tx::with_org_tx(&state, org, user, |mut ctx| async move {
        core_db::set_org(&mut ctx.tx, org)
            .await
            .map_err(ApiError::from)?;
        // Board-level RBAC (deny wins): a restrictive membership row on
        // this board blocks even org-level admins.
        crate::routes::board_access::authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            board_id,
            core_domain::policy::BoardOp::ConfigureBoard,
        )
        .await?;
        if !clear {
            if let Some(owner) =
                inbound_address_owner(&mut ctx.tx, org, &address_in_tx, Some(board_id), false).await?
            {
                return Err(ApiError::Conflict(format!(
                    "address already in use by {owner}"
                )));
            }
        }
        let result = core_db::emails::set_board_inbound_address(
            &mut *ctx.tx,
            org,
            board_id,
            (!clear).then_some(address_in_tx.as_str()),
        )
        .await;
        match result {
            Ok(()) => {}
            Err(core_db::DbError::NotFound) => return Err(ApiError::NotFound),
            Err(e) => return Err(e.into()),
        }
        core_db::audit::insert(
            &mut *ctx.tx,
            &audit_event(
                org,
                user,
                format!("inbound:board:{board_id}"),
                json!({ "address": if clear { serde_json::Value::Null } else { json!(address_in_tx) } }),
            ),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await;
    match result {
        Ok(()) => Ok(Json(BoardAddressResponse {
            board_id,
            inbound_address: (!clear).then_some(address),
        })),
        Err(ApiError::NotFound) => Err(ApiError::NotFound),
        Err(e) => Err(e),
    }
}

/// Describes who already owns an inbound address inside this org, if
/// anyone: `board '<name>'` or `the org catch-all`. When
/// `allow_current_catchall` is set, the org's OWN stored catch-all address
/// counts as free (the catch-all upsert updating itself).
async fn inbound_address_owner(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    address: &str,
    exclude_board: Option<Uuid>,
    allow_current_catchall: bool,
) -> ApiResult<Option<String>> {
    let needle = address.trim().to_ascii_lowercase();
    for (id, name, inbound) in core_db::emails::list_board_routes(&mut *conn, org_id).await? {
        if Some(id) == exclude_board {
            continue;
        }
        if inbound.is_some_and(|a| a.trim().to_ascii_lowercase() == needle) {
            return Ok(Some(format!("board '{name}'")));
        }
    }
    if let Some(settings) = core_db::emails::get_inbound_settings(&mut *conn, org_id).await? {
        let same = settings.catchall_address.trim().to_ascii_lowercase() == needle;
        if same && allow_current_catchall {
            return Ok(None);
        }
        if same {
            return Ok(Some("the org catch-all".into()));
        }
    }
    Ok(None)
}

/// Maps the one constraint the DB can still hit (a cross-org address) onto
/// the same typed 409 the in-org checks raise.
fn map_inbound_write(result: ApiResult<()>) -> ApiResult<()> {
    match result {
        Ok(()) => Ok(()),
        Err(ApiError::Internal(err)) => {
            if err.to_string().contains("idx_") {
                Err(ApiError::Conflict("address already in use".into()))
            } else {
                Err(ApiError::Internal(err))
            }
        }
        Err(other) => Err(other),
    }
}

// --- per-board outbound identity ---------------------------------------------

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BoardIdentityView {
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub reply_to: Option<String>,
    pub signature: Option<String>,
}

impl From<core_db::emails::BoardIdentity> for BoardIdentityView {
    fn from(value: core_db::emails::BoardIdentity) -> Self {
        Self {
            from_address: value.from_address,
            from_name: value.from_name,
            reply_to: value.reply_to,
            signature: value.signature,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PutBoardIdentityRequest {
    /// All fields optional and partial: only present keys are written; an
    /// explicit `null` (or blank string) clears that field back to the org
    /// identity.
    #[serde(default, deserialize_with = "double_option")]
    pub from_address: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub from_name: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub reply_to: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub signature: Option<Option<String>>,
}

/// Distinguishes an absent key from an explicit JSON `null` so PATCH
/// semantics stay honest (absent = untouched, null = clear).
fn double_option<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

/// PUT /api/boards/{id}/email-identity — set/clear per-board outbound
/// identity fields (from address, display name, reply-to, signature).
#[utoipa::path(operation_id = "email_board_identity_put", put,
    path = "/api/boards/{id}/email-identity", tag = "email",
    security(("bearer" = [])), request_body = PutBoardIdentityRequest,
    params(("id" = Uuid, Path, description = "Board id")),
    responses((status = 200, body = BoardIdentityView),
              (status = 404, description = "board not found"),
              (status = 409, description = "from address already in use")))]
pub async fn put_board_identity(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(board_id): Path<Uuid>,
    Json(req): Json<PutBoardIdentityRequest>,
) -> ApiResult<Json<BoardIdentityView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org = authed.principal.user.org_id;
    let user = authed.principal.user.id;

    // Validate shapes before opening the transaction.
    let from_address = match req.from_address.as_ref().map(|v| v.as_deref()) {
        Some(Some(raw)) if !raw.trim().is_empty() => Some(require_mailbox("from_address", raw)?),
        _ => None,
    };
    let reply_to = match req.reply_to.as_ref().map(|v| v.as_deref()) {
        Some(Some(raw)) if !raw.trim().is_empty() => Some(require_mailbox("reply_to", raw)?),
        _ => None,
    };
    // A board signature is a signature: same placeholders, same gate, so a
    // board cannot store a token the send path would not expand.
    if let Some(Some(raw)) = req.signature.as_ref() {
        crate::email::validate_signature_text("signature", raw)?;
    }

    let view = crate::org_tx::with_org_tx(&state, org, user, |mut ctx| async move {
        core_db::set_org(&mut ctx.tx, org)
            .await
            .map_err(ApiError::from)?;
        crate::routes::board_access::authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            board_id,
            core_domain::policy::BoardOp::ConfigureBoard,
        )
        .await?;
        if let Some(ref addr) = from_address {
            if let Some(owner) =
                board_from_address_owner(&mut ctx.tx, org, addr, Some(board_id)).await?
            {
                return Err(ApiError::Conflict(format!(
                    "from_address already used by {owner}"
                )));
            }
        }
        let mut changed: Vec<&str> = Vec::new();
        if let Some(value) = req.from_address.as_ref() {
            core_db::emails::set_board_identity_field(
                &mut ctx.tx,
                org,
                board_id,
                core_domain::settings::BoardIdentityField::FromAddress,
                value.as_deref(),
                user,
            )
            .await
            .map_err(board_identity_error)?;
            changed.push("from_address");
        }
        if let Some(value) = req.from_name.as_ref() {
            core_db::emails::set_board_identity_field(
                &mut ctx.tx,
                org,
                board_id,
                core_domain::settings::BoardIdentityField::FromName,
                value.as_deref(),
                user,
            )
            .await
            .map_err(board_identity_error)?;
            changed.push("from_name");
        }
        if req.reply_to.is_some() {
            core_db::emails::set_board_identity_field(
                &mut ctx.tx,
                org,
                board_id,
                core_domain::settings::BoardIdentityField::ReplyTo,
                reply_to.as_deref(),
                user,
            )
            .await
            .map_err(board_identity_error)?;
            changed.push("reply_to");
        }
        if let Some(value) = req.signature.as_ref() {
            core_db::emails::set_board_identity_field(
                &mut ctx.tx,
                org,
                board_id,
                core_domain::settings::BoardIdentityField::Signature,
                value.as_deref(),
                user,
            )
            .await
            .map_err(board_identity_error)?;
            changed.push("signature");
        }
        let identity = core_db::emails::get_board_identity(&mut ctx.tx, org, board_id).await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &audit_event(
                org,
                user,
                format!("outbound:board:{board_id}"),
                json!({ "changed": changed, "identity": BoardIdentityView::from(identity.clone()) }),
            ),
        )
        .await?;
        Ok((ctx, BoardIdentityView::from(identity)))
    })
    .await?;
    Ok(Json(view))
}

fn board_identity_error(err: core_db::DbError) -> ApiError {
    match err {
        core_db::DbError::NotFound => ApiError::NotFound,
        other => other.into(),
    }
}

/// Which other board (or the org identity) already sends from `address`.
async fn board_from_address_owner(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    address: &str,
    exclude_board: Option<Uuid>,
) -> ApiResult<Option<String>> {
    let needle = address.trim().to_ascii_lowercase();
    let rows = core_db::settings::list_org_values_with_prefix(
        &mut *conn,
        org_id,
        &format!("{}.", "email.board"),
    )
    .await
    .unwrap_or_default();
    let boards = core_db::emails::list_board_routes(&mut *conn, org_id).await?;
    let name_of = |id: Uuid| {
        boards
            .iter()
            .find(|(b, _, _)| *b == id)
            .map(|(_, name, _)| name.clone())
            .unwrap_or_else(|| id.to_string())
    };
    for row in rows {
        let Some((board_id, field)) = core_domain::settings::board_identity_parts(&row.key) else {
            continue;
        };
        if field != core_domain::settings::BoardIdentityField::FromAddress {
            continue;
        }
        if Some(board_id) == exclude_board {
            continue;
        }
        if row
            .value
            .as_str()
            .is_some_and(|v| v.trim().to_ascii_lowercase() == needle)
        {
            return Ok(Some(format!("board '{}'", name_of(board_id))));
        }
    }
    let org_from = core_db::settings::get_org_value(
        &mut *conn,
        org_id,
        core_domain::settings::EMAIL_FROM_ADDRESS,
    )
    .await?;
    if org_from
        .as_ref()
        .and_then(|v| v.as_str())
        .is_some_and(|v| v.trim().to_ascii_lowercase() == needle)
    {
        return Ok(Some("the org From address".into()));
    }
    Ok(None)
}

// --- transport credentials (sealed slot) -------------------------------------

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CredentialView {
    pub configured: bool,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CredentialsView {
    pub postmark: CredentialView,
    pub smtp: CredentialView,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PutTransportCredentialsRequest {
    /// `postmark` or `smtp` — which credential this request rotates.
    pub transport: String,
    /// Postmark server token (required when transport = postmark).
    #[serde(default)]
    pub postmark_server_token: Option<String>,
    /// SMTP password (required when transport = smtp).
    #[serde(default)]
    pub smtp_password: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteTransportCredentialsQuery {
    pub transport: String,
}

/// PUT /api/email/transport-credentials — seals the selected transport's
/// credential into the sealed slot (presence-only response). The plaintext
/// exists exactly once, in this request.
#[utoipa::path(operation_id = "email_transport_credentials_put", put,
    path = "/api/email/transport-credentials", tag = "email",
    security(("bearer" = [])), request_body = PutTransportCredentialsRequest,
    responses((status = 200, body = CredentialsView),
              (status = 422, description = "unknown transport or missing secret")))]
pub async fn put_transport_credentials(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<PutTransportCredentialsRequest>,
) -> ApiResult<Json<CredentialsView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org = authed.principal.user.org_id;
    let user = authed.principal.user.id;

    let (provider, payload, secret_label) = match req.transport.trim() {
        "postmark" => {
            let token = req
                .postmark_server_token
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .ok_or_else(|| {
                    ApiError::Validation("postmark_server_token is required".into())
                })?;
            (
                SEALED_POSTMARK_PROVIDER,
                json!({ "server_token": token }),
                "postmark_server_token",
            )
        }
        "smtp" => {
            let password = req
                .smtp_password
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .ok_or_else(|| ApiError::Validation("smtp_password is required".into()))?;
            (
                SEALED_SMTP_PROVIDER,
                json!({ "password": password }),
                "smtp_password",
            )
        }
        other => {
            return Err(ApiError::Validation(format!(
                "unknown transport '{other}': expected postmark or smtp"
            )))
        }
    };
    let sealed = crate::crypto::seal(
        &state.config.secret_key,
        serde_json::to_vec(&payload)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))?
            .as_slice(),
    )
    .map_err(|e| ApiError::Internal(anyhow::anyhow!(e.to_string())))?;

    crate::org_tx::with_org_tx(&state, org, user, |mut ctx| async move {
        core_db::byok::upsert_secret(
            &mut ctx.tx,
            Uuid::now_v7(),
            org,
            provider,
            &sealed,
            Some(user),
        )
        .await?;
        // Presence-only trail: name the field, never the value.
        core_db::audit::insert(
            &mut *ctx.tx,
            &audit_event(
                org,
                user,
                format!("transport:{provider}"),
                json!({ "field": secret_label, "status": "sealed" }),
            ),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    get_mail_settings(State(state), extensions).await.map(|Json(s)| {
        Json(CredentialsView {
            postmark: s.outbound.credentials.postmark,
            smtp: s.outbound.credentials.smtp,
        })
    })
}

/// DELETE /api/email/transport-credentials?transport=… — removes the sealed
/// credential; the transport then reports not-configured (mail stays queued).
#[utoipa::path(operation_id = "email_transport_credentials_delete", delete,
    path = "/api/email/transport-credentials", tag = "email",
    security(("bearer" = [])),
    params(("transport" = String, Query, description = "postmark | smtp")),
    responses((status = 200, body = CredentialsView),
              (status = 422, description = "unknown transport")))]
pub async fn delete_transport_credentials(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(req): Query<DeleteTransportCredentialsQuery>,
) -> ApiResult<Json<CredentialsView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org = authed.principal.user.org_id;
    let user = authed.principal.user.id;
    let provider = match req.transport.trim() {
        "postmark" => SEALED_POSTMARK_PROVIDER,
        "smtp" => SEALED_SMTP_PROVIDER,
        other => {
            return Err(ApiError::Validation(format!(
                "unknown transport '{other}': expected postmark or smtp"
            )))
        }
    };
    crate::org_tx::with_org_tx(&state, org, user, |mut ctx| async move {
        core_db::byok::delete_secret(&mut ctx.tx, org, provider).await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &audit_event(
                org,
                user,
                format!("transport:{provider}"),
                json!({ "status": "removed" }),
            ),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    get_mail_settings(State(state), extensions).await.map(|Json(s)| {
        Json(CredentialsView {
            postmark: s.outbound.credentials.postmark,
            smtp: s.outbound.credentials.smtp,
        })
    })
}

// --- combined settings view --------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct OutboundSettingsView {
    pub transport: String,
    pub from_address: String,
    pub from_name: String,
    pub reply_to: String,
    pub signature: String,
    /// The org's own greeting — what opens mail the workspace sends to its
    /// staff, and the fallback for a member with none (Wave 42).
    pub org_greeting: String,
    /// `allowed` | `org_locked`: whether members may sign with their own
    /// details. The UI renders a locked org's state rather than a disabled
    /// field with no reason.
    pub signature_policy: String,
    pub brand_name: String,
    pub logo_url: String,
    pub smtp_host: String,
    pub smtp_port: i64,
    pub smtp_username: String,
    pub postmark_base_url: String,
    /// Whether the SELECTED transport has a usable credential/endpoint
    /// combination (the honest not-configured bit the UI renders).
    pub ready: bool,
    /// Human reason when `ready` is false.
    pub not_ready_reason: Option<String>,
    pub credentials: CredentialsView,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BoardRouteView {
    pub board_id: Uuid,
    pub name: String,
    pub inbound_address: Option<String>,
    pub identity: BoardIdentityView,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReplyByEmailView {
    /// Inbound routing is what makes client replies land on tickets.
    pub enabled: bool,
    pub catchall_address: Option<String>,
    pub board_addresses: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MailSettingsResponse {
    pub inbound: InboundSettingsResponse,
    pub outbound: OutboundSettingsView,
    pub boards: Vec<BoardRouteView>,
    pub reply_by_email: ReplyByEmailView,
    pub digest_policy: String,
}

/// GET /api/email/settings — the whole mail surface in one read: inbound
/// routes + per-board identities, outbound identity + transport readiness
/// (never credential material), the delivery ledger's enablement bits, and
/// the org digest policy.
#[utoipa::path(operation_id = "email_settings_get", get, path = "/api/email/settings",
    tag = "email", security(("bearer" = [])),
    responses((status = 200, body = MailSettingsResponse)))]
pub async fn get_mail_settings(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<MailSettingsResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    let org = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;

    let inbound_row = core_db::emails::get_inbound_settings(&mut *tx, org).await?;
    let boards = core_db::emails::list_board_routes(&mut tx, org).await?;
    let mut board_views = Vec::with_capacity(boards.len());
    let mut board_addresses = 0_usize;
    for (board_id, name, inbound_address) in boards {
        if inbound_address.is_some() {
            board_addresses += 1;
        }
        let identity = core_db::emails::get_board_identity(&mut tx, org, board_id).await?;
        board_views.push(BoardRouteView {
            board_id,
            name,
            inbound_address,
            identity: identity.into(),
        });
    }

    let transport = as_text(&org_value_or_default(
        &mut tx,
        org,
        registry::EMAIL_TRANSPORT,
    )
    .await?)
    .unwrap_or_else(|| "none".into());
    let smtp_port = as_i64(
        &org_value_or_default(&mut tx, org, registry::EMAIL_SMTP_PORT).await?,
        587,
    );
    let smtp_host = as_text(&org_value_or_default(&mut tx, org, registry::EMAIL_SMTP_HOST).await?);
    let postmark_meta = core_db::byok::get_secret_meta(&mut tx, org, SEALED_POSTMARK_PROVIDER)
        .await?;
    let smtp_meta =
        core_db::byok::get_secret_meta(&mut tx, org, SEALED_SMTP_PROVIDER).await?;

    let (ready, not_ready_reason) = match transport.as_str() {
        "postmark" => match &postmark_meta {
            Some(_) => (true, None),
            None => (
                false,
                Some(
                    "the Postmark server token is not sealed yet — paste it below and save"
                        .to_string(),
                ),
            ),
        },
        "smtp" => match (&smtp_host, &smtp_meta) {
            (None, _) | (Some(_), None) => (
                false,
                Some(if smtp_host.is_none() {
                    "the SMTP host is not set".to_string()
                } else {
                    "the SMTP password is not sealed yet — paste it below and save".to_string()
                }),
            ),
            (Some(_), Some(_)) => (true, None),
        },
        _ => (
            false,
            Some("no outbound transport is selected — mail stays queued until one is".into()),
        ),
    };

    let view = MailSettingsResponse {
        inbound: InboundSettingsResponse {
            catchall_address: inbound_row.as_ref().map(|s| s.catchall_address.clone()),
            catchall_board_id: inbound_row.as_ref().map(|s| s.catchall_board_id),
            enabled: inbound_row.as_ref().is_some_and(|s| s.enabled),
        },
        outbound: OutboundSettingsView {
            transport,
            from_address: as_text(
                &org_value_or_default(&mut tx, org, registry::EMAIL_FROM_ADDRESS).await?,
            )
            .unwrap_or_default(),
            from_name: as_text(
                &org_value_or_default(&mut tx, org, registry::EMAIL_FROM_NAME).await?,
            )
            .unwrap_or_default(),
            reply_to: as_text(
                &org_value_or_default(&mut tx, org, registry::EMAIL_REPLY_TO).await?,
            )
            .unwrap_or_default(),
            signature: as_text(
                &org_value_or_default(&mut tx, org, registry::EMAIL_SIGNATURE).await?,
            )
            .unwrap_or_default(),
            org_greeting: as_text(
                &org_value_or_default(&mut tx, org, registry::EMAIL_ORG_GREETING).await?,
            )
            .unwrap_or_default(),
            signature_policy: crate::email::signature_policy(&mut *tx, org)
                .await?
                .as_str()
                .to_string(),
            brand_name: as_text(
                &org_value_or_default(&mut tx, org, registry::EMAIL_BRAND_NAME).await?,
            )
            .unwrap_or_default(),
            logo_url: as_text(&org_value_or_default(&mut tx, org, registry::EMAIL_LOGO_URL).await?)
                .unwrap_or_default(),
            smtp_host: smtp_host.unwrap_or_default(),
            smtp_port,
            smtp_username: as_text(
                &org_value_or_default(&mut tx, org, registry::EMAIL_SMTP_USERNAME).await?,
            )
            .unwrap_or_default(),
            postmark_base_url: as_text(
                &org_value_or_default(&mut tx, org, registry::EMAIL_POSTMARK_BASE_URL).await?,
            )
            .unwrap_or_default(),
            ready,
            not_ready_reason,
            credentials: CredentialsView {
                postmark: CredentialView {
                    configured: postmark_meta.is_some(),
                    updated_at: postmark_meta.map(|(_, at)| at.to_rfc3339()),
                },
                smtp: CredentialView {
                    configured: smtp_meta.is_some(),
                    updated_at: smtp_meta.map(|(_, at)| at.to_rfc3339()),
                },
            },
        },
        reply_by_email: ReplyByEmailView {
            enabled: inbound_row.as_ref().is_some_and(|s| s.enabled),
            catchall_address: inbound_row.as_ref().map(|s| s.catchall_address.clone()),
            board_addresses,
        },
        digest_policy: as_text(
            &org_value_or_default(&mut tx, org, registry::EMAIL_DIGEST_POLICY).await?,
        )
        .unwrap_or_else(|| "allowed".into()),
        boards: board_views,
    };
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(view))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PutOutboundSettingsRequest {
    #[serde(default)]
    pub transport: Option<String>,
    #[serde(default)]
    pub from_address: Option<String>,
    #[serde(default)]
    pub from_name: Option<String>,
    #[serde(default)]
    pub reply_to: Option<String>,
    #[serde(default)]
    pub signature: Option<String>,
    /// The org's own greeting (Wave 42).
    #[serde(default)]
    pub org_greeting: Option<String>,
    /// `allowed` | `org_locked` (Wave 42). Absent leaves the policy alone,
    /// so an older client that does not know the key cannot reset it.
    #[serde(default)]
    pub signature_policy: Option<String>,
    #[serde(default)]
    pub brand_name: Option<String>,
    #[serde(default)]
    pub logo_url: Option<String>,
    #[serde(default)]
    pub smtp_host: Option<String>,
    #[serde(default)]
    pub smtp_port: Option<i64>,
    #[serde(default)]
    pub smtp_username: Option<String>,
    #[serde(default)]
    pub postmark_base_url: Option<String>,
    #[serde(default)]
    pub digest_policy: Option<String>,
}

/// PUT /api/email/outbound-settings — partial update of the org outbound
/// identity, transport selection, and digest policy. Values are validated
/// against their registry declarations before any write.
#[utoipa::path(operation_id = "email_outbound_settings_put", put,
    path = "/api/email/outbound-settings", tag = "email",
    security(("bearer" = [])), request_body = PutOutboundSettingsRequest,
    responses((status = 200, body = MailSettingsResponse),
              (status = 422, description = "invalid value")))]
pub async fn put_outbound_settings(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<PutOutboundSettingsRequest>,
) -> ApiResult<Json<MailSettingsResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org = authed.principal.user.org_id;
    let user = authed.principal.user.id;

    // (registry key, JSON value) pairs, validated through the declarations.
    let mut updates: Vec<(&'static str, serde_json::Value)> = Vec::new();
    let mut push_text = |key: &'static str, raw: &Option<String>| -> ApiResult<()> {
        if let Some(value) = raw {
            let value = value.trim().to_string();
            validate_setting(key, &serde_json::json!(value), &mut updates)?;
        }
        Ok(())
    };
    push_text(registry::EMAIL_FROM_ADDRESS, &req.from_address)?;
    push_text(registry::EMAIL_FROM_NAME, &req.from_name)?;
    push_text(registry::EMAIL_REPLY_TO, &req.reply_to)?;
    push_text(registry::EMAIL_SIGNATURE, &req.signature)?;
    // The org greeting and the signature policy join the same editor and
    // the same save: the greeting, the org signature, and the rule that
    // says whether a member may have their own are one decision, so they
    // are one write.
    push_text(registry::EMAIL_ORG_GREETING, &req.org_greeting)?;
    push_text(registry::EMAIL_BRAND_NAME, &req.brand_name)?;
    push_text(registry::EMAIL_LOGO_URL, &req.logo_url)?;
    push_text(registry::EMAIL_SMTP_HOST, &req.smtp_host)?;
    push_text(registry::EMAIL_SMTP_USERNAME, &req.smtp_username)?;
    push_text(registry::EMAIL_POSTMARK_BASE_URL, &req.postmark_base_url)?;
    if let Some(port) = req.smtp_port {
        validate_setting(
            registry::EMAIL_SMTP_PORT,
            &serde_json::json!(port),
            &mut updates,
        )?;
    }
    if let Some(transport) = &req.transport {
        validate_setting(
            registry::EMAIL_TRANSPORT,
            &serde_json::json!(transport.trim()),
            &mut updates,
        )?;
    }
    if let Some(policy) = &req.digest_policy {
        validate_setting(
            registry::EMAIL_DIGEST_POLICY,
            &serde_json::json!(policy.trim()),
            &mut updates,
        )?;
    }
    if let Some(policy) = &req.signature_policy {
        validate_setting(
            registry::EMAIL_SIGNATURE_POLICY,
            &serde_json::json!(policy.trim()),
            &mut updates,
        )?;
    }
    // Mailbox-shaped text fields get the same shape gate the UI promises.
    for (key, value) in &updates {
        // A signature or greeting may only carry placeholders the send path
        // expands, checked on the write so the editor can name the field.
        if matches!(
            *key,
            registry::EMAIL_SIGNATURE | registry::EMAIL_ORG_GREETING
        ) {
            if let Some(text) = value.as_str() {
                crate::email::validate_signature_text(key, text)?;
            }
        }
        if matches!(
            *key,
            registry::EMAIL_FROM_ADDRESS | registry::EMAIL_REPLY_TO
        ) {
            if let Some(text) = value.as_str() {
                if !text.is_empty() && !is_mailbox(text) {
                    return Err(ApiError::Validation(format!("{key}: not a mailbox address")));
                }
            }
        }
    }

    crate::org_tx::with_org_tx(&state, org, user, |mut ctx| async move {
        for (key, value) in &updates {
            core_db::settings::upsert_org_setting(&mut ctx.tx, org, key, value, user).await?;
            ctx.emit(
                types::SETTING_UPDATED,
                "setting",
                Some(key),
                &json!({ "scope": "org", "key": key, "value": value }),
            )
            .await?;
        }
        core_db::audit::insert(
            &mut *ctx.tx,
            &audit_event(
                org,
                user,
                "outbound:settings".into(),
                json!({ "keys": updates.iter().map(|(k, _)| *k).collect::<Vec<_>>() }),
            ),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    get_mail_settings(State(state), extensions).await
}

/// Validates one (key, value) pair through the registry and appends it.
fn validate_setting(
    key: &'static str,
    value: &serde_json::Value,
    updates: &mut Vec<(&'static str, serde_json::Value)>,
) -> ApiResult<()> {
    let decl = registry::declaration(key)
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("undeclared key {key}")))?;
    let parsed = registry::SettingValue::from_json(value)
        .map_err(|e| ApiError::Validation(format!("{key}: {e}")))?;
    decl.validate(&parsed)
        .map_err(|e| ApiError::Validation(format!("{key}: {e}")))?;
    updates.push((key, parsed.to_json()));
    Ok(())
}

// --- delivery ledger ---------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct DeliveryLedgerRow {
    pub message_id: Uuid,
    pub rfc_message_id: String,
    pub subject: Option<String>,
    pub to_addresses: Vec<String>,
    pub from_address: String,
    pub status: String,
    pub attempts: i32,
    pub max_attempts: i32,
    pub last_error: Option<String>,
    pub ticket_id: Option<Uuid>,
    pub created_at: String,
    pub updated_at: String,
    /// Which rung of the greeting/signature chain supplied each block for
    /// THIS message (Wave 42), as the wire name plus the wording an admin
    /// reads. `None` is a row written before the lane recorded it, and it
    /// renders as "none" rather than a guess.
    pub greeting_source: Option<EmailSourceView>,
    pub signature_source: Option<EmailSourceView>,
}

/// One rung of the resolution chain, as the ledger reports it.
#[derive(Debug, Serialize, ToSchema)]
pub struct EmailSourceView {
    /// The stable wire name (`user_default`, `board_identity`, …).
    pub source: String,
    /// Ledger wording: "the sender", "the board", "the organization".
    pub label: String,
}

fn source_view(raw: Option<&str>) -> Option<EmailSourceView> {
    raw.map(|name| {
        let source = core_domain::email_signature::SignatureSource::parse(name);
        EmailSourceView {
            source: source.as_str().to_string(),
            label: source.admin_label().to_string(),
        }
    })
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeliveryLedgerResponse {
    pub rows: Vec<DeliveryLedgerRow>,
    /// Complete status set — zero counts included, so the summary strip
    /// never hides a state by omission.
    pub counts: DeliveryCounts,
    /// Opaque cursor for the next page; `None` = exhausted.
    pub next_cursor: Option<String>,
    /// False when no transport is configured: every row then sits queued.
    pub transport_configured: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeliveryCounts {
    pub queued: i64,
    pub sending: i64,
    pub sent: i64,
    pub dead: i64,
}

#[derive(Debug, Deserialize)]
pub struct DeliveryLedgerQuery {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub cursor: Option<String>,
}

const LEDGER_PAGE_DEFAULT: i64 = 25;
const LEDGER_PAGE_MAX: i64 = 100;

fn encode_ledger_cursor(created_at: chrono::DateTime<chrono::Utc>, id: Uuid) -> String {
    hex::encode(format!("{}|{}", created_at.to_rfc3339(), id))
}

fn decode_ledger_cursor(
    raw: &str,
) -> ApiResult<(chrono::DateTime<chrono::Utc>, Uuid)> {
    let bytes = hex::decode(raw)
        .map_err(|_| ApiError::Validation("cursor is not valid hex".into()))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| ApiError::Validation("cursor is not valid UTF-8".into()))?;
    let (ts, id) = text
        .split_once('|')
        .ok_or_else(|| ApiError::Validation("cursor is malformed".into()))?;
    let created_at = chrono::DateTime::parse_from_rfc3339(ts)
        .map_err(|_| ApiError::Validation("cursor timestamp is malformed".into()))?
        .with_timezone(&chrono::Utc);
    let id = Uuid::parse_str(id)
        .map_err(|_| ApiError::Validation("cursor id is malformed".into()))?;
    Ok((created_at, id))
}

/// GET /api/email/delivery-ledger — the outbound message ledger
/// (queued/sending/sent/dead) with attempts, last error, and counts.
#[utoipa::path(operation_id = "email_delivery_ledger_get", get,
    path = "/api/email/delivery-ledger", tag = "email",
    security(("bearer" = [])),
    params(
        ("status" = Option<String>, Query, description = "queued | sending | sent | dead"),
        ("limit" = Option<i64>, Query, description = "page size (default 25, max 100)"),
        ("cursor" = Option<String>, Query, description = "opaque next-page cursor"),
    ),
    responses((status = 200, body = DeliveryLedgerResponse)))]
pub async fn get_delivery_ledger(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(query): Query<DeliveryLedgerQuery>,
) -> ApiResult<Json<DeliveryLedgerResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    let org = authed.principal.user.org_id;
    if let Some(status) = query.status.as_deref() {
        if !["queued", "sending", "sent", "dead"].contains(&status) {
            return Err(ApiError::Validation(format!(
                "unknown status '{status}': expected queued, sending, sent, or dead"
            )));
        }
    }
    let limit = query
        .limit
        .unwrap_or(LEDGER_PAGE_DEFAULT)
        .clamp(1, LEDGER_PAGE_MAX);
    let before = query
        .cursor
        .as_deref()
        .map(decode_ledger_cursor)
        .transpose()?;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::emails::list_outbound_ledger(
        &mut tx,
        org,
        query.status.as_deref(),
        before,
        limit + 1,
    )
    .await?;
    let counts = core_db::emails::outbound_status_counts(&mut tx, org).await?;
    let transport = as_text(&org_value_or_default(
        &mut tx,
        org,
        registry::EMAIL_TRANSPORT,
    )
    .await?)
    .unwrap_or_else(|| "none".into());
    tx.commit().await.map_err(ApiError::from)?;

    let has_more = rows.len() as i64 > limit;
    let mut rows = rows;
    if has_more {
        rows.truncate(limit as usize);
    }
    let next_cursor = has_more
        .then(|| rows.last().map(|r| encode_ledger_cursor(r.created_at, r.message_id)))
        .flatten();
    let count_of = |status: &str| {
        counts
            .iter()
            .find(|(s, _)| s == status)
            .map(|(_, n)| *n)
            .unwrap_or(0)
    };
    Ok(Json(DeliveryLedgerResponse {
        rows: rows
            .into_iter()
            .map(|r| DeliveryLedgerRow {
                message_id: r.message_id,
                rfc_message_id: r.rfc_message_id,
                subject: r.subject,
                to_addresses: r.to_addresses,
                from_address: r.from_address,
                status: r.status,
                attempts: r.attempts,
                max_attempts: r.max_attempts,
                last_error: r.last_error,
                ticket_id: r.ticket_id,
                created_at: r.created_at.to_rfc3339(),
                updated_at: r.updated_at.to_rfc3339(),
                greeting_source: source_view(r.greeting_source.as_deref()),
                signature_source: source_view(r.signature_source.as_deref()),
            })
            .collect(),
        counts: DeliveryCounts {
            queued: count_of("queued"),
            sending: count_of("sending"),
            sent: count_of("sent"),
            dead: count_of("dead"),
        },
        next_cursor,
        transport_configured: transport != "none",
    }))
}

// --- test-send probe ---------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct TestSendRequest {
    /// Where the probe goes; absent sends to the caller's own address.
    #[serde(default)]
    pub to: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TestSendResponse {
    pub message_id: Uuid,
    pub to: String,
    pub transport: String,
    /// `queued` — the worker performs the actual send; the delivery ledger
    /// carries the outcome (never a fake synchronous "sent" here).
    pub outcome: String,
}

/// POST /api/email/test-send — renders a branded test message and enqueues
/// it through the configured transport. Refuses with a typed conflict when
/// no transport is configured (the honest not-configured state).
#[utoipa::path(operation_id = "email_test_send", post, path = "/api/email/test-send",
    tag = "email", security(("bearer" = [])), request_body = TestSendRequest,
    responses((status = 200, body = TestSendResponse),
              (status = 409, description = "transport not configured")))]
pub async fn test_send(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<TestSendRequest>,
) -> ApiResult<Json<TestSendResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org = authed.principal.user.org_id;
    let user = authed.principal.user.id;
    let to = match req.to.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        Some(raw) => require_mailbox("to", raw)?,
        None => authed.principal.user.email.trim().to_ascii_lowercase(),
    };

    // Readiness is decided before anything is enqueued so the operator gets
    // the reason immediately instead of a queued row that can never send.
    let (transport, identity, brand, reason) = {
        let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
        core_db::set_org_and_user(&mut tx, org, user)
            .await
            .map_err(ApiError::from)?;
        let transport = as_text(&org_value_or_default(&mut tx, org, registry::EMAIL_TRANSPORT).await?)
            .unwrap_or_else(|| "none".into());
        let postmark = core_db::byok::get_secret_meta(&mut tx, org, SEALED_POSTMARK_PROVIDER)
            .await?
            .is_some();
        let smtp_meta = core_db::byok::get_secret_meta(&mut tx, org, SEALED_SMTP_PROVIDER)
            .await?
            .is_some();
        let smtp_host =
            as_text(&org_value_or_default(&mut tx, org, registry::EMAIL_SMTP_HOST).await?);
        let reason = match transport.as_str() {
            "postmark" if !postmark => Some("the Postmark server token is not sealed"),
            "smtp" if smtp_host.is_none() => Some("the SMTP host is not set"),
            "smtp" if !smtp_meta => Some("the SMTP password is not sealed"),
            "none" => Some("no outbound transport is selected"),
            _ => None,
        };
        let identity = crate::email::resolve_mail_identity(&mut tx, org, None, &state.mail).await?;
        let brand = crate::email::resolve_brand(&mut tx, org).await?;
        // The probe resolves the envelope for the ADMIN who pressed the
        // button, not for the system: an admin checking "will my mail look
        // right" needs to see the signature that will actually go out,
        // which under `allowed` is their own. The greeting stays absent —
        // the probe's job is the transport and From identity.
        let envelope = crate::email::resolve_envelope(
            &mut tx,
            org,
            None,
            Some(user),
            &crate::email::EnvelopeOverrides::default(),
        )
        .await?;
        tx.commit().await.map_err(ApiError::from)?;
        (transport, identity, brand, envelope, reason)
    };
    if let Some(reason) = reason {
        return Err(ApiError::Conflict(format!(
            "mail is not configured: {reason}"
        )));
    }

    let rendered = core_mail::brand::render_test(
        &core_mail::brand::Brand {
            brand_name: &brand.brand_name,
            logo_url: brand.logo_url.as_deref(),
            greeting: None,
            signoff: envelope.signature.as_deref(),
        },
        &transport,
        &identity.from_address,
    );
    // The tx closure is `async move`; it takes its own copies of the
    // enqueue/log inputs while the response below still reports `to` and
    // `transport` (compile fix, Wave 34 lane 8: the fast-forwarded lane-8
    // commit used both after the move).
    let mail = state.mail.clone();
    let to_in_tx = to.clone();
    let transport_in_tx = transport.clone();
    let sources = crate::email::RenderedSources::of(&envelope);
    let message_id = crate::org_tx::with_org_tx(&state, org, user, |mut ctx| async move {
        let message_id = crate::email::enqueue_rendered_outbound(
            &mut ctx.tx,
            &mail,
            org,
            &to_in_tx,
            &rendered.subject,
            &rendered.text_body,
            None,
            sources,
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &audit_event(
                org,
                user,
                "outbound:test_send".into(),
                json!({ "to": to_in_tx, "transport": transport_in_tx, "message_id": message_id }),
            ),
        )
        .await?;
        Ok((ctx, message_id))
    })
    .await?;
    Ok(Json(TestSendResponse {
        message_id,
        to,
        transport,
        outcome: "queued".into(),
    }))
}
