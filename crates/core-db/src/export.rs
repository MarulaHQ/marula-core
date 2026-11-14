//! Org-data export loader (the private build tracker D1, portability/DR).
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
    ExportAsset, ExportAttachment, ExportAuditEvent, ExportBoardDetails, ExportBoardOperational,
    ExportBoardStatusDetails, ExportBundle, ExportCapabilityDefault, ExportClient, ExportComment,
    ExportConnectorBinding, ExportConnectorConnection, ExportContact, ExportContract,
    ExportCreditNote, ExportCreditNoteApplication, ExportDocument, ExportDocumentLink,
    ExportDocumentVersion, ExportExternalRef, ExportFieldValue, ExportInvoice, ExportInvoiceLine,
    ExportInvoicePayment, ExportInvoiceRun, ExportKbArticle, ExportKbCategory, ExportOrg,
    ExportPluginInstall, ExportRecordLink, ExportSections, ExportSite, ExportTag, ExportTicket,
    ExportTicketTag, ExportTimeEntry,
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

    // Money documents. Each loader is ONE query for its table (plus one for
    // the relation it nests), grouped in memory — the bundle is never an
    // N+1 over an org.
    let contracts = load_contracts(&mut *conn, org_id).await?;
    let invoice_runs = load_invoice_runs(&mut *conn, org_id).await?;
    let invoices = load_invoices(&mut *conn, org_id).await?;
    let credit_notes = load_credit_notes(&mut *conn, org_id).await?;

    let assets = load_assets(&mut *conn, org_id).await?;
    let kb_categories = load_kb_categories(&mut *conn, org_id).await?;
    let kb_articles = load_kb_articles(&mut *conn, org_id).await?;
    let board_operational = load_board_operational(&mut *conn, org_id).await?;
    let audit_events = load_audit_events(&mut *conn, org_id).await?;

    let plugin_installs = load_plugin_installs(&mut *conn, org_id).await?;
    let connector_connections = load_connector_connections(&mut *conn, org_id).await?;
    let capability_defaults = load_capability_defaults(&mut *conn, org_id).await?;
    let external_refs = load_external_refs(&mut *conn, org_id).await?;
    let record_links = load_record_links(&mut *conn, org_id).await?;

    let documents = load_documents(&mut *conn, org_id).await?;
    let document_links = load_document_links(&mut *conn, org_id).await?;
    let tags = load_tags(&mut *conn, org_id).await?;
    let ticket_tags = load_ticket_tags(&mut *conn, org_id).await?;
    let attachments = load_attachments(&mut *conn, org_id).await?;

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
            contracts,
            invoice_runs,
            invoices,
            credit_notes,
            assets,
            kb_categories,
            kb_articles,
            board_operational,
            audit_events,
            plugin_installs,
            connector_connections,
            capability_defaults,
            external_refs,
            record_links,
            documents,
            document_links,
            tags,
            ticket_tags,
            attachments,
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
        "SELECT id, ticket_id, phase_id, user_id, started_at, ended_at, minutes, billable,
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
            phase_id: r.phase_id,
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

// --- money documents ------------------------------------------------------------

/// Contract headers, ordered by the human contract number. Carried because
/// `invoices.contract_id` is NOT NULL: an invoice's agreement must resolve in
/// the target, or the money document arrives with a dangling parent. The
/// attached schedules (rates, fees, blocks, caps, minimums, escalators,
/// change orders) are a named residual gap, not a silent omission.
async fn load_contracts(conn: &mut PgConnection, org_id: Uuid) -> DbResult<Vec<ExportContract>> {
    let rows = sqlx::query!(
        "SELECT id, client_id, number, name, contract_type, status, currency,
                effective_from, effective_until, billing_frequency, billing_anchor,
                amount_minor, hourly_rate_minor, included_minutes, overage_rate_minor,
                unit_price_minor, unit_basis, incident_price_minor, incident_priority,
                po_number, notes, created_at, updated_at
         FROM contracts WHERE org_id = $1 ORDER BY number, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ExportContract {
            id: r.id,
            client_id: r.client_id,
            number: r.number,
            name: r.name,
            contract_type: r.contract_type,
            status: r.status,
            currency: r.currency,
            effective_from: r.effective_from,
            effective_until: r.effective_until,
            billing_frequency: r.billing_frequency,
            billing_anchor: r.billing_anchor,
            amount_minor: r.amount_minor,
            hourly_rate_minor: r.hourly_rate_minor,
            included_minutes: r.included_minutes,
            overage_rate_minor: r.overage_rate_minor,
            unit_price_minor: r.unit_price_minor,
            unit_basis: r.unit_basis,
            incident_price_minor: r.incident_price_minor,
            incident_priority: r.incident_priority,
            po_number: r.po_number,
            notes: r.notes,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
        .collect())
}

/// Billing runs, ordered by their period (then number) — the order an
/// operator reads a run list in.
async fn load_invoice_runs(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<ExportInvoiceRun>> {
    let rows = sqlx::query!(
        "SELECT id, number, period_from, period_to, invoice_count, total_minor,
                currency, due_in_days, created_at
         FROM invoice_runs WHERE org_id = $1 ORDER BY period_from, number, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ExportInvoiceRun {
            id: r.id,
            number: r.number,
            period_from: r.period_from,
            period_to: r.period_to,
            invoice_count: r.invoice_count,
            total_minor: r.total_minor,
            currency: r.currency,
            due_in_days: r.due_in_days,
            created_at: r.created_at,
        })
        .collect())
}

/// Invoices with lines, receipts and consumed time-entry ids grouped in
/// memory: four queries for the whole section, never one per invoice.
async fn load_invoices(conn: &mut PgConnection, org_id: Uuid) -> DbResult<Vec<ExportInvoice>> {
    let rows = sqlx::query!(
        r#"SELECT id, run_id, client_id, contract_id, number, status, currency,
                  period_from, period_to, issued_at, due_on, total_minor,
                  total_paid_minor, fx_rate_used::text AS fx_rate_used, fx_rate_date,
                  fx_source, base_total_minor, dunning_state, dunning_updated_at,
                  dunning_note, po_number, notes, created_by, created_at, updated_at
           FROM invoices WHERE org_id = $1 ORDER BY number, id"#,
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;

    let line_rows = sqlx::query!(
        r#"SELECT id, invoice_id, contract_id, kind, description, period_start,
                  period_end, quantity_minutes, unit_amount_minor, amount_minor,
                  position, fx_rate_used::text AS fx_rate_used, fx_rate_date,
                  fx_source, base_amount_minor
           FROM invoice_lines WHERE org_id = $1 ORDER BY invoice_id, position, id"#,
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut lines_by_invoice: BTreeMap<Uuid, Vec<ExportInvoiceLine>> = BTreeMap::new();
    for l in line_rows {
        lines_by_invoice
            .entry(l.invoice_id)
            .or_default()
            .push(ExportInvoiceLine {
                id: l.id,
                contract_id: l.contract_id,
                kind: l.kind,
                description: l.description,
                period_start: l.period_start,
                period_end: l.period_end,
                quantity_minutes: l.quantity_minutes,
                unit_amount_minor: l.unit_amount_minor,
                amount_minor: l.amount_minor,
                position: l.position,
                fx_rate_used: l.fx_rate_used,
                fx_rate_date: l.fx_rate_date,
                fx_source: l.fx_source,
                base_amount_minor: l.base_amount_minor,
            });
    }

    let payment_rows = sqlx::query!(
        r#"SELECT id, invoice_id, amount_minor, currency, provider, provider_ref,
                  notes, received_at, created_by, fx_rate_used::text AS fx_rate_used,
                  fx_rate_date, fx_source, base_amount_minor
           FROM invoice_payments WHERE org_id = $1 ORDER BY invoice_id, received_at, id"#,
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut payments_by_invoice: BTreeMap<Uuid, Vec<ExportInvoicePayment>> = BTreeMap::new();
    for p in payment_rows {
        payments_by_invoice
            .entry(p.invoice_id)
            .or_default()
            .push(ExportInvoicePayment {
                id: p.id,
                amount_minor: p.amount_minor,
                currency: p.currency,
                provider: p.provider,
                provider_ref: p.provider_ref,
                notes: p.notes,
                received_at: p.received_at,
                created_by: p.created_by,
                fx_rate_used: p.fx_rate_used,
                fx_rate_date: p.fx_rate_date,
                fx_source: p.fx_source,
                base_amount_minor: p.base_amount_minor,
            });
    }

    // Which time entries each invoice consumed. Ids only: the consumed entry
    // already travels in `time_entries`.
    let consumed = sqlx::query!(
        "SELECT invoice_id, time_entry_id FROM invoice_time_entries
         WHERE org_id = $1 ORDER BY invoice_id, time_entry_id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut consumed_by_invoice: BTreeMap<Uuid, Vec<Uuid>> = BTreeMap::new();
    for c in consumed {
        consumed_by_invoice
            .entry(c.invoice_id)
            .or_default()
            .push(c.time_entry_id);
    }

    Ok(rows
        .into_iter()
        .map(|i| ExportInvoice {
            lines: lines_by_invoice.remove(&i.id).unwrap_or_default(),
            payments: payments_by_invoice.remove(&i.id).unwrap_or_default(),
            billed_time_entry_ids: consumed_by_invoice.remove(&i.id).unwrap_or_default(),
            id: i.id,
            run_id: i.run_id,
            client_id: i.client_id,
            contract_id: i.contract_id,
            number: i.number,
            status: i.status,
            currency: i.currency,
            period_from: i.period_from,
            period_to: i.period_to,
            issued_at: i.issued_at,
            due_on: i.due_on,
            total_minor: i.total_minor,
            total_paid_minor: i.total_paid_minor,
            fx_rate_used: i.fx_rate_used,
            fx_rate_date: i.fx_rate_date,
            fx_source: i.fx_source,
            base_total_minor: i.base_total_minor,
            dunning_state: i.dunning_state,
            dunning_updated_at: i.dunning_updated_at,
            dunning_note: i.dunning_note,
            po_number: i.po_number,
            notes: i.notes,
            created_by: i.created_by,
            created_at: i.created_at,
            updated_at: i.updated_at,
        })
        .collect())
}

/// Credit notes with their applications nested, ordered by the human number.
/// A credit note is money: `total/applied/refunded_minor` are the document's
/// state, and the applications are the allocations that moved it.
async fn load_credit_notes(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<ExportCreditNote>> {
    let rows = sqlx::query!(
        "SELECT id, number, client_id, issued_against_invoice_id, status, currency,
                total_minor, applied_minor, refunded_minor, reason, created_by,
                created_at, updated_at
         FROM credit_notes WHERE org_id = $1 ORDER BY number, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;

    let application_rows = sqlx::query!(
        "SELECT id, credit_note_id, invoice_id, amount_minor, payment_id, created_by,
                created_at
         FROM credit_note_applications WHERE org_id = $1
         ORDER BY credit_note_id, created_at, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut by_note: BTreeMap<Uuid, Vec<ExportCreditNoteApplication>> = BTreeMap::new();
    for a in application_rows {
        by_note
            .entry(a.credit_note_id)
            .or_default()
            .push(ExportCreditNoteApplication {
                id: a.id,
                invoice_id: a.invoice_id,
                amount_minor: a.amount_minor,
                payment_id: a.payment_id,
                created_by: a.created_by,
                created_at: a.created_at,
            });
    }

    Ok(rows
        .into_iter()
        .map(|n| ExportCreditNote {
            id: n.id,
            number: n.number,
            client_id: n.client_id,
            issued_against_invoice_id: n.issued_against_invoice_id,
            status: n.status,
            currency: n.currency,
            total_minor: n.total_minor,
            applied_minor: n.applied_minor,
            refunded_minor: n.refunded_minor,
            reason: n.reason,
            created_by: n.created_by,
            created_at: n.created_at,
            updated_at: n.updated_at,
            applications: by_note.remove(&n.id).unwrap_or_default(),
        })
        .collect())
}

// --- assets ---------------------------------------------------------------------

/// Asset rows ordered by the human tag (client first, so a client's fleet
/// reads together; internal-org rows lead). `observed_fields` and
/// `field_sources` are deliberately absent: the first is feed-written state
/// that whichever connector resumes will re-observe, and the second names
/// `config_source_links` rows superseded by `external_refs` — neither is
/// information the target could use.
async fn load_assets(conn: &mut PgConnection, org_id: Uuid) -> DbResult<Vec<ExportAsset>> {
    let rows = sqlx::query!(
        r#"SELECT id, client_id, site_id, tag, name, kind, status, layout_id, fields,
                  curated_fields, identity_keys, ownership, rental_vendor, rental_start,
                  rental_end, rental_cost_minor, rental_currency, rental_cycle,
                  purchase_date, purchase_cost_minor, purchase_currency,
                  depreciation_method, useful_life_months, salvage_value_minor,
                  depreciation_rate_bps, merged_into_id, scope, created_at, updated_at
           FROM device_assets WHERE org_id = $1
           ORDER BY client_id NULLS FIRST, tag, id"#,
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|a| ExportAsset {
            id: a.id,
            client_id: a.client_id,
            site_id: a.site_id,
            tag: a.tag,
            name: a.name,
            kind: a.kind,
            status: a.status,
            layout_id: a.layout_id,
            fields: a.fields,
            curated_fields: a.curated_fields,
            identity_keys: a.identity_keys,
            ownership: a.ownership,
            rental_vendor: a.rental_vendor,
            rental_start: a.rental_start,
            rental_end: a.rental_end,
            rental_cost_minor: a.rental_cost_minor,
            rental_currency: a.rental_currency,
            rental_cycle: a.rental_cycle,
            purchase_date: a.purchase_date,
            purchase_cost_minor: a.purchase_cost_minor,
            purchase_currency: a.purchase_currency,
            depreciation_method: a.depreciation_method,
            useful_life_months: a.useful_life_months,
            salvage_value_minor: a.salvage_value_minor,
            depreciation_rate_bps: a.depreciation_rate_bps,
            merged_into_id: a.merged_into_id,
            scope: a.scope,
            created_at: a.created_at,
            updated_at: a.updated_at,
        })
        .collect())
}

// --- knowledge base -------------------------------------------------------------

async fn load_kb_categories(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<ExportKbCategory>> {
    let rows = sqlx::query!(
        "SELECT id, name, slug, description, position, client_visible, created_at,
                updated_at
         FROM kb_categories WHERE org_id = $1 ORDER BY name, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|c| ExportKbCategory {
            id: c.id,
            name: c.name,
            slug: c.slug,
            description: c.description,
            position: c.position,
            client_visible: c.client_visible,
            created_at: c.created_at,
            updated_at: c.updated_at,
        })
        .collect())
}

/// Articles ordered by slug — the org-unique key, so a re-import into a fresh
/// org still compares equal.
async fn load_kb_articles(conn: &mut PgConnection, org_id: Uuid) -> DbResult<Vec<ExportKbArticle>> {
    let rows = sqlx::query!(
        "SELECT id, category_id, title, slug, status, client_visible, body,
                content_jsonb, created_by, updated_by, created_at, updated_at
         FROM kb_articles WHERE org_id = $1 ORDER BY slug, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|a| ExportKbArticle {
            id: a.id,
            category_id: a.category_id,
            title: a.title,
            slug: a.slug,
            status: a.status,
            client_visible: a.client_visible,
            body: a.body,
            content_jsonb: a.content_jsonb,
            created_by: a.created_by,
            updated_by: a.updated_by,
            created_at: a.created_at,
            updated_at: a.updated_at,
        })
        .collect())
}

// --- board operational remainder --------------------------------------------------

/// The board columns the U5 manifest does not carry: two flat queries keyed
/// by the ids the manifest already publishes. Nothing here repeats a
/// manifest field, so there is exactly one description of a board in the
/// bundle.
async fn load_board_operational(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<ExportBoardOperational> {
    let boards = sqlx::query!(
        "SELECT id, description, inbound_address, board_group_id
         FROM boards WHERE org_id = $1 ORDER BY name, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;

    let statuses = sqlx::query!(
        "SELECT id, board_id, wip_limit, client_visible
         FROM board_statuses WHERE org_id = $1
         ORDER BY board_id, sort_order, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;

    Ok(ExportBoardOperational {
        boards: boards
            .into_iter()
            .map(|b| ExportBoardDetails {
                board_id: b.id,
                description: b.description,
                inbound_address: b.inbound_address,
                board_group_id: b.board_group_id,
            })
            .collect(),
        statuses: statuses
            .into_iter()
            .map(|s| ExportBoardStatusDetails {
                status_id: s.id,
                board_id: s.board_id,
                wip_limit: s.wip_limit,
                client_visible: s.client_visible,
            })
            .collect(),
    })
}

/// The audit stream, ordered by wall-clock then id. This is the org's record
/// of who did what; ticket history is the nested comment thread plus this
/// stream filtered to the ticket, which is why both are carried.
///
/// Secret discipline: `detail` is the payload the writer chose (a name, a
/// scope list, a field key), and no audit writer puts secret material in it —
/// the at-rest secrets live on rows (`webhooks`, `api_keys`,
/// `connector_connections`) that this loader never selects a secret column
/// from. The export tests seed a webhook signing secret, an API-key hash and a
/// connector credential and assert none of them appears anywhere in the
/// serialized bundle.
async fn load_audit_events(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<ExportAuditEvent>> {
    let rows = sqlx::query!(
        "SELECT id, actor_kind, actor_id, action, entity_type, entity_id, detail, ip,
                occurred_at
         FROM audit_events WHERE org_id = $1 ORDER BY occurred_at, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|e| ExportAuditEvent {
            id: e.id,
            actor_kind: e.actor_kind,
            actor_id: e.actor_id,
            action: e.action,
            entity_type: e.entity_type,
            entity_id: e.entity_id,
            detail: e.detail,
            ip: e.ip,
            occurred_at: e.occurred_at,
        })
        .collect())
}

// --- plugins + connectors ---------------------------------------------------------

/// Installs ordered by install time. One query: the non-secret settings ride
/// a LEFT JOIN, so an install without a settings row carries `{}` instead of
/// costing a second round trip. No credential column is selected; whether a
/// credential binding exists is reported by `has_credentials`, a boolean, so
/// an import knows the operator must re-provision rather than believing an
/// install arrived working.
async fn load_plugin_installs(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<ExportPluginInstall>> {
    let rows = sqlx::query!(
        "SELECT i.id, i.catalog_id, i.version, i.status::text AS \"status!\",
                i.granted_capabilities, i.pending_version, i.pending_manifest,
                i.pending_capabilities, i.installed_by, i.installed_at, i.updated_at,
                i.revoked_at, i.revoked_reason,
                COALESCE(s.config, '{}'::jsonb) AS \"settings!\"
         FROM plugin_installs i
         LEFT JOIN plugin_install_settings s ON s.install_id = i.id
         WHERE i.org_id = $1 ORDER BY i.installed_at, i.id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|i| ExportPluginInstall {
            id: i.id,
            catalog_id: i.catalog_id,
            version: i.version,
            status: i.status,
            granted_capabilities: i.granted_capabilities,
            pending_version: i.pending_version,
            pending_manifest: i.pending_manifest,
            pending_capabilities: i.pending_capabilities,
            settings: i.settings,
            installed_by: i.installed_by,
            installed_at: i.installed_at,
            updated_at: i.updated_at,
            revoked_at: i.revoked_at,
            revoked_reason: i.revoked_reason,
        })
        .collect())
}

/// Connections with their bindings nested. `secrets_encrypted` is never
/// selected — the column exists, the loader does not read it — and the
/// presence of a credential is reduced to a boolean, because the sealed
/// bytes are key-bound to the source install and cannot be opened elsewhere.
async fn load_connector_connections(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<ExportConnectorConnection>> {
    let rows = sqlx::query!(
        "SELECT id, installation_id, scope_client_id, label, settings,
                (secrets_encrypted IS NOT NULL) AS \"has_sealed_credential!\", status,
                status_detail, last_success_at, last_failure_at, created_at, updated_at
         FROM connector_connections WHERE org_id = $1 ORDER BY label, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;

    let binding_rows = sqlx::query!(
        "SELECT id, connection_id, binding_kind, label, external_id, external_parent,
                vendor_ids, enabled, created_at, updated_at
         FROM connector_bindings WHERE org_id = $1
         ORDER BY connection_id, label, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut bindings: BTreeMap<Uuid, Vec<ExportConnectorBinding>> = BTreeMap::new();
    for b in binding_rows {
        bindings
            .entry(b.connection_id)
            .or_default()
            .push(ExportConnectorBinding {
                id: b.id,
                binding_kind: b.binding_kind,
                label: b.label,
                external_id: b.external_id,
                external_parent: b.external_parent,
                vendor_ids: b.vendor_ids,
                enabled: b.enabled,
                created_at: b.created_at,
                updated_at: b.updated_at,
            });
    }

    Ok(rows
        .into_iter()
        .map(|c| ExportConnectorConnection {
            id: c.id,
            installation_id: c.installation_id,
            scope_client_id: c.scope_client_id,
            label: c.label,
            settings: c.settings,
            has_sealed_credential: c.has_sealed_credential,
            status: c.status,
            status_detail: c.status_detail,
            last_success_at: c.last_success_at,
            last_failure_at: c.last_failure_at,
            created_at: c.created_at,
            updated_at: c.updated_at,
            bindings: bindings.remove(&c.id).unwrap_or_default(),
        })
        .collect())
}

async fn load_capability_defaults(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<ExportCapabilityDefault>> {
    let rows = sqlx::query!(
        "SELECT id, scope_client_id, capability, connection_id
         FROM capability_defaults WHERE org_id = $1
         ORDER BY capability, scope_client_id NULLS FIRST, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|d| ExportCapabilityDefault {
            id: d.id,
            scope_client_id: d.scope_client_id,
            capability: d.capability,
            connection_id: d.connection_id,
        })
        .collect())
}

async fn load_external_refs(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<ExportExternalRef>> {
    let rows = sqlx::query!(
        "SELECT id, record_type, record_id, connection_id, source_system,
                external_kind, external_id, external_url, source_guid,
                first_seen_at, last_seen_at
         FROM external_refs WHERE org_id = $1
         ORDER BY record_type, record_id, source_system, external_kind, external_id, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ExportExternalRef {
            id: r.id,
            record_type: r.record_type,
            record_id: r.record_id,
            connection_id: r.connection_id,
            source_system: r.source_system,
            external_kind: r.external_kind,
            external_id: r.external_id,
            external_url: r.external_url,
            source_guid: r.source_guid,
            first_seen_at: r.first_seen_at,
            last_seen_at: r.last_seen_at,
        })
        .collect())
}

async fn load_record_links(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<ExportRecordLink>> {
    let rows = sqlx::query!(
        "SELECT id, from_kind, from_id, to_kind, to_id, created_by, created_at
         FROM record_links WHERE org_id = $1
         ORDER BY from_kind, from_id, to_kind, to_id, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|l| ExportRecordLink {
            id: l.id,
            from_kind: l.from_kind,
            from_id: l.from_id,
            to_kind: l.to_kind,
            to_id: l.to_id,
            created_by: l.created_by,
            created_at: l.created_at,
        })
        .collect())
}

// --- docs platform ---------------------------------------------------------------

/// Documents with every stored version nested — the body is text, so the
/// document moves whole; a title without its body would be a record the
/// target has to re-enter.
async fn load_documents(conn: &mut PgConnection, org_id: Uuid) -> DbResult<Vec<ExportDocument>> {
    let rows = sqlx::query!(
        "SELECT id, title, current_version, client_id, layout_id, fields, created_by,
                updated_by, created_at, updated_at
         FROM documents WHERE org_id = $1 ORDER BY title, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;

    let version_rows = sqlx::query!(
        "SELECT id, document_id, version, title, body, change_summary, author_id,
                restored_from_version, created_at
         FROM document_versions WHERE org_id = $1
         ORDER BY document_id, version, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut versions: BTreeMap<Uuid, Vec<ExportDocumentVersion>> = BTreeMap::new();
    for v in version_rows {
        versions
            .entry(v.document_id)
            .or_default()
            .push(ExportDocumentVersion {
                id: v.id,
                version: v.version,
                title: v.title,
                body: v.body,
                change_summary: v.change_summary,
                author_id: v.author_id,
                restored_from_version: v.restored_from_version,
                created_at: v.created_at,
            });
    }

    Ok(rows
        .into_iter()
        .map(|d| ExportDocument {
            id: d.id,
            title: d.title,
            current_version: d.current_version,
            client_id: d.client_id,
            layout_id: d.layout_id,
            fields: d.fields,
            created_by: d.created_by,
            updated_by: d.updated_by,
            created_at: d.created_at,
            updated_at: d.updated_at,
            versions: versions.remove(&d.id).unwrap_or_default(),
        })
        .collect())
}

async fn load_document_links(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<ExportDocumentLink>> {
    let rows = sqlx::query!(
        "SELECT id, from_kind, from_id, to_kind, to_id, created_by, created_at
         FROM document_links WHERE org_id = $1
         ORDER BY from_kind, from_id, to_kind, to_id, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|l| ExportDocumentLink {
            id: l.id,
            from_kind: l.from_kind,
            from_id: l.from_id,
            to_kind: l.to_kind,
            to_id: l.to_id,
            created_by: l.created_by,
            created_at: l.created_at,
        })
        .collect())
}

// --- tags + attachment metadata -----------------------------------------------------

async fn load_tags(conn: &mut PgConnection, org_id: Uuid) -> DbResult<Vec<ExportTag>> {
    let rows = sqlx::query!(
        "SELECT id, name, color, created_by, created_at, updated_at
         FROM tags WHERE org_id = $1 ORDER BY name, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|t| ExportTag {
            id: t.id,
            name: t.name,
            color: t.color,
            created_by: t.created_by,
            created_at: t.created_at,
            updated_at: t.updated_at,
        })
        .collect())
}

async fn load_ticket_tags(conn: &mut PgConnection, org_id: Uuid) -> DbResult<Vec<ExportTicketTag>> {
    let rows = sqlx::query!(
        "SELECT ticket_id, tag_id, applied_by, applied_at
         FROM ticket_tags WHERE org_id = $1 ORDER BY ticket_id, tag_id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|t| ExportTicketTag {
            ticket_id: t.ticket_id,
            tag_id: t.tag_id,
            applied_by: t.applied_by,
            applied_at: t.applied_at,
        })
        .collect())
}

/// Attachment METADATA for rows owned by a ticket or a ticket comment. The
/// bytes are not read (the loader never touches the ObjectStore) and
/// `storage_key` is not carried: the key is derivable from the content
/// address on the target, and a key copied from the source points at a
/// machine the target cannot reach. Email-owned and sign-off-owned rows are
/// excluded with their owner subsystems, which the bundle does not carry.
async fn load_attachments(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<ExportAttachment>> {
    let rows = sqlx::query!(
        "SELECT id, kind::text AS \"kind!\", ticket_id, ticket_comment_id, filename,
                content_type, size_bytes, sha256, uploaded_by_user_id, created_at
         FROM attachments
         WHERE org_id = $1 AND ticket_id IS NOT NULL
         ORDER BY created_at, id",
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|a| ExportAttachment {
            id: a.id,
            kind: a.kind,
            ticket_id: a.ticket_id,
            ticket_comment_id: a.ticket_comment_id,
            filename: a.filename,
            content_type: a.content_type,
            size_bytes: a.size_bytes,
            sha256: a.sha256,
            uploaded_by_user_id: a.uploaded_by_user_id,
            created_at: a.created_at,
        })
        .collect())
}
