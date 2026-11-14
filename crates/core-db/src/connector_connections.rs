//! Connector connections, bindings, external references and record links
//! (ADR 0032, ADR 0033; migration 0185).
//!
//! Purpose: the storage half of the connection model. A connection is one
//! configured use of an installed connector, scoped to the org or to one
//! client, carrying the settings the resolver needs to build a link and the
//! sealed credential it needs to act. A binding is a named destination that
//! connection may post to. An external reference says this Marula record is
//! that vendor object, and a record link says this record is related to that
//! one.
//!
//! Why this is not `config_identity`: that module owns the CI identity rules
//! (CI1–CI7) and the merge-candidate queue for one record type. This one is
//! record-type-agnostic, carries the connection a reference resolves through,
//! and is the seam the whole staff UI reads. `config_source_links` is
//! superseded by `external_refs` per ADR 0033 and keeps working until the
//! backfill lands.
//!
//! Non-purpose: no vendor HTTP, no link templating, no permission decisions.
//! The resolver that turns these rows into chips lives above this layer, and
//! every function here assumes the caller already holds the org binding and
//! the client-access check. Scoping is deliberately not re-implemented in SQL:
//! RLS is org-level in this product, and a second client-scoping policy would
//! be a second place to get it wrong.
//!
//! Query style: runtime `sqlx::query` with explicit row mapping, matching
//! `config_identity` rather than the `query!` macros, because these statements
//! carry `ON CONFLICT` inference against partial indexes and a compiled cache
//! entry for them would need re-preparing whenever a predicate moved.

use chrono::{DateTime, Utc};
use core_domain::connectors::{ConnectionStatus, LinkEndKind, RecordType};
use core_domain::entities::ConnectorSurfaces;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::DbResult;

const CONNECTION_COLUMNS: &str = "id, org_id, installation_id, scope_client_id, label, settings,
     status, status_detail, status_detail_vendor, last_success_at, last_failure_at,
     created_at, updated_at";

/// One configured use of an installed connector.
#[derive(Debug, Clone)]
pub struct Connection {
    pub id: Uuid,
    pub org_id: Uuid,
    pub installation_id: Uuid,
    /// `None` is the org-wide connection. `Some(client)` is a per-client
    /// instance — the G11 case, where two clients each have their own
    /// controller and their own credential.
    pub scope_client_id: Option<Uuid>,
    pub label: String,
    /// Non-secret settings that satisfy the manifest's schema. Readable
    /// without unsealing anything, so the resolver can build a link.
    pub settings: serde_json::Value,
    pub status: ConnectionStatus,
    /// A typed reason code the UI branches on (`otp_required`, `expired`, …).
    pub status_detail: Option<String>,
    /// The vendor's own words. Never parsed into prose: a paraphrase of an
    /// auth error sends an operator to the wrong fix.
    pub status_detail_vendor: Option<String>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_failure_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn parse_status(raw: &str) -> DbResult<ConnectionStatus> {
    ConnectionStatus::parse(raw).ok_or_else(|| {
        crate::DbError::Corrupt(format!(
            "connector_connections.status `{raw}` is not in the vocabulary"
        ))
    })
}

fn row_to_connection(r: sqlx::postgres::PgRow) -> DbResult<Connection> {
    Ok(Connection {
        id: r.try_get("id")?,
        org_id: r.try_get("org_id")?,
        installation_id: r.try_get("installation_id")?,
        scope_client_id: r.try_get("scope_client_id")?,
        label: r.try_get("label")?,
        settings: r.try_get("settings")?,
        status: parse_status(&r.try_get::<String, _>("status")?)?,
        status_detail: r.try_get("status_detail")?,
        status_detail_vendor: r.try_get("status_detail_vendor")?,
        last_success_at: r.try_get("last_success_at")?,
        last_failure_at: r.try_get("last_failure_at")?,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
    })
}

/// Every connection the org holds, org-wide and per client, labelled order.
pub async fn list_connections(conn: &mut PgConnection, org_id: Uuid) -> DbResult<Vec<Connection>> {
    let rows = sqlx::query(&format!(
        "SELECT {CONNECTION_COLUMNS} FROM connector_connections
         WHERE org_id = $1 ORDER BY label, created_at"
    ))
    .bind(org_id)
    .fetch_all(&mut *conn)
    .await?;
    rows.into_iter().map(row_to_connection).collect()
}

/// One connection inside a connector's roster entry.
#[derive(Debug, Clone)]
pub struct ConnectionRosterRow {
    pub id: Uuid,
    pub installation_id: Uuid,
    /// The installed manifest's `name` (the installer's fallback: the catalog
    /// slug, so a row whose version disappeared still names itself). Repeated
    /// here because per-connection rendering uses it without walking back to
    /// the parent.
    pub connector: String,
    pub label: String,
    pub scope_client_id: Option<Uuid>,
    /// The scoped client's name, so the roster can say which clients a
    /// connector serves by name instead of "2 items". `None` for the org-wide
    /// connection.
    pub scope_client_name: Option<String>,
    pub status: ConnectionStatus,
    pub status_detail: Option<String>,
    pub status_detail_vendor: Option<String>,
    pub last_success_at: Option<DateTime<Utc>>,
    /// The last proven failure. A status change without a timestamp is a
    /// claim; the roster's health line shows this beside the vendor's words.
    pub last_failure_at: Option<DateTime<Utc>>,
    /// Enabled destinations only: a disabled binding cannot receive a share,
    /// so counting it would promise a picker entry the dialog will not show.
    pub bindings_count: i64,
    /// The scoped client's `restricted_to` list. The roster route applies it:
    /// a connection never widens what a user may see, so a restricted client's
    /// connection must not surface in the list either.
    pub restricted_to: Vec<Uuid>,
}

/// One installed connector, with its configured uses nested.
///
/// The unit is the INSTALLATION, not the connection, because the first-run
/// state is an install with zero connections: a roster keyed on connections
/// hides exactly the card whose action is `Connect`. An empty `connections`
/// list is therefore a real entry, not a missing one.
#[derive(Debug, Clone)]
pub struct ConnectorRosterRow {
    pub installation_id: Uuid,
    /// The pinned manifest's name, falling back to the catalog slug.
    pub connector: String,
    /// The pinned manifest's own one-line purpose. Empty when it declares
    /// none — never re-authored here.
    pub description: String,
    /// What kind of vendor this is. Empty means the connector declared none
    /// and therefore contributes no chips — a complete connector.
    pub product_kinds: Vec<String>,
    pub connections: Vec<ConnectionRosterRow>,
}

/// The roster: every installed HTTP connector, with its connections nested.
///
/// Why installation-first: a connector that is installed but not yet
/// configured has no connection row, and the card that opens its settings is
/// exactly the one that must render. Only `http_connector` installs are
/// listed — a WASM plugin is not a connector and has no roster entry.
///
/// Status stays per connection. No rollup is computed here: the UI derives
/// "worst wins" and "1 of 4" from the rows, and a second answer in SQL would
/// be a second place to disagree with it.
///
/// Not the whole roster contract: the destinations themselves (the binding
/// rows) and the connection's audience are still absent. Both are a later
/// route; this read answers identity, health and the client each connection
/// serves, and says nothing it cannot evidence.
pub async fn list_connector_roster(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<ConnectorRosterRow>> {
    let installs = sqlx::query(
        "SELECT i.id AS installation_id,
                COALESCE(v.manifest->>'name', cat.slug, '') AS connector,
                COALESCE(v.manifest->>'description', '') AS description,
                COALESCE(v.manifest->'product_kinds', '[]'::jsonb) AS product_kinds
           FROM plugin_installs i
           JOIN plugin_versions v
             ON v.catalog_id = i.catalog_id AND v.version = i.version
           LEFT JOIN plugin_catalog cat ON cat.id = i.catalog_id
          WHERE i.org_id = $1
            AND i.status <> 'revoked'
            AND v.manifest->>'artifact_kind' = 'http_connector'
          ORDER BY connector, i.id",
    )
    .bind(org_id)
    .fetch_all(&mut *conn)
    .await?;

    let mut roster: Vec<ConnectorRosterRow> = Vec::with_capacity(installs.len());
    let mut index: std::collections::HashMap<Uuid, usize> =
        std::collections::HashMap::with_capacity(installs.len());
    for row in installs {
        let installation_id: Uuid = row.try_get("installation_id")?;
        let kinds: serde_json::Value = row.try_get("product_kinds")?;
        index.insert(installation_id, roster.len());
        roster.push(ConnectorRosterRow {
            installation_id,
            connector: row.try_get("connector")?,
            description: row.try_get("description")?,
            product_kinds: kind_strings(&kinds),
            connections: Vec::new(),
        });
    }

    if roster.is_empty() {
        return Ok(roster);
    }

    let connections = sqlx::query(
        "SELECT c.id, c.installation_id, c.scope_client_id, c.label, c.status,
                c.status_detail, c.status_detail_vendor,
                c.last_success_at, c.last_failure_at,
                COALESCE(cl.restricted_to, '[]'::jsonb) AS restricted_to,
                cl.name AS scope_client_name,
                (SELECT count(*) FROM connector_bindings b
                  WHERE b.org_id = c.org_id AND b.connection_id = c.id AND b.enabled)
                  AS bindings_count
           FROM connector_connections c
           LEFT JOIN clients cl
             ON cl.id = c.scope_client_id AND cl.org_id = c.org_id
          WHERE c.org_id = $1
          ORDER BY c.scope_client_id NULLS FIRST, c.label, c.created_at",
    )
    .bind(org_id)
    .fetch_all(&mut *conn)
    .await?;

    for row in connections {
        let installation_id: Uuid = row.try_get("installation_id")?;
        let Some(&slot) = index.get(&installation_id) else {
            // A connection of an install this roster does not list (a WASM
            // plugin, a revoked install). Dropping it is the filter working,
            // not a lost row: the resolver still reads it directly.
            continue;
        };
        let connector = roster[slot].connector.clone();
        roster[slot].connections.push(ConnectionRosterRow {
            id: row.try_get("id")?,
            installation_id,
            connector,
            label: row.try_get("label")?,
            scope_client_id: row.try_get("scope_client_id")?,
            scope_client_name: row.try_get("scope_client_name")?,
            status: parse_status(&row.try_get::<String, _>("status")?)?,
            status_detail: row.try_get("status_detail")?,
            status_detail_vendor: row.try_get("status_detail_vendor")?,
            last_success_at: row.try_get("last_success_at")?,
            last_failure_at: row.try_get("last_failure_at")?,
            bindings_count: row.try_get("bindings_count")?,
            restricted_to: restriction_list(&row.try_get::<serde_json::Value, _>("restricted_to")?),
        });
    }

    Ok(roster)
}

/// The declared product kinds as plain strings.
///
/// A declared kind is validated at manifest adoption, so the only way a
/// non-string lands here is drift between the gate and the resolver; dropping
/// it renders one fewer chip instead of failing the whole roster.
fn kind_strings(raw: &serde_json::Value) -> Vec<String> {
    raw.as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The connections of one installation — the roster row's expansion.
pub async fn list_connections_for_installation(
    conn: &mut PgConnection,
    org_id: Uuid,
    installation_id: Uuid,
) -> DbResult<Vec<Connection>> {
    let rows = sqlx::query(&format!(
        "SELECT {CONNECTION_COLUMNS} FROM connector_connections
         WHERE org_id = $1 AND installation_id = $2 ORDER BY scope_client_id NULLS FIRST, label"
    ))
    .bind(org_id)
    .bind(installation_id)
    .fetch_all(&mut *conn)
    .await?;
    rows.into_iter().map(row_to_connection).collect()
}

pub async fn get_connection(
    conn: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<Option<Connection>> {
    let row = sqlx::query(&format!(
        "SELECT {CONNECTION_COLUMNS} FROM connector_connections
         WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(row_to_connection).transpose()
}

/// Records the outcome of a canary or a real call.
///
/// The timestamp moves with the status in the same statement, because a status
/// without one is a claim and the pair is what the roster shows as evidence.
pub async fn record_connection_outcome(
    conn: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
    status: ConnectionStatus,
    detail: Option<&str>,
    vendor_detail: Option<&str>,
    succeeded: bool,
) -> DbResult<()> {
    sqlx::query(
        "UPDATE connector_connections
            SET status = $3,
                status_detail = $4,
                status_detail_vendor = $5,
                last_success_at = CASE WHEN $6 THEN now() ELSE last_success_at END,
                last_failure_at = CASE WHEN $6 THEN last_failure_at ELSE now() END,
                updated_at = now()
          WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(id)
    .bind(status.as_str())
    .bind(detail)
    .bind(vendor_detail)
    .bind(succeeded)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

// ---- bindings --------------------------------------------------------------

/// One named destination a connection may post to.
#[derive(Debug, Clone)]
pub struct Binding {
    pub id: Uuid,
    pub connection_id: Uuid,
    pub binding_kind: String,
    pub label: String,
    pub external_id: String,
    pub external_parent: Option<String>,
    pub vendor_ids: serde_json::Value,
    pub enabled: bool,
}

/// The destinations the share dialog offers.
///
/// Disabled bindings are excluded by default and can be included for the
/// management surface, which has to show what exists in order to re-enable it.
pub async fn list_bindings(
    conn: &mut PgConnection,
    org_id: Uuid,
    connection_id: Uuid,
    include_disabled: bool,
) -> DbResult<Vec<Binding>> {
    let rows = sqlx::query(
        "SELECT id, connection_id, binding_kind, label, external_id, external_parent,
                vendor_ids, enabled
           FROM connector_bindings
          WHERE org_id = $1 AND connection_id = $2 AND ($3 OR enabled)
          ORDER BY label",
    )
    .bind(org_id)
    .bind(connection_id)
    .bind(include_disabled)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Binding {
            id: r.get("id"),
            connection_id: r.get("connection_id"),
            binding_kind: r.get("binding_kind"),
            label: r.get("label"),
            external_id: r.get("external_id"),
            external_parent: r.get("external_parent"),
            vendor_ids: r.get("vendor_ids"),
            enabled: r.get("enabled"),
        })
        .collect())
}

// ---- external references ---------------------------------------------------

const REF_COLUMNS: &str = "id, org_id, record_type, record_id, connection_id, source_system,
     external_kind, external_id, external_url, source_guid, payload_ref, first_seen_at, last_seen_at";

/// This Marula record is that vendor object.
#[derive(Debug, Clone)]
pub struct ExternalRef {
    pub id: Uuid,
    pub record_type: RecordType,
    pub record_id: Uuid,
    /// `None` for an import-era reference with no live connection behind it.
    pub connection_id: Option<Uuid>,
    pub source_system: String,
    pub external_kind: String,
    pub external_id: String,
    /// A canonical URL the connector learned from the vendor. Used verbatim,
    /// never templated.
    pub external_url: Option<String>,
    pub source_guid: Option<String>,
    pub payload_ref: Option<String>,
    pub first_seen_at: DateTime<Utc>,
    /// `None` when no sync has refreshed this row — an import observation.
    pub last_seen_at: Option<DateTime<Utc>>,
}

fn row_to_ref(r: sqlx::postgres::PgRow) -> DbResult<ExternalRef> {
    let raw: String = r.try_get("record_type")?;
    let record_type = RecordType::parse(&raw).ok_or_else(|| {
        crate::DbError::Corrupt(format!(
            "external_refs.record_type `{raw}` is not in the vocabulary"
        ))
    })?;
    Ok(ExternalRef {
        id: r.try_get("id")?,
        record_type,
        record_id: r.try_get("record_id")?,
        connection_id: r.try_get("connection_id")?,
        source_system: r.try_get("source_system")?,
        external_kind: r.try_get("external_kind")?,
        external_id: r.try_get("external_id")?,
        external_url: r.try_get("external_url")?,
        source_guid: r.try_get("source_guid")?,
        payload_ref: r.try_get("payload_ref")?,
        first_seen_at: r.try_get("first_seen_at")?,
        last_seen_at: r.try_get("last_seen_at")?,
    })
}

/// Every external presence of one record — what the chip row reads.
pub async fn list_refs_for_record(
    conn: &mut PgConnection,
    org_id: Uuid,
    record_type: RecordType,
    record_id: Uuid,
) -> DbResult<Vec<ExternalRef>> {
    let rows = sqlx::query(&format!(
        "SELECT {REF_COLUMNS} FROM external_refs
          WHERE org_id = $1 AND record_type = $2 AND record_id = $3
          ORDER BY source_system, external_kind"
    ))
    .bind(org_id)
    .bind(record_type.as_str())
    .bind(record_id)
    .fetch_all(&mut *conn)
    .await?;
    rows.into_iter().map(row_to_ref).collect()
}

/// The reference for one vendor object, if this org already holds it.
///
/// Used by a sync to decide "update mine" versus "bind" without relying on the
/// unique index to raise and be caught, which would log a failure for a normal
/// second run.
pub async fn find_ref_by_external(
    conn: &mut PgConnection,
    org_id: Uuid,
    connection_id: Option<Uuid>,
    source_system: &str,
    external_kind: &str,
    external_id: &str,
) -> DbResult<Option<ExternalRef>> {
    let row = match connection_id {
        Some(connection) => {
            sqlx::query(&format!(
                "SELECT {REF_COLUMNS} FROM external_refs
                  WHERE org_id = $1 AND connection_id = $2 AND external_kind = $3 AND external_id = $4"
            ))
            .bind(org_id)
            .bind(connection)
            .bind(external_kind)
            .bind(external_id)
            .fetch_optional(&mut *conn)
            .await?
        }
        None => {
            sqlx::query(&format!(
                "SELECT {REF_COLUMNS} FROM external_refs
                  WHERE org_id = $1 AND connection_id IS NULL
                    AND source_system = $2 AND external_kind = $3 AND external_id = $4"
            ))
            .bind(org_id)
            .bind(source_system)
            .bind(external_kind)
            .bind(external_id)
            .fetch_optional(&mut *conn)
            .await?
        }
    };
    row.map(row_to_ref).transpose()
}

/// Binds a live reference, or refreshes the one that already holds this vendor
/// object.
///
/// `ON CONFLICT` targets the partial live-identity index. It cannot rebind the
/// row to a different record: `external_refs_identity_live` is unique per
/// connection, so a second record claiming the same object raises and the
/// caller reports a merge candidate instead of quietly stealing the link.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_live_ref(
    conn: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
    record_type: RecordType,
    record_id: Uuid,
    connection_id: Uuid,
    source_system: &str,
    external_kind: &str,
    external_id: &str,
    external_url: Option<&str>,
    source_guid: Option<&str>,
    payload_ref: Option<&str>,
) -> DbResult<ExternalRef> {
    let row = sqlx::query(&format!(
        "INSERT INTO external_refs
           (id, org_id, record_type, record_id, connection_id, source_system,
            external_kind, external_id, external_url, source_guid, payload_ref, last_seen_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, now())
         ON CONFLICT (org_id, connection_id, external_kind, external_id)
            WHERE connection_id IS NOT NULL
         DO UPDATE SET
            record_id = EXCLUDED.record_id,
            source_system = EXCLUDED.source_system,
            external_url = COALESCE(EXCLUDED.external_url, external_refs.external_url),
            source_guid = COALESCE(EXCLUDED.source_guid, external_refs.source_guid),
            payload_ref = COALESCE(EXCLUDED.payload_ref, external_refs.payload_ref),
            last_seen_at = now()
         RETURNING {REF_COLUMNS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(record_type.as_str())
    .bind(record_id)
    .bind(connection_id)
    .bind(source_system)
    .bind(external_kind)
    .bind(external_id)
    .bind(external_url)
    .bind(source_guid)
    .bind(payload_ref)
    .fetch_one(&mut *conn)
    .await?;
    row_to_ref(row)
}

// ---- capability resolution -------------------------------------------------

/// One action a provider offers on this record, ready to render.
///
/// `kind` is the closed behaviour vocabulary (`open` | `launch`). Held as a
/// string because this struct crosses the API boundary unchanged and the
/// behaviour set is already the wire contract in `connector-capabilities.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAction {
    pub kind: String,
    /// The connector names its own action ("Open in …", "Start session").
    /// Core decides where the chip sits, never what it is called.
    pub label: String,
    /// `None` only for a launch that needs an adapter method — not expressible
    /// in the manifest yet, so today every emitted action carries a URL.
    pub url: Option<String>,
}

/// One connection that can act for one capability on this record.
#[derive(Debug, Clone)]
pub struct ResolvedProvider {
    pub connection_id: Uuid,
    /// The installed manifest's name.
    pub connector: String,
    /// The operator's label for this connection ("NinjaOne — Acme"), because
    /// two connections of one vendor are the normal case and the label is what
    /// tells them apart.
    pub label: String,
    /// Kept for the UI to mark a degraded provider. A provider that cannot act
    /// is still present with its `open` action, never removed: a missing tool
    /// reads as "we don't have that".
    pub status: ConnectionStatus,
    pub is_primary: bool,
    pub actions: Vec<ResolvedAction>,
}

/// One capability's provider set on this record — the unit a chip renders.
#[derive(Debug, Clone)]
pub struct ResolvedCapability {
    pub kind: String,
    pub providers: Vec<ResolvedProvider>,
}

/// The record's client scope, for two decisions: whether a client-scoped
/// connection may appear on it, and which `capability_defaults` row applies.
#[derive(Debug, Clone)]
pub struct RecordScope {
    /// `None` for an internal-org asset or a ticket with no client.
    pub client_id: Option<Uuid>,
    /// The client's `restricted_to` list. The caller applies it; this module
    /// does not decide access, it only reports what the record says so the
    /// route does not have to know five record tables.
    pub restricted_to: Vec<Uuid>,
}

/// The record's client and its restriction list, or `None` when this product
/// has no such record type (the resolver still runs, with no client scope).
///
/// Why the table match lives here rather than in the route: `external_refs` is
/// record-type-agnostic, so exactly one place has to know the five tables that
/// carry a `client_id`. Keeping it beside the resolver means the chip row and
/// the capability defaults cannot disagree about which client a record belongs
/// to.
pub async fn resolve_record_scope(
    conn: &mut PgConnection,
    org_id: Uuid,
    record_type: RecordType,
    record_id: Uuid,
) -> DbResult<Option<RecordScope>> {
    match record_type {
        RecordType::Configuration => {
            scope_from_table(conn, org_id, "device_assets", record_id).await
        }
        RecordType::Ticket => scope_from_table(conn, org_id, "tickets", record_id).await,
        RecordType::Invoice => scope_from_table(conn, org_id, "invoices", record_id).await,
        RecordType::Contact => scope_from_table(conn, org_id, "contacts", record_id).await,
        // A client is its own scope, and carries the restriction list.
        RecordType::Client => {
            let row =
                sqlx::query("SELECT restricted_to FROM clients WHERE org_id = $1 AND id = $2")
                    .bind(org_id)
                    .bind(record_id)
                    .fetch_optional(&mut *conn)
                    .await?;
            let Some(row) = row else {
                return Err(crate::DbError::NotFound);
            };
            let raw: serde_json::Value = row.try_get("restricted_to")?;
            Ok(Some(RecordScope {
                client_id: Some(record_id),
                restricted_to: restriction_list(&raw),
            }))
        }
        // The remaining record types have no client column this resolver needs
        // (site, KB article, accounting documents that are not invoices). They
        // still resolve actions; they simply cannot pick a client default.
        _ => Ok(None),
    }
}

/// The two-column answer for a table whose `client_id` may be NULL.
async fn scope_from_table(
    conn: &mut PgConnection,
    org_id: Uuid,
    table: &str,
    record_id: Uuid,
) -> DbResult<Option<RecordScope>> {
    // `table` is a literal from the caller's match, never a bound value, so no
    // user input reaches the statement text.
    let sql = format!("SELECT client_id FROM {table} WHERE org_id = $1 AND id = $2");
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(record_id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(crate::DbError::NotFound)?;
    let client_id: Option<Uuid> = row.try_get("client_id")?;
    let restricted_to = match client_id {
        Some(client) => {
            let row =
                sqlx::query("SELECT restricted_to FROM clients WHERE org_id = $1 AND id = $2")
                    .bind(org_id)
                    .bind(client)
                    .fetch_optional(&mut *conn)
                    .await?;
            row.map(|r| {
                r.try_get::<serde_json::Value, _>("restricted_to")
                    .map(|v| restriction_list(&v))
            })
            .transpose()?
            .unwrap_or_default()
        }
        None => Vec::new(),
    };
    Ok(Some(RecordScope {
        client_id,
        restricted_to,
    }))
}

/// `clients.restricted_to` is a JSON array of user ids; malformed entries are
/// dropped rather than denied, because an unreadable restriction list must not
/// turn into "nobody may see this client" for every record it owns.
fn restriction_list(raw: &serde_json::Value) -> Vec<Uuid> {
    raw.as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str()?.parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// The manifest fields the resolver needs, and nothing else.
///
/// Parsed from the pinned `plugin_versions.manifest` document instead of the
/// entity, because a manifest written by another core version may carry fields
/// this build does not know; the resolver must read the two blocks it uses and
/// ignore the rest rather than refuse the whole record's chip row.
#[derive(Debug, Default, serde::Deserialize)]
struct SurfaceManifest {
    #[serde(default)]
    product_kinds: Vec<String>,
    #[serde(default)]
    connector_surfaces: Option<ConnectorSurfaces>,
}

/// Turns the stored rows into the payload every contextual surface reads.
///
/// The rules it enforces are the correspondence between ADR 0032's model and
/// §6.7's contract:
///
/// * a `disabled` connection is skipped, and an unresolved reference (no
///   connection) contributes nothing — the record surface reports those
///   separately as "unresolved", never as a button that fails on press;
/// * a client-scoped connection appears only on records of its own client, so
///   a per-client controller never leaks onto another client's device;
/// * a connection that cannot act keeps `open` and loses `launch`: a session
///   must never start from a credential the vendor already refused;
/// * a template that names a placeholder nothing can resolve yields no action
///   at all. A broken link is worse than a missing one;
/// * `capability_defaults` marks the primary provider for the record's client
///   (the client row beating the org row); with several providers and no
///   default, none is primary, which makes the UI open a menu instead of
///   guessing.
///
/// Non-purpose: no permission decision. The caller has already decided the
/// record is readable, and a chip never widens what its reader may see.
pub async fn resolve_external_actions(
    conn: &mut PgConnection,
    org_id: Uuid,
    record_type: RecordType,
    record_id: Uuid,
) -> DbResult<Vec<ResolvedCapability>> {
    // The record's own scope decides both the connection filter and the
    // default lookup. A record type that cannot be scoped answers `None`, and
    // only org-level defaults apply to it.
    let scope = match resolve_record_scope(conn, org_id, record_type, record_id).await {
        Ok(scope) => scope,
        // The route checks existence first; a missing record here is simply a
        // record with nothing to resolve, not an error the chip row should
        // turn into a 500.
        Err(crate::DbError::NotFound) => None,
        Err(other) => return Err(other),
    };
    let record_client = scope.as_ref().and_then(|s| s.client_id);

    // One statement: a record's references, the connection each resolves
    // through, and the pinned manifest that says what that connection can do.
    let rows = sqlx::query(
        "SELECT r.connection_id, r.external_kind, r.external_id, r.external_url,
                c.label AS connection_label, c.status, c.settings, c.scope_client_id,
                v.manifest
           FROM external_refs r
           JOIN connector_connections c
             ON c.id = r.connection_id AND c.org_id = r.org_id
           JOIN plugin_installs i
             ON i.id = c.installation_id AND i.org_id = c.org_id
           JOIN plugin_versions v
             ON v.catalog_id = i.catalog_id AND v.version = i.version
          WHERE r.org_id = $1 AND r.record_type = $2 AND r.record_id = $3
            AND c.status <> 'disabled'
          ORDER BY c.label, r.external_kind, r.id",
    )
    .bind(org_id)
    .bind(record_type.as_str())
    .bind(record_id)
    .fetch_all(&mut *conn)
    .await?;

    let defaults = capability_defaults_for(conn, org_id, record_client).await?;

    let mut capabilities: Vec<ResolvedCapability> = Vec::new();

    for row in rows {
        let connection_scope: Option<Uuid> = row.try_get("scope_client_id")?;
        if connection_scope.is_some() && connection_scope != record_client {
            continue;
        }
        let connection_id: Uuid = row.try_get("connection_id")?;
        let label: String = row.try_get("connection_label")?;
        let status = parse_status(&row.try_get::<String, _>("status")?)?;
        let settings: serde_json::Value = row.try_get("settings")?;
        let external_kind: String = row.try_get("external_kind")?;
        let external_id: String = row.try_get("external_id")?;
        let external_url: Option<String> = row.try_get("external_url")?;
        let manifest: serde_json::Value = row.try_get("manifest")?;
        let connector = manifest
            .get("name")
            .and_then(|name| name.as_str())
            .unwrap_or_default()
            .to_string();

        // A manifest that does not parse as the two blocks this resolver reads
        // contributes no actions. It is not an error for this request: the
        // record's other providers must still render.
        let Ok(manifest) = serde_json::from_value::<SurfaceManifest>(manifest) else {
            continue;
        };
        let Some(surfaces) = &manifest.connector_surfaces else {
            continue;
        };
        let Some(block) = surfaces.external_refs.iter().find(|block| {
            block.record == record_type.as_str() && block.external_kind == external_kind
        }) else {
            continue;
        };

        let base_url = base_url(&settings, &base_url_key(&manifest));

        // `open` prefers the URL the connector learned from the vendor, which
        // is used verbatim. A composed URL is the fallback only.
        let open_url = external_url.and_then(|url| safe_link(&url)).or_else(|| {
            block
                .open_url
                .as_deref()
                .and_then(|t| render_template(t, base_url.as_deref(), &external_id, &settings))
        });
        let open_action = open_url.map(|url| ResolvedAction {
            kind: "open".into(),
            label: format!("Open in {label}"),
            url: Some(url),
        });

        // Starting something is a security act, so a connection the vendor has
        // already refused never offers it. The open action above stays, which
        // is the documented degradation.
        //
        // The launch also carries the capability it belongs to, because a chip
        // is per capability and the block is per record: a vendor that is both
        // an RMM and a remote-access tool must not offer "Start session" under
        // the RMM chip. Absent means every declared kind, which is the right
        // default for a single-kind connector.
        let launch_action = if status.can_act() {
            block.launch.as_ref().and_then(|launch| {
                render_template(
                    &launch.url_template,
                    base_url.as_deref(),
                    &external_id,
                    &settings,
                )
                .map(|url| ResolvedAction {
                    kind: "launch".into(),
                    label: "Start session".into(),
                    url: Some(url),
                })
            })
        } else {
            None
        };
        let launch_capability = block
            .launch
            .as_ref()
            .and_then(|launch| launch.capability.as_deref());

        if open_action.is_none() && launch_action.is_none() {
            continue;
        }

        for kind in manifest.product_kinds {
            // A launch attributed to one capability is emitted only there; an
            // unattributed one rides every declared kind. `open` always rides
            // every kind: opening a device page is meaningful under any of them.
            let wants_launch = launch_action.is_some()
                && launch_capability.is_none_or(|capability| capability == kind.as_str());
            let mut actions: Vec<ResolvedAction> = Vec::new();
            if let Some(open) = &open_action {
                actions.push(open.clone());
            }
            if wants_launch {
                if let Some(launch) = &launch_action {
                    actions.push(launch.clone());
                }
            }
            // A kind that would get no action is not a capability on this
            // record: an empty chip is what an unlinked tool looks like.
            if actions.is_empty() {
                continue;
            }

            let index = match capabilities.iter().position(|c| c.kind == kind) {
                Some(index) => index,
                None => {
                    capabilities.push(ResolvedCapability {
                        kind,
                        providers: Vec::new(),
                    });
                    capabilities.len() - 1
                }
            };

            let capability = &mut capabilities[index];
            match capability
                .providers
                .iter_mut()
                .find(|p| p.connection_id == connection_id)
            {
                Some(provider) => {
                    for action in actions.iter() {
                        if !provider.actions.contains(action) {
                            provider.actions.push(action.clone());
                        }
                    }
                }
                None => capability.providers.push(ResolvedProvider {
                    connection_id,
                    connector: connector.clone(),
                    label: label.clone(),
                    status,
                    is_primary: false,
                    actions,
                }),
            }
        }
    }

    // The primary is the capability's default for THIS record's client. A
    // single provider needs no default to be the obvious press target; with
    // several and no default, none is primary on purpose.
    for capability in &mut capabilities {
        let chosen = defaults.get(&capability.kind).copied();
        let single = capability.providers.len() == 1;
        for provider in &mut capability.providers {
            provider.is_primary =
                chosen == Some(provider.connection_id) || (chosen.is_none() && single);
        }
        // Primary first, then the operator's labels: a stable order so a
        // re-render cannot reshuffle a menu under the pointer.
        capability.providers.sort_by(|a, b| {
            b.is_primary
                .cmp(&a.is_primary)
                .then_with(|| a.label.cmp(&b.label))
                .then_with(|| a.connection_id.cmp(&b.connection_id))
        });
    }

    Ok(capabilities)
}

/// The org default for each capability, with the record client's row winning.
///
/// One query, ordered so the client row sorts before the org row: the map
/// keeps the first answer it sees, which is the override. A connection that
/// does not appear among a capability's providers is simply never matched, so
/// a stale default marks nothing primary rather than inventing one.
async fn capability_defaults_for(
    conn: &mut PgConnection,
    org_id: Uuid,
    record_client: Option<Uuid>,
) -> DbResult<std::collections::HashMap<String, Uuid>> {
    let rows = sqlx::query(
        "SELECT capability, connection_id
           FROM capability_defaults
          WHERE org_id = $1
            AND (scope_client_id IS NULL OR scope_client_id = $2::uuid)
          ORDER BY scope_client_id NULLS LAST",
    )
    .bind(org_id)
    .bind(record_client)
    .fetch_all(&mut *conn)
    .await?;
    let mut defaults = std::collections::HashMap::with_capacity(rows.len());
    for row in rows {
        let capability: String = row.try_get("capability")?;
        let connection_id: Uuid = row.try_get("connection_id")?;
        defaults.entry(capability).or_insert(connection_id);
    }
    Ok(defaults)
}

/// The connection's base URL, as `settings.base_url`.
///
/// There is no manifest field naming the key: `settings_schema` describes the
/// shape of a connection's settings, not which one is the origin, and every
/// connector that builds a link today stores its origin under `base_url`. A
/// connection that does not carry one resolves `{base_url}` to nothing, which
/// drops the action instead of composing a link against an invented host.
fn base_url(settings: &serde_json::Value, key: &str) -> Option<String> {
    let raw = settings.get(key)?.as_str()?.trim();
    let raw = raw.trim_end_matches('/');
    let rest = raw.strip_prefix("https://")?;
    if rest.is_empty() || rest.starts_with('/') || rest.contains('@') {
        return None;
    }
    Some(raw.to_string())
}

/// The settings key holding the connection's origin: the manifest's declared
/// `connector_surfaces.base_url_key`, or the conventional `base_url` when it
/// declares none.
fn base_url_key(manifest: &SurfaceManifest) -> String {
    manifest
        .connector_surfaces
        .as_ref()
        .and_then(|surfaces| surfaces.base_url_key.clone())
        .unwrap_or_else(|| "base_url".to_string())
}

/// Renders one declared template, or `None` when any placeholder cannot be
/// resolved. Every value that lands in the URL is percent-encoded as a path
/// segment, so a device id containing `/` or `?` cannot rewrite the link.
fn render_template(
    template: &str,
    base_url: Option<&str>,
    external_id: &str,
    settings: &serde_json::Value,
) -> Option<String> {
    let mut out = String::with_capacity(template.len() + 64);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after.find('}')?;
        let token = &after[..close];
        match token {
            "base_url" => out.push_str(base_url?),
            "external_id" => out.push_str(&encode_path_segment(external_id)),
            _ => {
                let key = token.strip_prefix("settings.")?;
                let value = settings.get(key)?.as_str()?;
                out.push_str(&encode_path_segment(value));
            }
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    safe_link(&out)
}

/// The schemes a rendered action may carry.
///
/// The manifest gate already proves the template's own origin; the only
/// unvalidated input at press time is the base URL and settings, so this is
/// the second line: no `javascript:`, no `data:`, no bare relative string.
fn safe_link(url: &str) -> Option<String> {
    if url.is_empty() || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("tel:") {
        Some(url.to_string())
    } else {
        None
    }
}

/// Percent-encodes one path segment, keeping only RFC 3986 unreserved bytes.
///
/// Hand-rolled because `core-db` carries no URL crate and this is the only
/// place a value becomes a URL component; a dependency for nine lines is not
/// worth the supply-chain surface.
fn encode_path_segment(raw: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(raw.len());
    for byte in raw.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(*byte as char);
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    out
}

// ---- record links ----------------------------------------------------------

const LINK_COLUMNS: &str = "id, org_id, from_kind, from_id, to_kind, to_id, created_by, created_at";

/// One typed edge between two Marula records.
#[derive(Debug, Clone)]
pub struct RecordLink {
    pub id: Uuid,
    pub from_kind: LinkEndKind,
    pub from_id: Uuid,
    pub to_kind: LinkEndKind,
    pub to_id: Uuid,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

fn parse_end_kind(raw: &str) -> DbResult<LinkEndKind> {
    LinkEndKind::parse(raw).ok_or_else(|| {
        crate::DbError::Corrupt(format!(
            "record_links kind `{raw}` is not in the vocabulary"
        ))
    })
}

fn row_to_link(r: sqlx::postgres::PgRow) -> DbResult<RecordLink> {
    Ok(RecordLink {
        id: r.try_get("id")?,
        from_kind: parse_end_kind(&r.try_get::<String, _>("from_kind")?)?,
        from_id: r.try_get("from_id")?,
        to_kind: parse_end_kind(&r.try_get::<String, _>("to_kind")?)?,
        to_id: r.try_get("to_id")?,
        created_by: r.try_get("created_by")?,
        created_at: r.try_get("created_at")?,
    })
}

/// Links one record to another, refusing a pair no surface can draw.
///
/// The refusal is here as well as in the database on purpose: the router can
/// say which pair was refused and why, and the constraint still holds if some
/// other path reaches the table first.
pub async fn create_record_link(
    conn: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
    from: (LinkEndKind, Uuid),
    to: (LinkEndKind, Uuid),
    created_by: Option<Uuid>,
) -> DbResult<RecordLink> {
    let (from_kind, from_id) = from;
    let (to_kind, to_id) = to;
    if from_kind == to_kind && from_id == to_id {
        return Err(crate::DbError::Conflict(
            "a record cannot be linked to itself".into(),
        ));
    }
    if !LinkEndKind::pair_allowed(from_kind, to_kind) {
        return Err(crate::DbError::Conflict(format!(
            "the pair {} -> {} is not a supported link",
            from_kind.as_str(),
            to_kind.as_str()
        )));
    }

    let inserted = sqlx::query(&format!(
        "INSERT INTO record_links (id, org_id, from_kind, from_id, to_kind, to_id, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (org_id, from_kind, from_id, to_kind, to_id) DO NOTHING
         RETURNING {LINK_COLUMNS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(from_kind.as_str())
    .bind(from_id)
    .bind(to_kind.as_str())
    .bind(to_id)
    .bind(created_by)
    .fetch_optional(&mut *conn)
    .await?;

    if let Some(row) = inserted {
        return row_to_link(row);
    }

    // Already linked: linking twice is idempotent, not an error, because the
    // surface that offers it is a checkbox and not a create form.
    let row = sqlx::query(&format!(
        "SELECT {LINK_COLUMNS} FROM record_links
          WHERE org_id = $1 AND from_kind = $2 AND from_id = $3 AND to_kind = $4 AND to_id = $5"
    ))
    .bind(org_id)
    .bind(from_kind.as_str())
    .bind(from_id)
    .bind(to_kind.as_str())
    .bind(to_id)
    .fetch_one(&mut *conn)
    .await?;
    row_to_link(row)
}

/// The edges leaving one record — a ticket's configurations, for instance.
pub async fn list_links_from(
    conn: &mut PgConnection,
    org_id: Uuid,
    kind: LinkEndKind,
    record_id: Uuid,
) -> DbResult<Vec<RecordLink>> {
    let rows = sqlx::query(&format!(
        "SELECT {LINK_COLUMNS} FROM record_links
          WHERE org_id = $1 AND from_kind = $2 AND from_id = $3 ORDER BY created_at"
    ))
    .bind(org_id)
    .bind(kind.as_str())
    .bind(record_id)
    .fetch_all(&mut *conn)
    .await?;
    rows.into_iter().map(row_to_link).collect()
}

/// The edges arriving at one record — the "linked tickets" side of a
/// configuration, and the reverse direction the strip must not need to know
/// about.
pub async fn list_links_to(
    conn: &mut PgConnection,
    org_id: Uuid,
    kind: LinkEndKind,
    record_id: Uuid,
) -> DbResult<Vec<RecordLink>> {
    let rows = sqlx::query(&format!(
        "SELECT {LINK_COLUMNS} FROM record_links
          WHERE org_id = $1 AND to_kind = $2 AND to_id = $3 ORDER BY created_at"
    ))
    .bind(org_id)
    .bind(kind.as_str())
    .bind(record_id)
    .fetch_all(&mut *conn)
    .await?;
    rows.into_iter().map(row_to_link).collect()
}

/// Removes one edge. Returns whether a row was removed, so the caller can tell
/// an idempotent unlink from a stale id.
pub async fn delete_record_link(conn: &mut PgConnection, org_id: Uuid, id: Uuid) -> DbResult<bool> {
    let done = sqlx::query("DELETE FROM record_links WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(&mut *conn)
        .await?;
    Ok(done.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_status_parse_refuses_an_unknown_value() {
        let err = parse_status("very_connected").unwrap_err();
        assert!(matches!(err, crate::DbError::Corrupt(_)));
    }

    #[test]
    fn record_type_parse_refuses_an_unknown_value() {
        assert!(RecordType::parse("vehicles").is_none());
        assert_eq!(RecordType::parse("ticket"), Some(RecordType::Ticket));
    }

    /// The origin's settings key is declared, not assumed. A connector whose
    /// setting is called `controller_url` must not have `{base_url}` read the
    /// wrong field, and one that says nothing must still work.
    #[test]
    fn a_declared_base_url_key_names_the_origin_setting() {
        let declared = SurfaceManifest {
            product_kinds: Vec::new(),
            connector_surfaces: Some(core_domain::entities::ConnectorSurfaces {
                external_refs: Vec::new(),
                base_url_key: Some("controller_url".into()),
            }),
        };
        let settings = serde_json::json!({
            "controller_url": "https://unifi.example.test/",
            "base_url": "https://wrong.example.test"
        });
        assert_eq!(
            base_url(&settings, &base_url_key(&declared)).as_deref(),
            Some("https://unifi.example.test")
        );

        let conventional = SurfaceManifest {
            product_kinds: Vec::new(),
            connector_surfaces: None,
        };
        assert_eq!(base_url_key(&conventional), "base_url");
    }

    #[test]
    fn link_end_kind_round_trips_through_its_column_values() {
        for kind in LinkEndKind::ALL {
            assert_eq!(LinkEndKind::parse(kind.as_str()), Some(*kind));
        }
    }

    /// A template is rendered from data, so the encoding half is the security
    /// half: an external id cannot rewrite the path, and a placeholder nothing
    /// resolves must produce no action rather than a URL with braces in it.
    #[test]
    fn template_rendering_encodes_the_id_and_refuses_an_unresolved_placeholder() {
        let settings =
            serde_json::json!({ "base_url": "https://rmm.example.test/", "org": "acme" });
        let manifest = SurfaceManifest {
            product_kinds: Vec::new(),
            connector_surfaces: None,
        };
        let base = base_url(&settings, &base_url_key(&manifest));
        assert_eq!(base.as_deref(), Some("https://rmm.example.test"));

        assert_eq!(
            render_template(
                "{base_url}/device/{external_id}/join",
                base.as_deref(),
                "a/b?c",
                &settings
            )
            .as_deref(),
            Some("https://rmm.example.test/device/a%2Fb%3Fc/join")
        );
        assert_eq!(
            render_template(
                "{base_url}/org/{settings.org}",
                base.as_deref(),
                "x",
                &settings
            )
            .as_deref(),
            Some("https://rmm.example.test/org/acme")
        );
        // A setting the connection does not carry, or a base URL it lacks,
        // yields no link at all.
        assert!(render_template(
            "{base_url}/o/{settings.missing}",
            base.as_deref(),
            "x",
            &settings
        )
        .is_none());
        assert!(render_template("{base_url}/device/{external_id}", None, "x", &settings).is_none());
        // `{base_url}` must be https: a plain-http origin would downgrade the
        // link the manifest gate was written to keep safe.
        let http = serde_json::json!({ "base_url": "http://rmm.example.test" });
        assert!(base_url(&http, "base_url").is_none());
    }

    /// The URL a provider hands the browser is checked again at press time,
    /// because the learned `external_url` never passed the manifest gate.
    #[test]
    fn unsafe_schemes_are_never_emitted() {
        assert!(safe_link("https://ok.test/x").is_some());
        assert!(safe_link("tel:+15551234").is_some());
        assert!(safe_link("javascript:alert(1)").is_none());
        assert!(safe_link("data:text/html,x").is_none());
        assert!(safe_link("//relative.test/x").is_none());
        assert!(safe_link("https://ok.test/a b").is_none());
    }
}
