//! Org-data export loader (TASKLIST D1, portability/DR).
//!
//! Purpose: assemble the org's canonical [`ExportBundle`] — the U5
//! customization manifest EMBEDDED VERBATIM plus the operational data plane
//! (org profile, client/site/contact hierarchy, tickets with comment
//! threads, time entries, and every custom-field value) — with the same
//! DETERMINISTIC ordering contract as the manifest: two consecutive dumps
//! of an unchanged org are byte-identical apart from `generated_at`.
//!
//! Ordering rules mirror `manifest`: stable SEMANTIC keys first (names,
//! ticket numbers, instants), row ids only as tiebreakers, so a bundle
//! re-imported into a fresh org (new ids) still compares equal
//! section-for-section.
//!
//! Secret discipline: this module never reads the `users` table — people
//! appear only as opaque user-id references (assignees, comment authors,
//! time-entry owners, client visibility lists) — and the customization
//! section inherits the manifest's structural secret exclusions by
//! delegation. The `custom_field_values` pass-through carries only rows
//! whose definitions belong to this org (join-guarded), so a field
//! definition can never smuggle another tenant's value into the dump.
//!
//! Non-purpose: no import/apply path (the bundle documents and restores
//! via the D2 import machinery), no streaming here (core-api serializes
//! and hands the bytes to the ObjectStore), no users/roles export (not in
//! the D1 portability scope).

use crate::manifest;
use crate::{DbError, DbResult};
use core_domain::entities::export::{
    ExportBundle, ExportClient, ExportComment, ExportContact, ExportFieldValue, ExportOrg,
    ExportSections, ExportSite, ExportTicket, ExportTimeEntry,
};
use core_domain::status::{NoteKind, Priority, TicketStatusCategory};
use sqlx::PgConnection;
use std::collections::BTreeMap;
use std::str::FromStr;
use uuid::Uuid;

/// Envelope version. Bump ONLY for a breaking section reshape; additive
/// section growth keeps this at 1 (same evolution contract as the U5
/// manifest envelope it generalizes).
pub const EXPORT_BUNDLE_VERSION: u32 = 1;

/// Loads the org's full export bundle. Run inside a transaction with
/// `set_org` applied so RLS scopes every query to the tenant (the explicit
/// `org_id` predicates are belt-and-suspenders, matching the manifest
/// loader).
///
/// `generated_at` is injected by the caller so the API layer controls the
/// stamp (and tests can hold it fixed).
pub async fn load_export_bundle(
    conn: &mut PgConnection,
    org_id: Uuid,
    generated_at: chrono::DateTime<chrono::Utc>,
) -> DbResult<ExportBundle> {
    // The customization sections are the manifest loader's output verbatim
    // — one implementation, zero drift between the two surfaces.
    let customization = manifest::load_customization_manifest(conn, org_id, generated_at)
        .await?
        .sections;

    let org_row = sqlx::query!(
        "SELECT name, slug, ticket_prefix, created_at FROM orgs WHERE id = $1",
        org_id
    )
    .fetch_one(&mut *conn)
    .await?;
    let org = ExportOrg {
        name: org_row.name,
        slug: org_row.slug,
        ticket_prefix: org_row.ticket_prefix,
        created_at: org_row.created_at,
    };

    let clients = load_clients(&mut *conn, org_id).await?;
    let sites = load_sites(&mut *conn, org_id).await?;
    let contacts = load_contacts(&mut *conn, org_id).await?;
    let time_entries = load_time_entries(&mut *conn, org_id).await?;
    let field_values = load_custom_field_values(&mut *conn, org_id).await?;
    let tickets = load_tickets(&mut *conn, org_id).await?;

    Ok(ExportBundle {
        bundle_version: EXPORT_BUNDLE_VERSION,
        org_id,
        generated_at,
        sections: ExportSections {
            customization,
            org,
            clients,
            sites,
            contacts,
            tickets,
            time_entries,
            custom_field_values: field_values,
        },
    })
}

// --- client / site / contact hierarchy ----------------------------------------

async fn load_clients(conn: &mut PgConnection, org_id: Uuid) -> DbResult<Vec<ExportClient>> {
    let rows = sqlx::query!(
        "SELECT id, name, restricted_to, notes, created_at, updated_at
         FROM clients WHERE org_id = $1 ORDER BY name, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ExportClient {
            id: r.id,
            name: r.name,
            // `restricted_to` is JSONB (the same widening clients.rs uses).
            restricted_to: serde_json::from_value(r.restricted_to).unwrap_or_default(),
            notes: r.notes,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
        .collect())
}

async fn load_sites(conn: &mut PgConnection, org_id: Uuid) -> DbResult<Vec<ExportSite>> {
    let rows = sqlx::query!(
        "SELECT id, client_id, name, address, city, region, postal_code, country,
                latitude, longitude, timezone, created_at
         FROM sites WHERE org_id = $1 ORDER BY client_id, name, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ExportSite {
            id: r.id,
            client_id: r.client_id,
            name: r.name,
            address: r.address,
            city: r.city,
            region: r.region,
            postal_code: r.postal_code,
            country: r.country,
            latitude: r.latitude,
            longitude: r.longitude,
            timezone: r.timezone,
            created_at: r.created_at,
        })
        .collect())
}

async fn load_contacts(conn: &mut PgConnection, org_id: Uuid) -> DbResult<Vec<ExportContact>> {
    let rows = sqlx::query!(
        "SELECT id, client_id, site_id, name, email, phone, title, is_primary, created_at
         FROM contacts WHERE org_id = $1 ORDER BY client_id, name, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ExportContact {
            id: r.id,
            client_id: r.client_id,
            site_id: r.site_id,
            name: r.name,
            email: r.email,
            phone: r.phone,
            title: r.title,
            is_primary: r.is_primary,
            created_at: r.created_at,
        })
        .collect())
}

// --- tickets + comments ----------------------------------------------------------

/// Loads every ticket with its comment thread grouped in memory. The
/// ticket number is the org's stable semantic key, so it leads the ORDER
/// BY; comments sort by wall-clock then id inside each thread.
async fn load_tickets(conn: &mut PgConnection, org_id: Uuid) -> DbResult<Vec<ExportTicket>> {
    let rows = sqlx::query!(
        "SELECT id, number, board_id, status_id, title, body, priority,
                category, client_id, site_id, contact_id, assignee_id, requester_id,
                due_at, resolved_at, created_at, updated_at
         FROM tickets WHERE org_id = $1 ORDER BY number, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;

    let comments = sqlx::query!(
        "SELECT id, ticket_id, author_id, kind, body, created_at
         FROM ticket_comments WHERE org_id = $1 ORDER BY ticket_id, created_at, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut comments_by_ticket: BTreeMap<Uuid, Vec<ExportComment>> = BTreeMap::new();
    for c in comments {
        comments_by_ticket
            .entry(c.ticket_id)
            .or_default()
            .push(ExportComment {
                id: c.id,
                author_id: c.author_id,
                kind: NoteKind::from_str(&c.kind).map_err(|_| unknown("kind", &c.kind))?,
                body: c.body,
                created_at: c.created_at,
            });
    }

    rows.into_iter()
        .map(|t| {
            Ok(ExportTicket {
                id: t.id,
                number: t.number,
                board_id: t.board_id,
                status_id: t.status_id,
                title: t.title,
                body: t.body,
                priority: Priority::from_str(&t.priority)
                    .map_err(|_| unknown("priority", &t.priority))?,
                category: TicketStatusCategory::from_str(&t.category)
                    .map_err(|_| unknown("category", &t.category))?,
                client_id: t.client_id,
                site_id: t.site_id,
                contact_id: t.contact_id,
                assignee_id: t.assignee_id,
                requester_id: t.requester_id,
                due_at: t.due_at,
                resolved_at: t.resolved_at,
                created_at: t.created_at,
                updated_at: t.updated_at,
                comments: comments_by_ticket.remove(&t.id).unwrap_or_default(),
            })
        })
        .collect()
}

// --- time entries + custom-field values ---------------------------------------------

async fn load_time_entries(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<ExportTimeEntry>> {
    let rows = sqlx::query!(
        "SELECT id, ticket_id, user_id, started_at, ended_at, minutes, billable,
                notes, work_type, unbillable_reason, created_at
         FROM time_entries WHERE org_id = $1 ORDER BY started_at, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ExportTimeEntry {
            id: r.id,
            ticket_id: r.ticket_id,
            user_id: r.user_id,
            started_at: r.started_at,
            ended_at: r.ended_at,
            minutes: r.minutes as i64,
            billable: r.billable,
            notes: r.notes,
            work_type: r.work_type,
            unbillable_reason: r.unbillable_reason,
            created_at: r.created_at,
        })
        .collect())
}

/// Every custom-field value in the org, flat across entity families, with
/// the family resolved from the definition via a join. Sorting is by
/// (entity, entity_id, field_id) — all stable semantic keys.
async fn load_custom_field_values(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<ExportFieldValue>> {
    let rows = sqlx::query!(
        "SELECT d.entity, v.entity_id, v.field_id, v.value
         FROM custom_field_values v
         JOIN custom_field_defs d ON d.id = v.field_id AND d.org_id = v.org_id
         WHERE v.org_id = $1
         ORDER BY d.entity, v.entity_id, v.field_id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ExportFieldValue {
            entity: r.entity,
            entity_id: r.entity_id,
            field_id: r.field_id,
            value: r.value,
        })
        .collect())
}

/// Unknown enum text is a decode failure with a precise field name (the
/// same contract the manifest loader applies to status categories).
fn unknown(field: &str, value: &str) -> DbError {
    DbError::Sql(sqlx::Error::ColumnDecode {
        index: field.into(),
        source: format!("unknown {field} {value:?}").into(),
    })
}
