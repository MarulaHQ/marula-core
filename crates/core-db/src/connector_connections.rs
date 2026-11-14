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

    #[test]
    fn link_end_kind_round_trips_through_its_column_values() {
        for kind in LinkEndKind::ALL {
            assert_eq!(LinkEndKind::parse(kind.as_str()), Some(*kind));
        }
    }
}
