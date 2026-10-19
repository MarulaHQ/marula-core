//! Custom-field endpoints (TASKLIST 2.17): definition CRUD per board and
//! typed value set/clear on tickets.
//!
//! Purpose: the HTTP surface for dynamic forms. Definitions are board-admin
//! gated (org `ManageBoards` + board `ConfigureBoard`, deny wins); ticket
//! values are validated against their definition through core-domain
//! `custom_fields` before storage, and every mutation emits its outbox event
//! in the same transaction (§4.2).
//!
//! Non-purpose: no kind-specific rendering decisions (the web client owns
//! that), no org-wide definition management (v1 scopes defs to boards).

use crate::auth::middleware::{authed, require_permission};
use crate::cache::{self, Caches};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use core_domain::custom_fields::{self, FieldConfig, FieldError, FieldKind};
use core_domain::events::types;
use core_domain::policy::{Module, Permission};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::board_access::authorize_board_op;
use core_domain::policy::BoardOp;

fn tickets(action: core_domain::policy::Action) -> Permission {
    Permission::Matrix {
        module: Module::Tickets,
        action,
    }
}

fn board_manage() -> Permission {
    Permission::ManageBoards
}

/// Cache key for a board's governing definitions. The set is org-wide defs plus
/// the board's own, identical for every member, so it caches per (org, board).
/// The `custom_field.*` listener flush is the `custom-field|<org>` prefix, which
/// covers this scoped key.
fn defs_for_board_key(org_id: Uuid, board_id: Uuid) -> String {
    Caches::ref_key_scoped(
        Caches::NS_CUSTOM_FIELD,
        org_id,
        &format!("board:{board_id}"),
    )
}

// --- wire shapes -------------------------------------------------------------

/// Wire view of one field definition; choice kinds carry their closed
/// option set so generated clients can render editors directly.
#[derive(Debug, Serialize, ToSchema)]
pub struct FieldDefView {
    pub id: Uuid,
    pub org_id: Uuid,
    /// Owning board (`None` = org-wide; v1 always creates board-scoped).
    pub board_id: Option<Uuid>,
    pub entity: String,
    pub name: String,
    pub kind: String,
    /// Closed option set for select/multiselect; empty otherwise.
    pub options: Vec<String>,
    pub required: bool,
    pub sort_order: i32,
}

impl From<core_db::custom_fields::FieldDefRow> for FieldDefView {
    fn from(row: core_db::custom_fields::FieldDefRow) -> Self {
        let config = FieldConfig::from_json(&row.config);
        FieldDefView {
            id: row.id,
            org_id: row.org_id,
            board_id: row.board_id,
            entity: row.entity,
            name: row.name,
            kind: row.kind,
            options: config.options,
            required: row.required,
            sort_order: row.sort_order,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateFieldDefRequest {
    pub name: String,
    pub kind: FieldKindWire,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub sort_order: i32,
    /// §4.6: registry key of a plugin field_validator entry
    /// (`plugin.<slug>.<id>`); stored in config and enforced on every
    /// value write (fail-closed when the entry disappears).
    #[serde(default)]
    pub validator_entry_key: Option<String>,
}

/// The definition kinds (mirror of migration 0009's CHECK set). Kept as a
/// string-newtype wire enum so unknown kinds fail validation, not deserialization.
#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FieldKindWire {
    Text,
    Number,
    Date,
    Select,
    Multiselect,
    Bool,
    Url,
    User,
}

impl From<FieldKindWire> for FieldKind {
    fn from(w: FieldKindWire) -> Self {
        match w {
            FieldKindWire::Text => FieldKind::Text,
            FieldKindWire::Number => FieldKind::Number,
            FieldKindWire::Date => FieldKind::Date,
            FieldKindWire::Select => FieldKind::Select,
            FieldKindWire::Multiselect => FieldKind::MultiSelect,
            FieldKindWire::Bool => FieldKind::Bool,
            FieldKindWire::Url => FieldKind::Url,
            FieldKindWire::User => FieldKind::User,
        }
    }
}

fn domain_kind(wire: FieldKindWire) -> Result<FieldKind, ApiError> {
    Ok(wire.into())
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateFieldDefRequest {
    pub name: Option<String>,
    /// Replaces the option set for choice kinds.
    pub options: Option<Vec<String>>,
    pub required: Option<bool>,
    pub sort_order: Option<i32>,
    /// §4.6: `Some(None)` clears the plugin validator; `Some(Some(key))`
    /// sets it; absent leaves it untouched.
    #[serde(default, deserialize_with = "double_option")]
    pub validator_entry_key: Option<Option<String>>,
}

/// Serde helper: distinguishes an absent key from an explicit `null`.
fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    serde::Deserialize::deserialize(de).map(Some)
}

/// One value assignment on a ticket create/patch: raw JSON validated against
/// its definition server-side.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct TicketFieldInput {
    pub field_id: Uuid,
    pub value: serde_json::Value,
}

/// One stored ticket value with its definition context.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TicketCustomFieldValue {
    pub field_id: Uuid,
    pub name: String,
    pub kind: String,
    pub required: bool,
    pub sort_order: i32,
    pub value: serde_json::Value,
}

impl TryFrom<core_db::custom_fields::FieldValueRow> for TicketCustomFieldValue {
    type Error = FieldError;

    fn try_from(row: core_db::custom_fields::FieldValueRow) -> Result<Self, FieldError> {
        let kind: FieldKind = row
            .kind
            .parse()
            .map_err(|_| FieldError::TypeMismatch { kind: "unknown" })?;
        // Values were canonical when stored; re-parsing here guarantees the
        // wire payload is valid against the CURRENT definition config.
        let config = FieldConfig::from_json(&row.config);
        let parsed = custom_fields::parse_value(kind, &config, &row.value)?;
        Ok(TicketCustomFieldValue {
            field_id: row.field_id,
            name: row.name,
            kind: row.kind,
            required: row.required,
            sort_order: row.sort_order,
            value: parsed.to_json(),
        })
    }
}

/// A ticket payload enriched with its custom-field values (detail + list).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TicketWithFields {
    #[serde(flatten)]
    pub ticket: core_domain::entities::Ticket,
    pub custom_fields: Vec<TicketCustomFieldValue>,
    /// The ticket's site zone, when the site names one (Wave 50 lane T2).
    ///
    /// The record-zone chain is site → org default → UTC, and the server ships
    /// the site's raw value only: the org default is the registry setting
    /// `general.default_timezone`, which every member may already read, so the
    /// fallback is resolved once in the client instead of being frozen into
    /// every row that leaves this endpoint.
    #[schema(nullable = true)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site_timezone: Option<String>,
}

impl TicketWithFields {
    pub fn new(
        ticket: core_domain::entities::Ticket,
        values: Vec<TicketCustomFieldValue>,
        site_timezone: Option<String>,
    ) -> Self {
        TicketWithFields {
            ticket,
            custom_fields: values,
            site_timezone,
        }
    }

    /// Enriches one ticket with its values and its site zone.
    ///
    /// Takes the connection rather than a generic executor because it runs two
    /// reads — the values and the site — inside the caller's transaction.
    pub async fn load(
        conn: &mut sqlx::PgConnection,
        org_id: Uuid,
        ticket: core_domain::entities::Ticket,
    ) -> ApiResult<Self> {
        let site_timezone = site_zone(&mut *conn, org_id, ticket.site_id).await?;
        let rows = core_db::custom_fields::list_values(&mut *conn, org_id, ticket.id)
            .await
            .map_err(ApiError::from)?;
        Ok(Self::new(ticket, project_values(rows)?, site_timezone))
    }

    /// Enriches a page of tickets with ONE batched values query and ONE batched
    /// site query, however many sites the page touches.
    pub async fn load_many(
        conn: &mut sqlx::PgConnection,
        org_id: Uuid,
        tickets: Vec<core_domain::entities::Ticket>,
    ) -> ApiResult<Vec<Self>> {
        let ids: Vec<Uuid> = tickets.iter().map(|t| t.id).collect();
        let site_ids: Vec<Uuid> = tickets.iter().filter_map(|t| t.site_id).collect();
        let grouped = core_db::custom_fields::list_values_many(&mut *conn, org_id, &ids)
            .await
            .map_err(ApiError::from)?;
        let zones = site_zones(&mut *conn, org_id, &site_ids).await?;
        let mut out = Vec::with_capacity(tickets.len());
        for ticket in tickets {
            let rows = grouped.get(&ticket.id).cloned().unwrap_or_default();
            let zone = ticket
                .site_id
                .and_then(|id| zones.get(&id).cloned().flatten());
            out.push(Self::new(ticket, project_values(rows)?, zone));
        }
        Ok(out)
    }
}

/// One site's zone, when the ticket has a site and the site names one.
pub(crate) async fn site_zone(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    site_id: Option<Uuid>,
) -> ApiResult<Option<String>> {
    let Some(site_id) = site_id else {
        return Ok(None);
    };
    let mut zones = site_zones(conn, org_id, &[site_id]).await?;
    Ok(zones.remove(&site_id).flatten())
}

/// Site id → its zone, for the sites that name one.
async fn site_zones(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    site_ids: &[Uuid],
) -> ApiResult<std::collections::HashMap<Uuid, Option<String>>> {
    if site_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let sites = core_db::clients::get_sites_by_ids(&mut *conn, org_id, site_ids)
        .await
        .map_err(ApiError::from)?;
    Ok(sites.into_iter().map(|s| (s.id, s.timezone)).collect())
}

fn project_values(
    rows: Vec<core_db::custom_fields::FieldValueRow>,
) -> ApiResult<Vec<TicketCustomFieldValue>> {
    rows.into_iter()
        .map(TicketCustomFieldValue::try_from)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ApiError::Validation(e.to_string()))
}

// --- helpers ------------------------------------------------------------------

/// Loads the def and verifies it governs this ticket's board (org-wide or
/// owned by it). Returns the parsed domain kind + config.
async fn governing_def(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    field_id: Uuid,
    board_id: Uuid,
) -> ApiResult<(core_db::custom_fields::FieldDefRow, FieldKind, FieldConfig)> {
    let def = core_db::custom_fields::get_def(conn, org_id, field_id)
        .await
        .map_err(|e| match e {
            core_db::DbError::NotFound => ApiError::NotFound,
            other => ApiError::Internal(other.into()),
        })?;
    if def.entity != "ticket" || def.board_id.map(|b| b != board_id).unwrap_or(false) {
        return Err(ApiError::NotFound);
    }
    let kind: FieldKind = def
        .kind
        .parse()
        .map_err(|_| ApiError::Validation("definition has an unknown kind".into()))?;
    let config = FieldConfig::from_json(&def.config);
    Ok((def, kind, config))
}

/// §4.6: shape-checks a validator registry key and returns the config
/// json with the key set (or removed when `key` is None).
fn config_with_validator(
    base: &serde_json::Value,
    key: Option<&String>,
) -> ApiResult<serde_json::Value> {
    let mut config = base.clone();
    match key {
        Some(key) => {
            let ok = key.starts_with("plugin.")
                && key.chars().all(|c| {
                    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_' || c == '-'
                });
            if !ok {
                return Err(ApiError::Validation(
                    "validator_entry_key must be a plugin.<slug>.<id> registry key".into(),
                ));
            }
            config["validator_entry_key"] = serde_json::json!(key);
        }
        None => {
            if let Some(obj) = config.as_object_mut() {
                obj.remove("validator_entry_key");
            }
        }
    }
    Ok(config)
}

fn def_error(err: FieldError) -> ApiError {
    ApiError::Validation(err.to_string())
}

// --- shared application helpers ----------------------------------------------

/// Validates + stores a batch of field inputs against the definitions
/// governing `board_id`, returning the stored wire views in input order.
/// Unknown or foreign field ids are 404s; bad values are 422s.
pub(crate) async fn apply_ticket_field_inputs(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    board_id: Uuid,
    ticket_id: Uuid,
    inputs: &[TicketFieldInput],
) -> ApiResult<Vec<TicketCustomFieldValue>> {
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    let defs = core_db::custom_fields::list_defs_for_board(&mut *conn, org_id, board_id)
        .await
        .map_err(ApiError::from)?;
    let mut stored = Vec::with_capacity(inputs.len());
    for input in inputs {
        let def = defs
            .iter()
            .find(|d| d.id == input.field_id)
            .ok_or(ApiError::NotFound)?;
        let kind: FieldKind = def
            .kind
            .parse()
            .map_err(|_| ApiError::Validation("definition has an unknown kind".into()))?;
        let config = FieldConfig::from_json(&def.config);
        let parsed = custom_fields::parse_value(kind, &config, &input.value).map_err(def_error)?;
        let canonical = parsed.to_json();
        core_db::custom_fields::upsert_value(
            &mut *conn,
            org_id,
            input.field_id,
            ticket_id,
            &canonical,
        )
        .await?;
        stored.push(TicketCustomFieldValue {
            field_id: def.id,
            name: def.name.clone(),
            kind: def.kind.clone(),
            required: def.required,
            sort_order: def.sort_order,
            value: canonical,
        });
    }
    Ok(stored)
}

/// 422 naming every REQUIRED definition of the board that has no stored
/// value on the ticket — the create-time completeness gate.
pub(crate) async fn enforce_required_present(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    board_id: Uuid,
    ticket_id: Uuid,
) -> ApiResult<()> {
    let defs = core_db::custom_fields::list_defs_for_board(&mut *conn, org_id, board_id)
        .await
        .map_err(ApiError::from)?;
    let required: Vec<&core_db::custom_fields::FieldDefRow> =
        defs.iter().filter(|d| d.required).collect();
    if required.is_empty() {
        return Ok(());
    }
    let values = core_db::custom_fields::list_values(&mut *conn, org_id, ticket_id)
        .await
        .map_err(ApiError::from)?;
    let filled: std::collections::HashSet<Uuid> = values.iter().map(|v| v.field_id).collect();
    let missing: Vec<String> = required
        .iter()
        .filter(|d| !filled.contains(&d.id))
        .map(|d| d.name.clone())
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(ApiError::Validation(format!(
        "missing required fields: {}",
        missing.join(", ")
    )))
}

// --- definition endpoints -----------------------------------------------------

/// GET /api/boards/{id}/fields — active ticket definitions governing this
/// board (org-wide plus board-owned), sort-order stable.
#[utoipa::path(operation_id = "fields_list_for_board", get, path = "/api/boards/{id}/fields", tag = "fields",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 200, body = [FieldDefView])))]
pub async fn list_defs(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<FieldDefView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let org_id = authed.principal.user.org_id;
    // The board's governing definitions are the same for every viewer of the
    // board, so a cache hit skips the transaction. A foreign board is never
    // cached: the org-scoped key misses, the RLS read 404s below, and nothing
    // is stored.
    let key = defs_for_board_key(org_id, id);
    if let Some(defs) = cache::caches().get_ref::<Vec<core_db::custom_fields::FieldDefRow>>(&key) {
        return Ok(Json(defs.into_iter().map(FieldDefView::from).collect()));
    }
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    // The path board must belong to the caller's org (foreign ids 404).
    core_db::boards::get_board(&mut *tx, org_id, id)
        .await
        .map_err(ApiError::from)?;
    let defs = core_db::custom_fields::list_defs_for_board(&mut *tx, org_id, id)
        .await
        .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    cache::caches().put_ref(key, defs.clone());
    Ok(Json(defs.into_iter().map(FieldDefView::from).collect()))
}

/// POST /api/boards/{id}/fields — creates a board-scoped definition
/// (board-admin gated); emits `custom_field.def_created`.
#[utoipa::path(operation_id = "fields_create_def", post, path = "/api/boards/{id}/fields", tag = "fields",
    security(("bearer" = [])), params(("id" = Uuid, Path)), request_body = CreateFieldDefRequest,
    responses((status = 200, body = FieldDefView),
              (status = 422, description = "invalid definition")))]
pub async fn create_def(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateFieldDefRequest>,
) -> ApiResult<Json<FieldDefView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, board_manage())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let kind = domain_kind(req.kind)?;
    let config = FieldConfig {
        options: req.options.clone(),
    };
    let (name, config) =
        custom_fields::validate_def(&req.name, kind, &config).map_err(def_error)?;
    let mut config_json = config.to_json();
    if req.validator_entry_key.is_some() {
        config_json = config_with_validator(&config_json, req.validator_entry_key.as_ref())?;
    }

    let def = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        // The path board must belong to the caller's org (foreign ids 404).
        core_db::boards::get_board(&mut *ctx.tx, org_id, id)
            .await
            .map_err(ApiError::from)?;
        authorize_board_op(&mut ctx.tx, &authed.principal, id, BoardOp::ConfigureBoard).await?;
        let row = core_db::custom_fields::insert_def(
            &mut ctx.tx,
            &core_db::custom_fields::FieldDefRow {
                id: Uuid::now_v7(),
                org_id,
                board_id: Some(id),
                entity: "ticket".into(),
                name: name.clone(),
                kind: kind.as_str().to_string(),
                config: config_json,
                required: req.required,
                sort_order: req.sort_order,
            },
        )
        .await?;
        ctx.emit(
            types::CUSTOM_FIELD_DEF_CREATED,
            "custom_field_def",
            Some(row.id.to_string().as_str()),
            &serde_json::json!({
                "field_id": row.id,
                "board_id": id,
                "name": row.name,
                "kind": row.kind,
                "required": row.required,
            }),
        )
        .await?;
        Ok((ctx, row))
    })
    .await?;

    Ok(Json(FieldDefView::from(def)))
}

/// PATCH /api/boards/{id}/fields/{field_id} — partial edit of a board-owned
/// definition. Kind is immutable (it would orphan stored values); emit
/// `custom_field.def_updated` on success.
#[utoipa::path(operation_id = "fields_update_def", patch, path = "/api/boards/{id}/fields/{field_id}", tag = "fields",
    security(("bearer" = [])), params(("id" = Uuid, Path), ("field_id" = Uuid, Path)),
    request_body = UpdateFieldDefRequest,
    responses((status = 200, body = FieldDefView)))]
pub async fn update_def(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, field_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateFieldDefRequest>,
) -> ApiResult<Json<FieldDefView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, board_manage())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let def = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        // The path board must belong to the caller's org (foreign ids 404).
        core_db::boards::get_board(&mut *ctx.tx, org_id, id)
            .await
            .map_err(ApiError::from)?;
        authorize_board_op(&mut ctx.tx, &authed.principal, id, BoardOp::ConfigureBoard).await?;
        let current = core_db::custom_fields::get_def(&mut ctx.tx, org_id, field_id)
            .await
            .map_err(|e| match e {
                core_db::DbError::NotFound => ApiError::NotFound,
                other => ApiError::Internal(other.into()),
            })?;
        if current.board_id != Some(id) {
            return Err(ApiError::NotFound);
        }
        let kind: FieldKind = current
            .kind
            .parse()
            .map_err(|_| ApiError::Validation("definition has an unknown kind".into()))?;

        let name = req.name.as_deref().map(str::trim);
        if let Some(name) = name {
            let (_, _) = custom_fields::validate_def(name, kind, &FieldConfig::default())
                .map_err(def_error)?;
        }
        let mut config_json = match (&req.options, kind.is_choice()) {
            (Some(options), true) => {
                let config = FieldConfig {
                    options: options.clone(),
                };
                let (_, normalized) =
                    custom_fields::validate_def(&current.name, kind, &config).map_err(def_error)?;
                Some(normalized.to_json())
            }
            (Some(_), false) => {
                return Err(ApiError::Validation(
                    "only select/multiselect fields take options".into(),
                ));
            }
            (None, _) => None,
        };
        if let Some(validator) = &req.validator_entry_key {
            let base = config_json
                .clone()
                .unwrap_or_else(|| current.config.clone());
            config_json = Some(config_with_validator(&base, validator.as_ref())?);
        }

        let updated = core_db::custom_fields::update_def(
            &mut ctx.tx,
            org_id,
            field_id,
            &core_db::custom_fields::DefPatch {
                name,
                config: config_json.as_ref(),
                required: req.required,
                sort_order: req.sort_order,
            },
        )
        .await?;
        ctx.emit(
            types::CUSTOM_FIELD_DEF_UPDATED,
            "custom_field_def",
            Some(updated.id.to_string().as_str()),
            &serde_json::json!({
                "field_id": updated.id,
                "board_id": id,
                "name": updated.name,
            }),
        )
        .await?;
        Ok((ctx, updated))
    })
    .await?;

    Ok(Json(FieldDefView::from(def)))
}

/// DELETE /api/boards/{id}/fields/{field_id} — removes the definition and
/// cascades its stored values; emits `custom_field.def_removed`.
#[utoipa::path(operation_id = "fields_delete_def", delete, path = "/api/boards/{id}/fields/{field_id}", tag = "fields",
    security(("bearer" = [])), params(("id" = Uuid, Path), ("field_id" = Uuid, Path)),
    responses((status = 200)))]
pub async fn delete_def(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, field_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, board_manage())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        // The path board must belong to the caller's org (foreign ids 404).
        core_db::boards::get_board(&mut *ctx.tx, org_id, id)
            .await
            .map_err(ApiError::from)?;
        authorize_board_op(&mut ctx.tx, &authed.principal, id, BoardOp::ConfigureBoard).await?;
        let deleted = core_db::custom_fields::delete_def(&mut ctx.tx, org_id, field_id).await;
        match deleted {
            Ok(()) => {}
            Err(core_db::DbError::NotFound) => return Err(ApiError::NotFound),
            Err(other) => return Err(ApiError::Internal(other.into())),
        }
        ctx.emit(
            types::CUSTOM_FIELD_DEF_REMOVED,
            "custom_field_def",
            Some(field_id.to_string().as_str()),
            &serde_json::json!({ "field_id": field_id, "board_id": id }),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;

    Ok(Json(serde_json::json!({ "deleted": field_id })))
}

// --- ticket value endpoints ---------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetTicketFieldValueRequest {
    pub value: serde_json::Value,
}

/// PUT /api/tickets/{id}/fields/{field_id} — validates the incoming JSON
/// against the definition (kind coercion, option membership) and stores the
/// canonical encoding; emits `ticket.field_set`.
#[utoipa::path(operation_id = "fields_set_ticket_value", put, path = "/api/tickets/{id}/fields/{field_id}", tag = "fields",
    security(("bearer" = [])), params(("id" = Uuid, Path), ("field_id" = Uuid, Path)),
    request_body = SetTicketFieldValueRequest,
    responses((status = 200, body = TicketCustomFieldValue),
              (status = 422, description = "value does not validate against the definition")))]
pub async fn set_ticket_value(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, field_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<SetTicketFieldValueRequest>,
) -> ApiResult<Json<TicketCustomFieldValue>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let stored = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await?;
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            ticket.board_id,
            BoardOp::WorkTicket,
        )
        .await?;
        let (def, kind, config) =
            governing_def(&mut ctx.tx, org_id, field_id, ticket.board_id).await?;
        let parsed = custom_fields::parse_value(kind, &config, &req.value).map_err(def_error)?;
        let canonical = parsed.to_json();
        // §4.6: a configured plugin validator runs on EVERY write,
        // fail-closed — an unavailable validator blocks the write rather
        // than waving it through.
        if let Some(key) = def
            .config
            .get("validator_entry_key")
            .and_then(|v| v.as_str())
        {
            let entry = core_db::plugin_surface::get_entry_by_key(
                &mut *ctx.tx,
                org_id,
                "field_validator",
                key,
            )
            .await
            .map_err(ApiError::from)?;
            let schema_value = entry
                .as_ref()
                .and_then(|e| e.definition.get("schema"))
                .cloned();
            let Some(schema_value) = schema_value else {
                return Err(ApiError::Validation(format!(
                    "field validator {key} is not currently available"
                )));
            };
            let schema = core_domain::entities::plugins::SettingsSchema::new(schema_value.clone())
                .map_err(|e| ApiError::Validation(e.to_string()))?;
            let wrapped = serde_json::json!({ "value": canonical });
            schema
                .validate_config(&wrapped)
                .map_err(|e| ApiError::Validation(e.to_string()))?;
        }
        core_db::custom_fields::upsert_value(&mut ctx.tx, org_id, field_id, id, &canonical).await?;
        ctx.emit(
            types::TICKET_FIELD_SET,
            "ticket",
            Some(id.to_string().as_str()),
            &serde_json::json!({
                "ticket_id": id,
                "field_id": field_id,
                "value": canonical,
            }),
        )
        .await?;
        Ok((
            ctx,
            TicketCustomFieldValue {
                field_id,
                name: def.name,
                kind: def.kind,
                required: def.required,
                sort_order: def.sort_order,
                value: canonical,
            },
        ))
    })
    .await?;

    Ok(Json(stored))
}

/// DELETE /api/tickets/{id}/fields/{field_id} — clears one value; refuses
/// (422) to empty a REQUIRED definition; emits `ticket.field_cleared`.
#[utoipa::path(operation_id = "fields_clear_ticket_value", delete, path = "/api/tickets/{id}/fields/{field_id}", tag = "fields",
    security(("bearer" = [])), params(("id" = Uuid, Path), ("field_id" = Uuid, Path)),
    responses((status = 200),
              (status = 422, description = "field is required and cannot be emptied")))]
pub async fn clear_ticket_value(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, field_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await?;
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            ticket.board_id,
            BoardOp::WorkTicket,
        )
        .await?;
        let (def, _, _) = governing_def(&mut ctx.tx, org_id, field_id, ticket.board_id).await?;
        if def.required {
            return Err(ApiError::Validation(format!(
                "`{}` is required and cannot be cleared",
                def.name
            )));
        }
        let removed =
            core_db::custom_fields::clear_value(&mut ctx.tx, org_id, field_id, id).await?;
        if removed {
            ctx.emit(
                types::TICKET_FIELD_CLEARED,
                "ticket",
                Some(id.to_string().as_str()),
                &serde_json::json!({ "ticket_id": id, "field_id": field_id }),
            )
            .await?;
        }
        Ok((ctx, ()))
    })
    .await?;

    Ok(Json(serde_json::json!({ "cleared": field_id })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_db::custom_fields::FieldDefRow;

    fn a_def(org_id: Uuid, board_id: Uuid) -> FieldDefRow {
        FieldDefRow {
            id: Uuid::new_v4(),
            org_id,
            board_id: Some(board_id),
            entity: "ticket".into(),
            name: "Severity".into(),
            kind: "select".into(),
            config: serde_json::json!({ "options": ["low", "high"] }),
            required: false,
            sort_order: 0,
        }
    }

    #[test]
    fn board_defs_are_served_from_the_cache_then_flushed_by_a_custom_field_event() {
        let _guard = cache::only_one_at_a_time();
        let caches = cache::__enable_for_tests();
        cache::__reset_for_tests();
        let org = Uuid::new_v4();
        let board = Uuid::new_v4();
        let key = defs_for_board_key(org, board);
        let value = vec![a_def(org, board)];

        // First read: a miss, so the handler runs its query and fills the cache.
        assert!(caches.get_ref::<Vec<FieldDefRow>>(&key).is_none());
        caches.put_ref(key.clone(), value.clone());
        // Second read: served from the cache; the transaction is skipped.
        let got = caches
            .get_ref::<Vec<FieldDefRow>>(&key)
            .expect("served from the cache");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, value[0].id);

        // The `custom_field.*` flush is the `custom-field|<org>` prefix, which
        // covers this board-scoped key.
        caches.invalidate_refs(&Caches::ref_key(Caches::NS_CUSTOM_FIELD, org));
        assert!(caches.get_ref::<Vec<FieldDefRow>>(&key).is_none());
    }

    #[test]
    fn a_custom_field_event_for_another_org_leaves_this_orgs_defs_cached() {
        let _guard = cache::only_one_at_a_time();
        let caches = cache::__enable_for_tests();
        cache::__reset_for_tests();
        let org = Uuid::new_v4();
        let board = Uuid::new_v4();
        let other = Uuid::new_v4();
        let key = defs_for_board_key(org, board);
        caches.put_ref(key.clone(), vec![a_def(org, board)]);

        caches.invalidate_refs(&Caches::ref_key(Caches::NS_CUSTOM_FIELD, other));

        assert!(caches.get_ref::<Vec<FieldDefRow>>(&key).is_some());
    }

    #[test]
    fn a_board_flush_does_not_clear_the_custom_field_family() {
        let _guard = cache::only_one_at_a_time();
        let caches = cache::__enable_for_tests();
        cache::__reset_for_tests();
        let org = Uuid::new_v4();
        let board = Uuid::new_v4();
        let key = defs_for_board_key(org, board);
        caches.put_ref(key.clone(), vec![a_def(org, board)]);

        // A board event clears `board|<org>`; the custom-field family survives.
        caches.invalidate_refs(&Caches::ref_key(Caches::NS_BOARD, org));

        assert!(caches.get_ref::<Vec<FieldDefRow>>(&key).is_some());
    }
}
