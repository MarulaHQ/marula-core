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

use crate::auth::middleware::authed;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use core_db::connector_connections::{
    resolve_external_actions, resolve_record_scope, ConnectionRosterRow,
};
use core_domain::connectors::RecordType;
use core_domain::policy::{check, Action, Module, Permission};
use serde::Serialize;
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

    let capabilities = resolve_external_actions(&mut tx, org_id, kind, record_id).await?;
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
/// Still absent, deliberately: binding rows and the audience. Both are a later
/// route, not a field this one omits by accident.
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

    let caller = authed.principal.user.id;
    let admin = authed.principal.is_owner_or_admin();
    Ok(Json(ConnectorRosterResponse {
        connectors: rows
            .into_iter()
            .map(|row| ConnectorRosterView {
                installation_id: row.installation_id,
                connector: row.connector,
                description: row.description,
                product_kinds: row.product_kinds,
                connections: row
                    .connections
                    .into_iter()
                    .filter(|connection| visible_to(connection, caller, admin))
                    .map(view_of)
                    .collect(),
            })
            .collect(),
    }))
}

/// Whether this connection may be shown to the caller. A connection never
/// widens client access, so a restricted client's connection is filtered by
/// the same list its own detail page is.
fn visible_to(row: &ConnectionRosterRow, caller: Uuid, admin: bool) -> bool {
    admin || row.restricted_to.is_empty() || row.restricted_to.contains(&caller)
}

fn view_of(row: ConnectionRosterRow) -> ConnectionView {
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
    }
}
