//! The connector read surfaces (ADR 0032, ADR 0033; `connector-surface.md`
//! §6.7): one resolver payload for every contextual surface, and the
//! installation-first settings roster.
//!
//! Why one endpoint and not one per surface: the chips on a configuration, the
//! linked-records strip on a ticket and the action on a recommendation are the
//! same question — "where can I act on this record" — and a second endpoint
//! would be a second answer that drifts. The connector contributes the
//! declaration; this route renders it, so a marketplace connector reaches all
//! three surfaces on the day it installs.
//!
//! Two decisions worth stating because they are security decisions, not
//! formatting:
//!
//! * an unknown `record_type` is a typed 422 naming the accepted set — never a
//!   500 and never a silent default;
//! * a record the caller may not read is a 404, not a 403. The API refuses to
//!   disclose that the record exists, the same posture the record's own route
//!   takes for a restricted client.
//!
//! Non-purpose: no action is EXECUTED here. `launch` is a URL the browser
//! opens (the deterministic form the manifest can express today); the audited
//! adapter call is a later capability and will not live in this read path.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use core_db::connector_connections::{
    client_display_name, connection_usage, count_enabled_bindings, create_binding,
    create_connection, delete_binding, delete_capability_default, delete_connection,
    get_connection, get_connection_secrets, get_installation_pin, list_bindings,
    resolve_external_actions, resolve_record_scope, unknown_roles, update_binding,
    update_connection, upsert_capability_default, Binding, BindingUpdate, Connection,
    ConnectionRosterRow, ConnectionUpdate, ConnectionViewer, NewBinding, NewConnection,
};
use core_domain::connectors::{ProductKind, RecordType};
use core_domain::entities::plugins::SettingsSchema;
use core_domain::policy::{check, Action, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use uuid::Uuid;

/// The record types this route can authorise today.
///
/// Closed on purpose: a record type Core cannot show has no chip, so accepting
/// one here would store a link into a surface that does not exist. The
/// vocabulary is wider than this list (`connectors::RecordType::ALL`); the
/// difference is deliberate and is what the refusal names.
const SUPPORTED: &[RecordType] = &[
    RecordType::Configuration,
    RecordType::Ticket,
    RecordType::Invoice,
    RecordType::Contact,
    RecordType::Client,
];

/// The permission module each supported record's own detail route asks for.
///
/// Reusing the route's gate rather than inventing a connector permission is
/// what keeps a chip from becoming a side door: if the caller cannot open the
/// record, they cannot ask what can be done with it.
fn record_module(kind: RecordType) -> Option<Module> {
    match kind {
        RecordType::Configuration | RecordType::Client | RecordType::Contact => {
            Some(Module::Clients)
        }
        RecordType::Ticket => Some(Module::Tickets),
        RecordType::Invoice => Some(Module::Commercial),
        _ => None,
    }
}

// --- GET /api/records/{record_type}/{record_id}/external-actions ------------

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ExternalActionsResponse {
    pub capabilities: Vec<CapabilityView>,
}

/// One capability's providers on this record — the chip row's unit.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CapabilityView {
    /// An open product kind (`rmm`, `remote_access`, …, or one nobody planned).
    pub kind: String,
    pub providers: Vec<ProviderView>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProviderView {
    #[schema(value_type = uuid::Uuid)]
    pub connection_id: Uuid,
    /// The installed manifest's name.
    pub connector: String,
    /// The operator's label for the connection, so two controllers of one
    /// vendor are distinguishable.
    pub label: String,
    /// `connected` | `needs_attachment` | `auth_failed` | `degraded` |
    /// `disabled` (never `disabled` here — the resolver skips those).
    pub status: String,
    /// True when this is the capability's default for the record's client, or
    /// the only provider. False with several providers and no default: the UI
    /// opens the menu rather than guessing.
    pub is_primary: bool,
    pub actions: Vec<ActionView>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ActionView {
    /// `open` | `launch`.
    pub kind: String,
    pub label: String,
    #[schema(value_type = Option<String>)]
    pub url: Option<String>,
}

/// GET /api/records/{record_type}/{record_id}/external-actions — every
/// provider that can act on this record, grouped by capability.
#[utoipa::path(operation_id = "records_external_actions", get,
    path = "/api/records/{record_type}/{record_id}/external-actions",
    tag = "connectors",
    security(("bearer" = [])),
    params(
        ("record_type" = String, Path, description = "configuration | client | contact | ticket | invoice"),
        ("record_id" = Uuid, Path),
    ),
    responses(
        (status = 200, body = ExternalActionsResponse),
        (status = 404, description = "the record does not exist or is not visible to the caller"),
        (status = 422, description = "unknown or unsupported record type"),
    ))]
pub async fn external_actions(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((record_type, record_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<ExternalActionsResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let kind = parse_supported_record_type(&record_type)?;
    let module = record_module(kind).ok_or_else(|| unsupported_record_type(&record_type))?;
    let org_id = authed.principal.user.org_id;

    // The module gate is the record's own gate. A refusal is a 404, not a 403:
    // "you may not read this" and "this does not exist" must be the same
    // answer here or the endpoint becomes an existence oracle.
    if !check(
        &authed.principal.permissions,
        Permission::Matrix {
            module,
            action: Action::View,
        },
    )
    .is_allowed()
    {
        return Err(ApiError::NotFound);
    }

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;

    // Absence and invisibility collapse to one answer before any work is done.
    let scope = match resolve_record_scope(&mut tx, org_id, kind, record_id).await {
        Ok(Some(scope)) => scope,
        Ok(None) => {
            // The record type has no client scope this build resolves, but it
            // also does not prove the record exists. A route only reaches here
            // for a supported type, all of which do.
            return Err(ApiError::NotFound);
        }
        Err(core_db::DbError::NotFound) => return Err(ApiError::NotFound),
        Err(e) => return Err(e.into()),
    };
    // Same per-client visibility rule the client route applies. Admins bypass
    // it, exactly as they do there.
    if !scope.restricted_to.is_empty()
        && !scope.restricted_to.contains(&authed.principal.user.id)
        && !authed.principal.is_owner_or_admin()
    {
        return Err(ApiError::NotFound);
    }

    let capabilities =
        resolve_external_actions(&mut tx, org_id, kind, record_id, &viewer_of(&authed)).await?;
    tx.commit().await.map_err(ApiError::from)?;

    Ok(Json(ExternalActionsResponse {
        capabilities: capabilities
            .into_iter()
            .map(|capability| CapabilityView {
                kind: capability.kind,
                providers: capability
                    .providers
                    .into_iter()
                    .map(|provider| ProviderView {
                        connection_id: provider.connection_id,
                        connector: provider.connector,
                        label: provider.label,
                        status: provider.status.as_str().to_string(),
                        is_primary: provider.is_primary,
                        actions: provider
                            .actions
                            .into_iter()
                            .map(|action| ActionView {
                                kind: action.kind,
                                label: action.label,
                                url: action.url,
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect(),
    }))
}

/// Parses a record type against the closed vocabulary, refusing with the set
/// this route accepts rather than the set the schema allows.
fn parse_supported_record_type(raw: &str) -> ApiResult<RecordType> {
    let trimmed = raw.trim();
    let kind = RecordType::parse(trimmed).ok_or_else(|| unsupported_record_type(trimmed))?;
    if !SUPPORTED.contains(&kind) {
        return Err(unsupported_record_type(trimmed));
    }
    Ok(kind)
}

fn unsupported_record_type(raw: &str) -> ApiError {
    let allowed = SUPPORTED
        .iter()
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    ApiError::Validation(format!(
        "unknown record type `{raw}` (supported: {allowed})"
    ))
}

// --- GET /api/connectors/roster ---------------------------------------------

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ConnectorRosterResponse {
    pub connectors: Vec<ConnectorRosterView>,
}

/// One installed connector, with its configured uses nested.
///
/// The installation is the unit because the first-run state is an install with
/// zero connections; an entry with `connections: []` is the card whose action
/// is `Connect`, not a missing row.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ConnectorRosterView {
    #[schema(value_type = uuid::Uuid)]
    pub installation_id: Uuid,
    /// The pinned manifest's name (the catalog slug when the version is gone).
    pub connector: String,
    /// The pinned manifest's own one-line purpose, verbatim. Empty when it
    /// declares none; Core never re-authors it.
    pub description: String,
    /// What kind of vendor this is. Empty means the connector declared none and
    /// contributes no contextual chip.
    pub product_kinds: Vec<String>,
    pub connections: Vec<ConnectionView>,
}

/// One connection inside a connector's roster entry.
///
/// No status rollup is computed here or in SQL: the UI derives "worst wins"
/// and "1 of 4" from these rows, and a second answer would be a second place
/// to disagree with it.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ConnectionView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub installation_id: Uuid,
    /// The installed manifest's name, repeated so per-connection rendering
    /// does not have to walk back to the parent.
    pub connector: String,
    pub label: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub scope_client_id: Option<Uuid>,
    /// The scoped client's name. `None` for the org-wide connection.
    pub scope_client_name: Option<String>,
    pub status: String,
    pub status_detail: Option<String>,
    /// The vendor's own words, verbatim.
    pub status_detail_vendor: Option<String>,
    #[schema(value_type = Option<String>, format = "date-time")]
    pub last_success_at: Option<chrono::DateTime<chrono::Utc>>,
    #[schema(value_type = Option<String>, format = "date-time")]
    pub last_failure_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Enabled destinations only.
    pub bindings_count: i64,
    /// The connection's non-secret settings. A field the manifest marks
    /// `writeOnly` is returned as `null` when it appears at all: the sealed
    /// value never crosses this boundary, and a client reads `secrets_set` to
    /// learn whether it is configured.
    pub settings: Value,
    /// The credential fields that hold a value, by name — never the values.
    /// Empty is "nothing configured yet", which is what the settings form
    /// renders as "Set".
    pub secrets_set: Vec<String>,
    /// `roles.id` for the roles allowed to see and act on this connection.
    /// Empty means every caller the module permission allows.
    pub audience_roles: Vec<String>,
}

/// GET /api/connectors/roster — every installed HTTP connector with its
/// connections.
///
/// Gated on the Plugins matrix rather than the record modules: this is the
/// admin settings list, and it carries connection health for every client.
/// The per-client visibility rule still applies to the CONNECTIONS: a
/// restricted client's connection is absent rather than listed with a client
/// the caller cannot open. The connector itself always appears, because an
/// install is not secret the way a client is — a card with no visible
/// connections still has to say the connector is installed.
///
/// The audience is applied here as well as in the resolver: a connection
/// outside the caller's roles is absent from the roster exactly as it is absent
/// from a chip row. Binding rows are not in this payload — the connection's own
/// `GET /connector-connections/{id}/bindings` carries them, so a roster of ten
/// connections does not ship every destination an org owns.
#[utoipa::path(operation_id = "connectors_roster", get,
    path = "/api/connectors/roster",
    tag = "connectors",
    security(("bearer" = [])),
    responses((status = 200, body = ConnectorRosterResponse)))]
pub async fn roster(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<ConnectorRosterResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    if !check(
        &authed.principal.permissions,
        Permission::Matrix {
            module: Module::Plugins,
            action: Action::View,
        },
    )
    .is_allowed()
    {
        return Err(ApiError::Forbidden(
            "the connector roster requires plugins:view".into(),
        ));
    }

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let rows = core_db::connector_connections::list_connector_roster(&mut tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;

    let viewer = viewer_of(&authed);
    Ok(Json(ConnectorRosterResponse {
        connectors: rows
            .into_iter()
            .map(|row| {
                // Parsed once per install, not once per connection: every
                // connection under this card shares the pinned schema.
                let schema = row
                    .settings_schema
                    .clone()
                    .and_then(|raw| SettingsSchema::new(raw).ok());
                ConnectorRosterView {
                    installation_id: row.installation_id,
                    connector: row.connector,
                    description: row.description,
                    product_kinds: row.product_kinds,
                    connections: row
                        .connections
                        .into_iter()
                        .filter(|connection| visible_to(connection, &viewer))
                        .map(|connection| {
                            view_of(connection, schema.as_ref(), &state.config.secret_key)
                        })
                        .collect(),
                }
            })
            .collect(),
    }))
}

/// Whether this connection may be shown to the caller.
///
/// Two independent doors, both closed by default: a connection never widens
/// client access, so a restricted client's connection is filtered by the same
/// list its own detail page is; and a connection with an audience is invisible
/// outside it. The audience filter here is the same predicate the resolver
/// applies in SQL, so a connection cannot be missing from a chip row and
/// present in the roster.
fn visible_to(row: &ConnectionRosterRow, viewer: &ConnectionViewer) -> bool {
    let client_allowed = viewer.is_admin
        || row.restricted_to.is_empty()
        || row.restricted_to.contains(&viewer.user_id);
    client_allowed && viewer.is_in_audience(&row.audience_roles)
}

fn view_of(
    row: ConnectionRosterRow,
    schema: Option<&SettingsSchema>,
    key: &crate::config::SecretKey,
) -> ConnectionView {
    ConnectionView {
        id: row.id,
        installation_id: row.installation_id,
        connector: row.connector,
        label: row.label,
        scope_client_id: row.scope_client_id,
        scope_client_name: row.scope_client_name,
        status: row.status.as_str().to_string(),
        status_detail: row.status_detail,
        status_detail_vendor: row.status_detail_vendor,
        last_success_at: row.last_success_at,
        last_failure_at: row.last_failure_at,
        bindings_count: row.bindings_count,
        settings: masked_settings(schema, &row.settings),
        secrets_set: secrets_set(key, row.secrets_encrypted.as_deref()),
        audience_roles: row.audience_roles,
    }
}

// ---------------------------------------------------------------------------
// The write path: connections, destinations, capability defaults.
//
// Posture, in the order it matters:
//
// * the gates are the neighbouring plugin routes' gates, not a new permission:
//   reads ask `plugins:view`, writes ask `special:manage_plugins` and an
//   owner/admin principal;
// * `secrets` is write-only on every request and absent from every response.
//   The response names which credential fields hold a value (`secrets_set`) so
//   the UI can say "configured" while holding nothing;
// * settings and secrets are judged against the PINNED manifest's
//   `settings_schema` by one validator, so a connection cannot store a value
//   its own connector could not have declared;
// * a connection scoped to a restricted client is refused to a caller who
//   cannot see that client — the same rule, and the same 404, the roster and
//   the record surface apply;
// * nothing logs a request body here. The handlers never bind `Json<…>` to a
//   tracing field and never echo `secrets` back.
// ---------------------------------------------------------------------------

/// `plugins:view` — the roster's own gate, reused so the two reads cannot
/// disagree about who may look.
fn require_connector_view(authed: &crate::auth::AuthedUser) -> ApiResult<()> {
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Plugins,
            action: Action::View,
        },
    )
}

/// Where a connection may be changed: the plugin admin gate the neighbouring
/// plugin routes use (`special:manage_plugins` + owner/admin).
fn require_connector_admin(authed: &crate::auth::AuthedUser) -> ApiResult<()> {
    require_permission(&authed.principal, Permission::ManagePlugins)?;
    if !authed.principal.is_owner_or_admin() {
        return Err(ApiError::Forbidden(
            "connector configuration is owner/admin".into(),
        ));
    }
    Ok(())
}

fn viewer_of(authed: &crate::auth::AuthedUser) -> ConnectionViewer {
    ConnectionViewer::new(
        authed.principal.user.id,
        authed.principal.roles.iter().map(|role| role.id).collect(),
        authed.principal.is_owner_or_admin(),
    )
}

/// The restricted-client rule, applied to a connection every time it is read or
/// written through this surface.
///
/// Same answer as the roster and the record surface: 404, because "this client
/// is not yours" and "this client does not exist" must not be distinguishable.
async fn require_client_visible(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    client_id: Option<Uuid>,
    authed: &crate::auth::AuthedUser,
) -> ApiResult<()> {
    let Some(client) = client_id else {
        return Ok(());
    };
    let scope = match resolve_record_scope(conn, org_id, RecordType::Client, client).await {
        Ok(Some(scope)) => scope,
        Ok(None) => return Err(ApiError::NotFound),
        Err(core_db::DbError::NotFound) => return Err(ApiError::NotFound),
        Err(e) => return Err(e.into()),
    };
    if !scope.restricted_to.is_empty()
        && !scope.restricted_to.contains(&authed.principal.user.id)
        && !authed.principal.is_owner_or_admin()
    {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

/// A connection row the caller may act on, or 404.
async fn connection_for_write(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    authed: &crate::auth::AuthedUser,
) -> ApiResult<Connection> {
    let row = get_connection(conn, org_id, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    require_client_visible(conn, org_id, row.scope_client_id, authed).await?;
    // Seeing a connection is part of "may see and act on it": a caller outside
    // the audience gets the same 404 the roster and the resolver give, not a
    // destination list the chip row would never have offered them.
    if !viewer_of(authed).is_in_audience(&row.audience_roles) {
        return Err(ApiError::NotFound);
    }
    Ok(row)
}

/// The audience, parsed into the canonical `roles.id::text` form the column
/// stores, with every id checked against this org's roles.
///
/// A uuid that names no role is a typo that would silently grant nobody access,
/// so it is refused here. Duplicates are collapsed: the column is a set.
async fn resolve_audience(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    raw: &[String],
) -> ApiResult<Vec<String>> {
    let mut ids: Vec<Uuid> = Vec::with_capacity(raw.len());
    for value in raw {
        let id: Uuid = value.trim().parse().map_err(|_| {
            ApiError::Validation(format!("audience_roles entry {value:?} is not a role id"))
        })?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    let unknown = unknown_roles(conn, org_id, &ids).await?;
    if !unknown.is_empty() {
        let names = unknown
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(ApiError::Validation(format!(
            "audience_roles names role(s) this org does not have: {names}"
        )));
    }
    Ok(ids.iter().map(|id| id.to_string()).collect())
}

/// The write-only credential object, or `None` when the field was absent.
///
/// Only scalar values are accepted: a credential is a token, and accepting a
/// nested document would let an adapter's whole config hide in a column nobody
/// validates.
fn read_secrets(raw: Option<Value>) -> ApiResult<Option<Map<String, Value>>> {
    let Some(value) = raw else {
        return Ok(None);
    };
    let Value::Object(map) = value else {
        return Err(ApiError::Validation(
            "secrets must be an object of credential fields".into(),
        ));
    };
    for (key, value) in &map {
        match value {
            Value::String(text) if text.is_empty() => {
                return Err(ApiError::Validation(format!(
                    "secret {key:?} is empty — send the value, or null to remove it"
                )))
            }
            Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null => {}
            _ => {
                return Err(ApiError::Validation(format!(
                    "secret {key:?} must be a string, number or boolean"
                )))
            }
        }
    }
    Ok(Some(map))
}

/// The settings object, or an empty one when the field was absent.
fn read_settings(raw: Option<Value>) -> ApiResult<Value> {
    match raw {
        None | Some(Value::Null) => Ok(json!({})),
        Some(value @ Value::Object(_)) => Ok(value),
        Some(_) => Err(ApiError::Validation(
            "settings must be a JSON object".into(),
        )),
    }
}

/// Judges settings and credential values against the pinned schema.
///
/// The two documents are one settings shape split by storage — queryable values
/// in `settings`, sealed values in `secrets_encrypted` — so they are validated
/// together, as the manifest declared them. A `writeOnly` key sent through
/// `settings` is refused rather than quietly sealed, because a client that puts
/// a credential there believes it was stored in the clear and is right to be
/// told otherwise.
fn validate_settings(
    schema: Option<&SettingsSchema>,
    settings: &Value,
    secrets: &Map<String, Value>,
) -> ApiResult<()> {
    let Some(schema) = schema else {
        let settings_empty = settings.as_object().is_none_or(|map| map.is_empty());
        if !settings_empty || !secrets.is_empty() {
            return Err(ApiError::Validation(
                "this connector's manifest declares no settings".into(),
            ));
        }
        return Ok(());
    };

    if let Some(map) = settings.as_object() {
        for key in map.keys() {
            if schema.is_write_only(key) {
                return Err(ApiError::Validation(format!(
                    "settings key {key:?} is a credential — send it in `secrets`, never in `settings`"
                )));
            }
        }
    }
    for key in secrets.keys() {
        if !schema.is_write_only(key) {
            return Err(ApiError::Validation(format!(
                "secret {key:?} is not a credential field of this connector — send it in `settings`"
            )));
        }
    }

    let mut document = settings.clone();
    if let Some(map) = document.as_object_mut() {
        for (key, value) in secrets {
            map.insert(key.clone(), value.clone());
        }
    }
    schema.validate_config(&document).map_err(|e| match e {
        core_domain::error::DomainError::Validation(message) => ApiError::Validation(message),
        other => ApiError::Validation(other.to_string()),
    })
}

/// The settings a client may see: anything the manifest marks `writeOnly` is
/// replaced with `null`.
///
/// Defensive rather than load-bearing — the write path refuses a credential in
/// `settings` — because a row written by an older path, or by hand, must not
/// start leaking the day a manifest marks a field it did not mark before.
fn masked_settings(schema: Option<&SettingsSchema>, settings: &Value) -> Value {
    let Some(schema) = schema else {
        return settings.clone();
    };
    let mut out = settings.clone();
    if let Some(map) = out.as_object_mut() {
        for key in schema.write_only_keys() {
            if map.contains_key(&key) {
                map.insert(key, Value::Null);
            }
        }
    }
    out
}

/// Which credential fields hold a value. Names only, never values.
///
/// A document that cannot be opened is reported as "nothing configured": the
/// server cannot prove a value is set, and claiming it is would be the worse
/// lie of the two. The route that replaces a secret reads the same failure as a
/// refusal, so the operator re-enters it.
fn secrets_set(key: &crate::config::SecretKey, sealed: Option<&[u8]>) -> Vec<String> {
    let Some(bytes) = sealed else {
        return Vec::new();
    };
    let Ok(plain) = crate::crypto::open(key, bytes) else {
        return Vec::new();
    };
    let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(&plain) else {
        return Vec::new();
    };
    let mut names: Vec<String> = map
        .into_iter()
        .filter(|(_, value)| !value.is_null())
        .map(|(name, _)| name)
        .collect();
    names.sort();
    names
}

/// The stored credential document as a map, for a merge. Refuses rather than
/// guesses when it cannot be opened.
fn open_secrets(
    key: &crate::config::SecretKey,
    sealed: Option<&[u8]>,
) -> ApiResult<Map<String, Value>> {
    let Some(bytes) = sealed else {
        return Ok(Map::new());
    };
    let plain = crate::crypto::open(key, bytes).map_err(|_| {
        ApiError::Conflict(
            "the stored credential cannot be opened with this server's key — \
             re-enter every credential field to replace it"
                .into(),
        )
    })?;
    serde_json::from_slice::<Map<String, Value>>(&plain)
        .map_err(|_| ApiError::Conflict("the stored credential is not a readable document".into()))
}

fn seal_secrets(
    key: &crate::config::SecretKey,
    map: Map<String, Value>,
) -> ApiResult<Option<Vec<u8>>> {
    if map.is_empty() {
        return Ok(None);
    }
    let bytes = serde_json::to_vec(&Value::Object(map))
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("sealing failed: {e}")))?;
    crate::crypto::seal(key, &bytes)
        .map(Some)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("sealing failed: {e}")))
}

fn schema_of(raw: Option<Value>) -> ApiResult<Option<SettingsSchema>> {
    match raw {
        None | Some(Value::Null) => Ok(None),
        Some(raw) => SettingsSchema::new(raw).map(Some).map_err(|e| {
            ApiError::Validation(format!(
                "the pinned manifest's settings_schema is invalid: {e}"
            ))
        }),
    }
}

/// A connection as a client sees it: settings masked, credentials named only.
fn connection_view(
    row: &Connection,
    connector: &str,
    scope_client_name: Option<String>,
    schema: Option<&SettingsSchema>,
    key: &crate::config::SecretKey,
    sealed: Option<&[u8]>,
) -> ConnectionView {
    ConnectionView {
        id: row.id,
        installation_id: row.installation_id,
        connector: connector.to_string(),
        label: row.label.clone(),
        scope_client_id: row.scope_client_id,
        scope_client_name,
        status: row.status.as_str().to_string(),
        status_detail: row.status_detail.clone(),
        status_detail_vendor: row.status_detail_vendor.clone(),
        last_success_at: row.last_success_at,
        last_failure_at: row.last_failure_at,
        bindings_count: 0,
        settings: masked_settings(schema, &row.settings),
        secrets_set: secrets_set(key, sealed),
        audience_roles: row.audience_roles.clone(),
    }
}

// --- POST /api/connector-installations/{installation_id}/connections --------

// Deliberately no `Debug`: this body carries credentials, and a derived Debug is
// how one reaches a tracing field the day somebody adds `debug!(?req)`.
#[derive(Deserialize, utoipa::ToSchema)]
pub struct CreateConnectionRequest {
    /// What the operator calls it ("NinjaOne — Acme"). Two connections of one
    /// install differ by label, so this is required.
    pub label: String,
    /// The client this connection serves. Absent is the org-wide connection.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub scope_client_id: Option<Uuid>,
    /// Non-secret settings, validated against the pinned `settings_schema`.
    pub settings: Option<Value>,
    /// Credential values, keyed by the schema's `writeOnly` fields. Write-only:
    /// never returned, never logged.
    pub secrets: Option<Value>,
    /// `roles.id` values allowed to see and act on this connection. Absent or
    /// empty means every caller the module permission allows.
    pub audience_roles: Option<Vec<String>>,
}

/// POST /api/connector-installations/{installation_id}/connections — configure
/// one use of an installed connector.
#[utoipa::path(operation_id = "connector_connections_create", post,
    path = "/api/connector-installations/{installation_id}/connections",
    tag = "connectors",
    security(("bearer" = [])),
    params(("installation_id" = Uuid, Path)),
    request_body = CreateConnectionRequest,
    responses(
        (status = 201, body = ConnectionView),
        (status = 404, description = "no such installation in this org, or the client is not visible"),
        (status = 422, description = "settings or credentials do not satisfy the pinned schema"),
    ))]
pub async fn connections_create(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(installation_id): Path<Uuid>,
    Json(req): Json<CreateConnectionRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<ConnectionView>)> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_connector_admin(&authed)?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;

    let label = req.label.trim().to_string();
    if label.is_empty() || label.chars().count() > 120 {
        return Err(ApiError::Validation(
            "label must be 1..=120 characters".into(),
        ));
    }
    let settings = read_settings(req.settings)?;
    let secrets_in = read_secrets(req.secrets)?.unwrap_or_default();
    let audience_in = req.audience_roles.clone().unwrap_or_default();

    let created = crate::org_tx::with_org_tx(&state, org_id, actor, |mut tx| {
        let state = &state;
        let label = label.clone();
        let settings = settings.clone();
        let secrets_in = secrets_in.clone();
        let audience_in = audience_in.clone();
        async move {
            let pin = get_installation_pin(&mut tx.tx, org_id, installation_id)
                .await
                .map_err(|e| match e {
                    core_db::DbError::NotFound => ApiError::NotFound,
                    other => other.into(),
                })?;
            // The roster only draws connectors; a connection of a WASM plugin is
            // a use nothing in the UI can show.
            if pin.artifact_kind.as_deref() != Some("http_connector") {
                return Err(ApiError::Validation(
                    "that installation is not an http_connector".into(),
                ));
            }
            let schema = schema_of(pin.settings_schema.clone())?;
            validate_settings(schema.as_ref(), &settings, &secrets_in)?;
            require_client_visible(&mut tx.tx, org_id, req.scope_client_id, &authed).await?;
            let audience = resolve_audience(&mut tx.tx, org_id, &audience_in).await?;

            let sealed = seal_secrets(&state.config.secret_key, secrets_in.clone())?;
            let row = create_connection(
                &mut tx.tx,
                &NewConnection {
                    id: Uuid::now_v7(),
                    org_id,
                    installation_id,
                    scope_client_id: req.scope_client_id,
                    label: &label,
                    settings: &settings,
                    secrets_encrypted: sealed.as_deref(),
                    audience_roles: &audience,
                },
            )
            .await?;
            let client_name = match row.scope_client_id {
                Some(client) => client_display_name(&mut tx.tx, org_id, client).await?,
                None => None,
            };
            let mut view = connection_view(
                &row,
                &pin.connector,
                client_name,
                schema.as_ref(),
                &state.config.secret_key,
                sealed.as_deref(),
            );
            view.bindings_count = count_enabled_bindings(&mut tx.tx, org_id, row.id).await?;
            Ok((tx, view))
        }
    })
    .await?;

    Ok((axum::http::StatusCode::CREATED, Json(created)))
}

// --- PATCH /api/connector-connections/{id} ----------------------------------

// No `Debug`, for the same reason as `CreateConnectionRequest`.
#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateConnectionRequest {
    pub label: Option<String>,
    /// `null` moves the connection back to org-wide; absent leaves it alone.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub scope_client_id: Option<Option<Uuid>>,
    /// Merged into the stored settings: a key present in the body replaces the
    /// stored one, `null` removes it, an absent key is untouched.
    pub settings: Option<Value>,
    /// Merged into the sealed credential document, with the same three rules.
    /// An absent `secrets` leaves the stored credential exactly as it is, which
    /// is what "the operator did not retype it" looks like.
    pub secrets: Option<Value>,
    pub audience_roles: Option<Vec<String>>,
}

fn double_option<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Option::<T>::deserialize(de).map(Some)
}

/// PATCH /api/connector-connections/{id} — change what was configured.
#[utoipa::path(operation_id = "connector_connections_update", patch,
    path = "/api/connector-connections/{id}",
    tag = "connectors",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = UpdateConnectionRequest,
    responses(
        (status = 200, body = ConnectionView),
        (status = 404, description = "no such connection, or its client is not visible"),
        (status = 409, description = "the stored credential cannot be opened with this server's key"),
        (status = 422, description = "settings or credentials do not satisfy the pinned schema"),
    ))]
pub async fn connections_update(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateConnectionRequest>,
) -> ApiResult<Json<ConnectionView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_connector_admin(&authed)?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;

    let label = match &req.label {
        Some(label) => {
            let trimmed = label.trim().to_string();
            if trimmed.is_empty() || trimmed.chars().count() > 120 {
                return Err(ApiError::Validation(
                    "label must be 1..=120 characters".into(),
                ));
            }
            Some(trimmed)
        }
        None => None,
    };
    let patch_settings = match &req.settings {
        Some(value) => Some(read_settings(Some(value.clone()))?),
        None => None,
    };
    let patch_secrets = read_secrets(req.secrets.clone())?;

    let view = crate::org_tx::with_org_tx(&state, org_id, actor, |mut tx| {
        let state = &state;
        let req = &req;
        let label = label.clone();
        let patch_settings = patch_settings.clone();
        let patch_secrets = patch_secrets.clone();
        async move {
            let existing = connection_for_write(&mut tx.tx, org_id, id, &authed).await?;
            let pin = get_installation_pin(&mut tx.tx, org_id, existing.installation_id)
                .await
                .map_err(|e| match e {
                    core_db::DbError::NotFound => ApiError::NotFound,
                    other => other.into(),
                })?;
            let schema = schema_of(pin.settings_schema.clone())?;

            // Settings merge key by key; `null` removes.
            let mut settings = existing.settings.clone();
            if let Some(patch) = &patch_settings {
                let mut merged = settings.as_object().cloned().unwrap_or_default();
                for (key, value) in patch.as_object().cloned().unwrap_or_default() {
                    if value.is_null() {
                        merged.remove(&key);
                    } else {
                        merged.insert(key, value);
                    }
                }
                settings = Value::Object(merged);
            }

            // Secrets: the stored document is read on every PATCH because the
            // schema's `required` list is judged against the whole credential
            // set — a label-only edit must not read as "the credential went
            // missing". It is only REWRITTEN when the body carries a
            // replacement, so an untouched credential keeps its sealed bytes.
            let stored = get_connection_secrets(&mut tx.tx, org_id, id).await?;
            let mut secrets = open_secrets(&state.config.secret_key, stored.as_deref())?;
            let secrets_update = match &patch_secrets {
                None => None,
                Some(patch) => {
                    for (key, value) in patch {
                        if value.is_null() {
                            secrets.remove(key);
                        } else {
                            secrets.insert(key.clone(), value.clone());
                        }
                    }
                    Some(seal_secrets(&state.config.secret_key, secrets.clone())?)
                }
            };

            validate_settings(schema.as_ref(), &settings, &secrets)?;

            if let Some(scope) = req.scope_client_id.flatten() {
                require_client_visible(&mut tx.tx, org_id, Some(scope), &authed).await?;
            }
            let audience = match &req.audience_roles {
                Some(raw) => Some(resolve_audience(&mut tx.tx, org_id, raw).await?),
                None => None,
            };

            let row = update_connection(
                &mut tx.tx,
                org_id,
                id,
                &ConnectionUpdate {
                    label: label.as_deref(),
                    scope_client_id: req.scope_client_id,
                    settings: patch_settings.as_ref().map(|_| &settings),
                    secrets_encrypted: secrets_update.as_ref().map(|bytes| bytes.as_deref()),
                    audience_roles: audience.as_deref(),
                },
            )
            .await?
            .ok_or(ApiError::NotFound)?;

            let client_name = match row.scope_client_id {
                Some(client) => client_display_name(&mut tx.tx, org_id, client).await?,
                None => None,
            };
            let sealed_for_view = get_connection_secrets(&mut tx.tx, org_id, id).await?;
            let mut view = connection_view(
                &row,
                &pin.connector,
                client_name,
                schema.as_ref(),
                &state.config.secret_key,
                sealed_for_view.as_deref(),
            );
            view.bindings_count = count_enabled_bindings(&mut tx.tx, org_id, id).await?;
            Ok((tx, view))
        }
    })
    .await?;

    Ok(Json(view))
}

// --- DELETE /api/connector-connections/{id} ---------------------------------

/// DELETE /api/connector-connections/{id} — remove a connection only while
/// nothing points at it.
///
/// `external_refs` and `user_external_identities` cascade from this row, so a
/// hard delete would silently erase the record of which vendor object a
/// configuration is and which person a vendor identity belonged to. The 409
/// names what still points here; revoking the installation is the path that
/// takes those with it, and that is deliberately a different act.
#[utoipa::path(operation_id = "connector_connections_delete", delete,
    path = "/api/connector-connections/{id}",
    tag = "connectors",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 204),
        (status = 404, description = "no such connection, or its client is not visible"),
        (status = 409, description = "destinations, external references or vendor identities still point at it"),
    ))]
pub async fn connections_delete(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_connector_admin(&authed)?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;

    crate::org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        connection_for_write(&mut tx.tx, org_id, id, &authed).await?;
        let usage = connection_usage(&mut tx.tx, org_id, id).await?;
        if usage.total() > 0 {
            return Err(ApiError::Conflict(format!(
                "connection is still referenced: {} destination(s), {} external reference(s), \
                 {} vendor identity(ies) — revoke the installation to remove those",
                usage.bindings, usage.external_refs, usage.identities
            )));
        }
        if !delete_connection(&mut tx.tx, org_id, id).await? {
            return Err(ApiError::NotFound);
        }
        Ok((tx, ()))
    })
    .await?;

    Ok(axum::http::StatusCode::NO_CONTENT)
}

// --- bindings ---------------------------------------------------------------

/// The closed destination vocabulary in `connector_bindings.binding_kind`.
/// Repeated here so a bad value is a typed 422 naming the set, instead of a
/// database CHECK surfacing as a 500.
const BINDING_KINDS: &[&str] = &[
    "slack_channel",
    "teams_conversation",
    "rewst_workflow",
    "voip_route",
];

fn parse_binding_kind(raw: &str) -> ApiResult<&'static str> {
    let trimmed = raw.trim();
    BINDING_KINDS
        .iter()
        .copied()
        .find(|kind| *kind == trimmed)
        .ok_or_else(|| {
            ApiError::Validation(format!(
                "unknown binding_kind {raw:?} (declared kinds: {})",
                BINDING_KINDS.join(", ")
            ))
        })
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateBindingRequest {
    /// `slack_channel` | `teams_conversation` | `rewst_workflow` | `voip_route`.
    pub binding_kind: String,
    /// What the operator called it ("#helpdesk-alerts").
    pub label: String,
    /// The vendor's id for the destination. Slack: the channel id (C…), never
    /// a name — a name resolves ambiguously and a private channel answers with
    /// a generic error.
    pub external_id: String,
    /// The workspace or tenant the destination sits in, when the vendor needs
    /// it (Slack: team id, Teams: tenant id).
    pub external_parent: Option<String>,
    /// Vendor ids the deep link needs and that differ per kind. Ids only.
    pub vendor_ids: Option<Value>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct BindingView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub connection_id: Uuid,
    pub binding_kind: String,
    pub label: String,
    pub external_id: String,
    pub external_parent: Option<String>,
    pub vendor_ids: Value,
    pub enabled: bool,
}

fn binding_view(binding: Binding) -> BindingView {
    BindingView {
        id: binding.id,
        connection_id: binding.connection_id,
        binding_kind: binding.binding_kind,
        label: binding.label,
        external_id: binding.external_id,
        external_parent: binding.external_parent,
        vendor_ids: binding.vendor_ids,
        enabled: binding.enabled,
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct BindingListResponse {
    pub bindings: Vec<BindingView>,
}

/// GET /api/connector-connections/{id}/bindings — every destination the share
/// dialog may offer, disabled ones included: the management surface has to show
/// what exists in order to re-enable it.
#[utoipa::path(operation_id = "connector_bindings_list", get,
    path = "/api/connector-connections/{id}/bindings",
    tag = "connectors",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = BindingListResponse),
        (status = 404, description = "no such connection, or its client is not visible"),
    ))]
pub async fn bindings_list(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<BindingListResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_connector_view(&authed)?;
    let org_id = authed.principal.user.org_id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    connection_for_write(&mut tx, org_id, id, &authed).await?;
    let bindings = list_bindings(&mut tx, org_id, id, true).await?;
    tx.commit().await.map_err(ApiError::from)?;

    Ok(Json(BindingListResponse {
        bindings: bindings.into_iter().map(binding_view).collect(),
    }))
}

/// POST /api/connector-connections/{id}/bindings — add a named destination.
#[utoipa::path(operation_id = "connector_bindings_create", post,
    path = "/api/connector-connections/{id}/bindings",
    tag = "connectors",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = CreateBindingRequest,
    responses(
        (status = 201, body = BindingView),
        (status = 404, description = "no such connection, or its client is not visible"),
        (status = 409, description = "that destination is already bound to this connection"),
        (status = 422, description = "unknown binding kind or malformed destination"),
    ))]
pub async fn bindings_create(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateBindingRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<BindingView>)> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_connector_admin(&authed)?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;

    let kind = parse_binding_kind(&req.binding_kind)?;
    let label = req.label.trim().to_string();
    if label.is_empty() || label.chars().count() > 200 {
        return Err(ApiError::Validation(
            "label must be 1..=200 characters".into(),
        ));
    }
    let external_id = req.external_id.trim().to_string();
    if external_id.is_empty() {
        return Err(ApiError::Validation(
            "external_id is the vendor's own id for the destination and cannot be empty".into(),
        ));
    }
    let vendor_ids = match req.vendor_ids.clone() {
        None | Some(Value::Null) => json!({}),
        Some(value @ Value::Object(_)) => value,
        Some(_) => {
            return Err(ApiError::Validation(
                "vendor_ids must be a JSON object of vendor ids".into(),
            ))
        }
    };

    let created = crate::org_tx::with_org_tx(&state, org_id, actor, |mut tx| {
        let label = label.clone();
        let external_id = external_id.clone();
        let vendor_ids = vendor_ids.clone();
        let req = &req;
        async move {
            connection_for_write(&mut tx.tx, org_id, id, &authed).await?;
            let row = create_binding(
                &mut tx.tx,
                &NewBinding {
                    id: Uuid::now_v7(),
                    org_id,
                    connection_id: id,
                    binding_kind: kind,
                    label: &label,
                    external_id: &external_id,
                    external_parent: req.external_parent.as_deref(),
                    vendor_ids: &vendor_ids,
                    enabled: req.enabled.unwrap_or(true),
                },
            )
            .await
            // The unique index on (org, connection, kind, external_id) is the
            // duplicate-destination refusal; say that instead of "database".
            .map_err(|e| match e {
                core_db::DbError::Sql(sqlx::Error::Database(db))
                    if db.constraint() == Some("connector_bindings_destination_unique") =>
                {
                    ApiError::Conflict("that destination is already bound".into())
                }
                other => other.into(),
            })?;
            Ok((tx, binding_view(row)))
        }
    })
    .await?;

    Ok((axum::http::StatusCode::CREATED, Json(created)))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateBindingRequest {
    pub label: Option<String>,
    pub enabled: Option<bool>,
    /// `null` clears the workspace/tenant id; absent leaves it alone.
    #[serde(default, deserialize_with = "double_option")]
    pub external_parent: Option<Option<String>>,
    pub vendor_ids: Option<Value>,
}

/// PATCH /api/connector-bindings/{id} — rename, disable or re-point metadata.
///
/// `external_id` is not here on purpose: it is the destination's identity at
/// the vendor, and changing it is a new destination rather than an edit.
#[utoipa::path(operation_id = "connector_bindings_update", patch,
    path = "/api/connector-bindings/{id}",
    tag = "connectors",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = UpdateBindingRequest,
    responses(
        (status = 200, body = BindingView),
        (status = 404, description = "no such binding, or its connection's client is not visible"),
        (status = 422, description = "malformed label or vendor_ids"),
    ))]
pub async fn bindings_update(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateBindingRequest>,
) -> ApiResult<Json<BindingView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_connector_admin(&authed)?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;

    let label = match &req.label {
        Some(label) => {
            let trimmed = label.trim().to_string();
            if trimmed.is_empty() || trimmed.chars().count() > 200 {
                return Err(ApiError::Validation(
                    "label must be 1..=200 characters".into(),
                ));
            }
            Some(trimmed)
        }
        None => None,
    };
    let vendor_ids = match &req.vendor_ids {
        None => None,
        Some(Value::Object(_)) => req.vendor_ids.clone(),
        Some(_) => {
            return Err(ApiError::Validation(
                "vendor_ids must be a JSON object of vendor ids".into(),
            ))
        }
    };

    let updated = crate::org_tx::with_org_tx(&state, org_id, actor, |mut tx| {
        let label = label.clone();
        let vendor_ids = vendor_ids.clone();
        let req = &req;
        async move {
            let binding = binding_in_org(&mut tx.tx, org_id, id).await?;
            connection_for_write(&mut tx.tx, org_id, binding.connection_id, &authed).await?;
            let row = update_binding(
                &mut tx.tx,
                org_id,
                id,
                &BindingUpdate {
                    label: label.as_deref(),
                    enabled: req.enabled,
                    external_parent: req.external_parent.as_ref().map(|value| value.as_deref()),
                    vendor_ids: vendor_ids.as_ref(),
                },
            )
            .await?
            .ok_or(ApiError::NotFound)?;
            Ok((tx, binding_view(row)))
        }
    })
    .await?;

    Ok(Json(updated))
}

/// DELETE /api/connector-bindings/{id} — remove a destination.
///
/// A share that already went out keeps its audit row: the binding id is
/// recorded there, and the row is what still proves where a message landed.
#[utoipa::path(operation_id = "connector_bindings_delete", delete,
    path = "/api/connector-bindings/{id}",
    tag = "connectors",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 204),
        (status = 404, description = "no such binding, or its connection's client is not visible"),
    ))]
pub async fn bindings_delete(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_connector_admin(&authed)?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;

    crate::org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let binding = binding_in_org(&mut tx.tx, org_id, id).await?;
        connection_for_write(&mut tx.tx, org_id, binding.connection_id, &authed).await?;
        if !delete_binding(&mut tx.tx, org_id, id).await? {
            return Err(ApiError::NotFound);
        }
        Ok((tx, ()))
    })
    .await?;

    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// One binding by id, refusing with the same 404 an absent row gets.
async fn binding_in_org(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> ApiResult<Binding> {
    let row = sqlx::query(
        "SELECT id, connection_id, binding_kind, label, external_id, external_parent,
                vendor_ids, enabled
           FROM connector_bindings WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(row) = row else {
        return Err(ApiError::NotFound);
    };
    use sqlx::Row;
    Ok(Binding {
        id: row.get("id"),
        connection_id: row.get("connection_id"),
        binding_kind: row.get("binding_kind"),
        label: row.get("label"),
        external_id: row.get("external_id"),
        external_parent: row.get("external_parent"),
        vendor_ids: row.get("vendor_ids"),
        enabled: row.get("enabled"),
    })
}

// --- capability defaults ----------------------------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CapabilityDefaultRequest {
    /// The connection that becomes this capability's primary.
    #[schema(value_type = uuid::Uuid)]
    pub connection_id: Uuid,
    /// Absent is the org default; present overrides it for one client.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub scope_client_id: Option<Uuid>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CapabilityDefaultView {
    pub capability: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub scope_client_id: Option<Uuid>,
    #[schema(value_type = uuid::Uuid)]
    pub connection_id: Uuid,
}

/// PUT /api/capability-defaults/{capability} — which provider a chip runs first.
///
/// A default never hides the other providers; it decides which press target the
/// chip body is. Refused when the connection does not declare the capability,
/// because a default that can never match marks nothing primary and reads to an
/// operator as a setting that did not save.
#[utoipa::path(operation_id = "capability_defaults_put", put,
    path = "/api/capability-defaults/{capability}",
    tag = "connectors",
    security(("bearer" = [])),
    params(("capability" = String, Path)),
    request_body = CapabilityDefaultRequest,
    responses(
        (status = 200, body = CapabilityDefaultView),
        (status = 404, description = "no such connection, or its client is not visible"),
        (status = 422, description = "unknown capability, or one the connection does not declare"),
    ))]
pub async fn capability_default_put(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(capability): Path<String>,
    Json(req): Json<CapabilityDefaultRequest>,
) -> ApiResult<Json<CapabilityDefaultView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_connector_admin(&authed)?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;

    let capability = ProductKind::parse(&capability).map_err(|_| {
        let known = ProductKind::KNOWN.join(", ");
        ApiError::Validation(format!(
            "unknown capability {capability:?} (known kinds: {known}; any well-formed kind a manifest declares is accepted)"
        ))
    })?;
    let capability = capability.as_str().to_string();

    let view = crate::org_tx::with_org_tx(&state, org_id, actor, |mut tx| {
        let capability = capability.clone();
        let req = &req;
        async move {
            let connection =
                connection_for_write(&mut tx.tx, org_id, req.connection_id, &authed).await?;
            require_client_visible(&mut tx.tx, org_id, req.scope_client_id, &authed).await?;

            let pin = get_installation_pin(&mut tx.tx, org_id, connection.installation_id)
                .await
                .map_err(|e| match e {
                    core_db::DbError::NotFound => ApiError::NotFound,
                    other => other.into(),
                })?;
            if !pin.product_kinds.iter().any(|kind| kind == &capability) {
                return Err(ApiError::Validation(format!(
                    "{} does not declare the {capability} capability",
                    pin.connector
                )));
            }
            // A client default naming a connection bound to another client can
            // never match a record of that client, so it is refused here rather
            // than stored as a setting that does nothing.
            if let (Some(default_client), Some(connection_client)) =
                (req.scope_client_id, connection.scope_client_id)
            {
                if default_client != connection_client {
                    return Err(ApiError::Validation(
                        "a client-scoped default must name a connection scoped to that same client"
                            .into(),
                    ));
                }
            }

            upsert_capability_default(
                &mut tx.tx,
                org_id,
                Uuid::now_v7(),
                &capability,
                req.scope_client_id,
                req.connection_id,
            )
            .await?;
            Ok((
                tx,
                CapabilityDefaultView {
                    capability,
                    scope_client_id: req.scope_client_id,
                    connection_id: req.connection_id,
                },
            ))
        }
    })
    .await?;

    Ok(Json(view))
}

#[derive(Debug, Deserialize, utoipa::IntoParams, utoipa::ToSchema)]
pub struct CapabilityScopeQuery {
    /// The client the default is scoped to. Absent is the org default.
    pub scope_client_id: Option<Uuid>,
}

/// DELETE /api/capability-defaults/{capability}?scope_client_id= — clear the
/// primary, so the chip falls back to "several providers, no default".
#[utoipa::path(operation_id = "capability_defaults_delete", delete,
    path = "/api/capability-defaults/{capability}",
    tag = "connectors",
    security(("bearer" = [])),
    params(("capability" = String, Path), CapabilityScopeQuery),
    responses(
        (status = 204),
        (status = 404, description = "no default is set for that capability and scope"),
    ))]
pub async fn capability_default_delete(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(capability): Path<String>,
    Query(query): Query<CapabilityScopeQuery>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_connector_admin(&authed)?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;

    let capability = ProductKind::parse(&capability)
        .map_err(|_| ApiError::Validation(format!("unknown capability {capability:?}")))?
        .as_str()
        .to_string();

    crate::org_tx::with_org_tx(&state, org_id, actor, |mut tx| {
        let capability = capability.clone();
        async move {
            require_client_visible(&mut tx.tx, org_id, query.scope_client_id, &authed).await?;
            if !delete_capability_default(&mut tx.tx, org_id, &capability, query.scope_client_id)
                .await?
            {
                return Err(ApiError::NotFound);
            }
            Ok((tx, ()))
        }
    })
    .await?;

    Ok(axum::http::StatusCode::NO_CONTENT)
}
