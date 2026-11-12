//! Template registry + render-context resolution (Wave 13 lane 5, KB3/KB4,
//! migration 0070).
//!
//! The read/write model for the one `Template` entity (ADR 0004): shipped
//! defaults are system rows (`is_system` + `builtin` tag, mirrored from
//! core_domain::templates::DEFAULT_TEMPLATES — the drift test pins the two
//! copies), orgs fork into mutable copies (`forked_from`), org-created
//! templates are first-class rows. Never edit a system row in place.
//!
//! Also resolves render contexts: the joined entity snapshot (ticket +
//! client + contact + assignee + org, project + client, quote + client)
//! the typed-variable renderer consumes. Like the rest of `core-db`,
//! functions are generic over `Executor` and rely on the caller's
//! transaction carrying `app.org_id` (RLS).

use crate::{DbError, DbResult};
use serde_json::Value;
use uuid::Uuid;

/// One `templates` row (kind stays raw SQL text; the API parses it
/// against the closed `TemplateKind` vocabulary).
#[derive(Debug, Clone)]
pub struct TemplateEntry {
    pub id: Uuid,
    pub org_id: Uuid,
    pub kind: String,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub subject: Option<String>,
    pub body: String,
    /// The optional branded HTML part of an email-kind template
    /// (migration 0182), riding beside the text body the way
    /// `email_messages.body_html` rides beside `body_text`. `None` for
    /// every text-only template.
    pub html: Option<String>,
    pub variables: Value,
    pub builtin: Option<String>,
    pub is_system: bool,
    pub forked_from: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

fn entry_from_row(row: sqlx::postgres::PgRow) -> DbResult<TemplateEntry> {
    use sqlx::Row;
    Ok(TemplateEntry {
        id: row.get("id"),
        org_id: row.get("org_id"),
        kind: row.get("kind"),
        slug: row.get("slug"),
        name: row.get("name"),
        description: row.get("description"),
        subject: row.get("subject"),
        body: row.get("body"),
        html: row.get("html"),
        variables: row.get("variables"),
        builtin: row.get("builtin"),
        is_system: row.get("is_system"),
        forked_from: row.get("forked_from"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

const TEMPLATE_COLS: &str = "id, org_id, kind, slug, name, description, subject, body, html, \
     variables, builtin, is_system, forked_from, created_at, updated_at";

/// The org's templates, shipped defaults first (by builtin tag), then
/// org-owned rows by kind + name. `kind` filters to one template kind.
pub async fn list_templates(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
    kind: Option<&str>,
    limit: i64,
    offset: i64,
) -> DbResult<Vec<TemplateEntry>> {
    let sql = if kind.is_some() {
        format!(
            "SELECT {TEMPLATE_COLS} FROM templates WHERE org_id = $1 AND kind = $2 \
             ORDER BY (builtin IS NOT NULL) DESC, builtin, name LIMIT $3 OFFSET $4"
        )
    } else {
        format!(
            "SELECT {TEMPLATE_COLS} FROM templates WHERE org_id = $1 \
             ORDER BY (builtin IS NOT NULL) DESC, builtin, name LIMIT $2 OFFSET $3"
        )
    };
    let mut q = sqlx::query(&sql).bind(org_id);
    if let Some(k) = kind {
        q = q.bind(k);
    }
    let rows = q.bind(limit).bind(offset).fetch_all(executor).await?;
    rows.into_iter().map(entry_from_row).collect()
}

pub async fn get_template(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
    template_id: Uuid,
) -> DbResult<TemplateEntry> {
    let sql = format!("SELECT {TEMPLATE_COLS} FROM templates WHERE org_id = $1 AND id = $2");
    sqlx::query(&sql)
        .bind(org_id)
        .bind(template_id)
        .fetch_optional(executor)
        .await?
        .map(entry_from_row)
        .ok_or(DbError::NotFound)?
}

/// The one outbound lookup: the org's template row for a slug within one
/// kind. The enqueue path calls this once per system-sent mail on an
/// already-cold path — the `(org_id, kind, slug)` unique index is the
/// read's whole cost, and a missing row (`None`) is the builtin-fallback
/// signal, not an error: a template that does not exist must never fail
/// the mail that was on its way out.
pub async fn find_org_template(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
    kind: &str,
    slug: &str,
) -> DbResult<Option<TemplateEntry>> {
    let sql = format!(
        "SELECT {TEMPLATE_COLS} FROM templates WHERE org_id = $1 AND kind = $2 AND slug = $3"
    );
    sqlx::query(&sql)
        .bind(org_id)
        .bind(kind)
        .bind(slug)
        .fetch_optional(executor)
        .await?
        .map(entry_from_row)
        .transpose()
}

/// Inserts an org-owned row (create or fork). System rows are only ever
/// written by the migration — this always writes `is_system = false`.
/// `html` carries the branded part of an email-kind template; a fork
/// passes the origin's value so forking a branded template cannot drop
/// its HTML part on the floor.
#[allow(clippy::too_many_arguments)]
pub async fn insert_template(
    conn: &mut sqlx::PgConnection,
    id: Uuid,
    org_id: Uuid,
    kind: &str,
    slug: &str,
    name: &str,
    description: &str,
    subject: Option<&str>,
    body: &str,
    html: Option<&str>,
    variables: &Value,
    forked_from: Option<&str>,
) -> DbResult<TemplateEntry> {
    let sql = format!(
        "INSERT INTO templates (id, org_id, kind, slug, name, description, subject, body, html, \
         variables, builtin, is_system, forked_from) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NULL, false, $11) \
         RETURNING {TEMPLATE_COLS}"
    );
    let row = sqlx::query(&sql)
        .bind(id)
        .bind(org_id)
        .bind(kind)
        .bind(slug)
        .bind(name)
        .bind(description)
        .bind(subject)
        .bind(body)
        .bind(html)
        .bind(variables)
        .bind(forked_from)
        .fetch_one(&mut *conn)
        .await?;
    entry_from_row(row)
}

/// Updates an ORG-OWNED row's content. The `NOT is_system` guard makes a
/// shipped default immutable at the storage layer too — the API rejects
/// with a typed error first, the row guard is the second lock.
#[allow(clippy::too_many_arguments)]
pub async fn update_template(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    template_id: Uuid,
    name: &str,
    description: &str,
    subject: Option<&str>,
    body: &str,
    html: Option<&str>,
    variables: &Value,
) -> DbResult<TemplateEntry> {
    let sql = format!(
        "UPDATE templates SET name = $3, description = $4, subject = $5, body = $6, \
         html = $7, variables = $8, updated_at = now() \
         WHERE org_id = $1 AND id = $2 AND NOT is_system \
         RETURNING {TEMPLATE_COLS}"
    );
    sqlx::query(&sql)
        .bind(org_id)
        .bind(template_id)
        .bind(name)
        .bind(description)
        .bind(subject)
        .bind(body)
        .bind(html)
        .bind(variables)
        .fetch_optional(&mut *conn)
        .await?
        .map(entry_from_row)
        .ok_or(DbError::NotFound)?
}

/// Restores a FORKED PRESET row's content to the canonical shipped copy
/// (KB4: presets stay canonical, orgs fork). Only `is_system` rows with a
/// `builtin` tag qualify — org-created rows are never restorable.
pub async fn restore_default_template(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    template_id: Uuid,
    subject: Option<&str>,
    body: &str,
    html: Option<&str>,
) -> DbResult<TemplateEntry> {
    let sql = format!(
        "UPDATE templates SET subject = $3, body = $4, html = $5, variables = '[]'::jsonb, \
         updated_at = now() \
         WHERE org_id = $1 AND id = $2 AND is_system AND builtin IS NOT NULL \
         RETURNING {TEMPLATE_COLS}"
    );
    sqlx::query(&sql)
        .bind(org_id)
        .bind(template_id)
        .bind(subject)
        .bind(body)
        .bind(html)
        .fetch_optional(&mut *conn)
        .await?
        .map(entry_from_row)
        .ok_or(DbError::NotFound)?
}

/// Deletes an org-owned row. The `NOT is_system` guard keeps shipped
/// defaults undeletable (the API surfaces a typed 409 first).
pub async fn delete_template(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    template_id: Uuid,
) -> DbResult<TemplateEntry> {
    let sql = format!(
        "DELETE FROM templates WHERE org_id = $1 AND id = $2 AND NOT is_system \
         RETURNING {TEMPLATE_COLS}"
    );
    sqlx::query(&sql)
        .bind(org_id)
        .bind(template_id)
        .fetch_optional(&mut *conn)
        .await?
        .map(entry_from_row)
        .ok_or(DbError::NotFound)?
}

// --- shipped defaults -----------------------------------------------------------

/// Seeds the shipped default templates for a NEW org at registration
/// (migration 0070 backfills pre-lane orgs; this covers orgs created
/// after it). Content comes from `core_domain::templates::
/// DEFAULT_TEMPLATES` — the canonical copy, so a fresh org's seeds and a
/// restore always agree by construction.
pub async fn seed_default_templates(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
) -> DbResult<usize> {
    use core_domain::templates::DEFAULT_TEMPLATES;
    let mut seeded = 0;
    for d in DEFAULT_TEMPLATES {
        let exists: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM templates WHERE org_id = $1 AND kind = $2 AND slug = $3 LIMIT 1",
        )
        .bind(org_id)
        .bind(d.kind.as_str())
        .bind(d.slug)
        .fetch_optional(&mut *conn)
        .await?;
        if exists.is_none() {
            insert_template(
                conn,
                Uuid::now_v7(),
                org_id,
                d.kind.as_str(),
                d.slug,
                d.name,
                d.description,
                d.subject,
                d.body,
                d.html,
                &serde_json::json!([]),
                None,
            )
            .await?;
            // Mark the row as the shipped system row (insert_template always
            // writes org-owned rows; the default IS the system row).
            sqlx::query("UPDATE templates SET builtin = $3, is_system = true WHERE org_id = $1 AND kind = $2 AND slug = $3")
                .bind(org_id)
                .bind(d.kind.as_str())
                .bind(d.slug)
                .execute(&mut *conn)
                .await?;
        }
        seeded += 1;
    }
    Ok(seeded)
}

/// Seeds the system-outbound email templates for a NEW org at
/// registration (migration 0182 backfills pre-lane orgs; this covers
/// orgs created after it). These seed as EDITABLE ORG ROWS on purpose —
/// the enqueue lookup reads the org's row by the kind's fixed slug, so
/// forking cannot rewire it and an immutable system row would make the
/// builtin fallback unreachable. Content comes from
/// `core_domain::templates::OUTBOUND_EMAIL_TEMPLATES`, the canonical
/// copy; the drift test pins a fresh seed byte-identical to it.
pub async fn seed_outbound_email_templates(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
) -> DbResult<usize> {
    use core_domain::templates::OUTBOUND_EMAIL_TEMPLATES;
    let mut seeded = 0;
    for d in OUTBOUND_EMAIL_TEMPLATES {
        let exists: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM templates WHERE org_id = $1 AND kind = $2 AND slug = $3 LIMIT 1",
        )
        .bind(org_id)
        .bind(d.kind.as_str())
        .bind(d.slug)
        .fetch_optional(&mut *conn)
        .await?;
        if exists.is_none() {
            insert_template(
                conn,
                Uuid::now_v7(),
                org_id,
                d.kind.as_str(),
                d.slug,
                d.name,
                d.description,
                d.subject,
                d.body,
                d.html,
                &serde_json::json!([]),
                None,
            )
            .await?;
        }
        seeded += 1;
    }
    Ok(seeded)
}

// --- render contexts ----------------------------------------------------------

/// Org-flavor context (kb kind): just the org + rendering user.
pub async fn org_render_context(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
    user_id: Uuid,
) -> DbResult<TemplateContextRow> {
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT o.name, u.display_name FROM orgs o \
         LEFT JOIN users u ON u.id = $2 WHERE o.id = $1",
    )
    .bind(org_id)
    .bind(user_id)
    .fetch_optional(executor)
    .await?;
    row.map(|(org_name, user_name)| TemplateContextRow {
        org_name: Some(org_name),
        user_name,
        ..Default::default()
    })
    .ok_or(DbError::NotFound)
}

/// The joined entity snapshot behind one render (ticket, project, or quote
/// flavor; absent joins resolve `None` and render empty for permitted keys).
#[derive(Debug, Clone, Default)]
pub struct TemplateContextRow {
    pub org_name: Option<String>,
    pub user_name: Option<String>,
    pub client_name: Option<String>,
    pub contact_name: Option<String>,
    pub contact_email: Option<String>,
    pub contact_phone: Option<String>,
    pub ticket_number: Option<String>,
    pub ticket_summary: Option<String>,
    pub ticket_priority: Option<String>,
    pub ticket_status: Option<String>,
    pub assignee_name: Option<String>,
    pub quote_number: Option<String>,
    pub quote_status: Option<String>,
    pub quote_total: Option<String>,
    pub quote_currency: Option<String>,
    pub project_name: Option<String>,
    pub project_status: Option<String>,
}

impl TemplateContextRow {
    /// Flattens to the `path -> display string` map the renderer consumes
    /// (`None` values are omitted — permitted keys render empty).
    pub fn to_map(&self) -> std::collections::BTreeMap<String, String> {
        use core_domain::templates::RenderContext;
        let mut m = RenderContext::new();
        for (k, v) in [
            ("org.name", &self.org_name),
            ("user.name", &self.user_name),
            ("client.name", &self.client_name),
            ("contact.name", &self.contact_name),
            ("contact.email", &self.contact_email),
            ("contact.phone", &self.contact_phone),
            ("ticket.number", &self.ticket_number),
            ("ticket.summary", &self.ticket_summary),
            ("ticket.priority", &self.ticket_priority),
            ("ticket.status", &self.ticket_status),
            ("assignee.name", &self.assignee_name),
            ("quote.number", &self.quote_number),
            ("quote.status", &self.quote_status),
            ("quote.total", &self.quote_total),
            ("quote.currency", &self.quote_currency),
            ("project.name", &self.project_name),
            ("project.status", &self.project_status),
        ] {
            if let Some(v) = v {
                m.insert(k.to_string(), v.clone());
            }
        }
        m
    }
}

/// Raw ticket-context row: org/user/client/contact (3) + ticket (4) + assignee.
type TicketContextRow = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Raw project-context row: org + user + client + project name/status.
type ProjectContextRow = (String, Option<String>, Option<String>, String, String);

/// Raw quote-context row: org/user/client/contact (3) + quote (4).
type QuoteContextRow = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    String,
    Option<i64>,
    String,
);

/// Ticket-flavor context: ticket + client + contact + assignee + org +
/// the rendering user (`user_id` = the actor, `user.name` in templates).
pub async fn ticket_render_context(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
    ticket_id: Uuid,
    user_id: Uuid,
) -> DbResult<TemplateContextRow> {
    let row: Option<TicketContextRow> = sqlx::query_as(
        "SELECT o.name, u.display_name, c.name, ct.name, ct.email, ct.phone, \
                t.number::text, t.title, t.priority::text, st.name, a.display_name \
         FROM tickets t \
         JOIN orgs o ON o.id = t.org_id \
         LEFT JOIN users u ON u.id = $3 \
         LEFT JOIN clients c ON c.id = t.client_id \
         LEFT JOIN contacts ct ON ct.id = t.contact_id \
         LEFT JOIN board_statuses st ON st.id = t.status_id \
         LEFT JOIN users a ON a.id = t.assignee_id \
         WHERE t.org_id = $1 AND t.id = $2",
    )
    .bind(org_id)
    .bind(ticket_id)
    .bind(user_id)
    .fetch_optional(executor)
    .await?;
    row.map(
        |(
            org_name,
            user_name,
            client_name,
            contact_name,
            contact_email,
            contact_phone,
            ticket_number,
            ticket_summary,
            ticket_priority,
            ticket_status,
            assignee_name,
        )| TemplateContextRow {
            org_name,
            user_name,
            client_name,
            contact_name,
            contact_email,
            contact_phone,
            ticket_number,
            ticket_summary,
            ticket_priority,
            ticket_status,
            assignee_name,
            ..Default::default()
        },
    )
    .ok_or(DbError::NotFound)
}

/// Project-flavor context: project + client + org + the rendering user.
pub async fn project_render_context(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
    project_id: Uuid,
    user_id: Uuid,
) -> DbResult<TemplateContextRow> {
    let row: Option<ProjectContextRow> = sqlx::query_as(
        "SELECT o.name, u.display_name, c.name, p.name, p.status::text \
         FROM projects p \
         JOIN orgs o ON o.id = p.org_id \
         LEFT JOIN users u ON u.id = $3 \
         LEFT JOIN clients c ON c.id = p.client_id \
         WHERE p.org_id = $1 AND p.id = $2",
    )
    .bind(org_id)
    .bind(project_id)
    .bind(user_id)
    .fetch_optional(executor)
    .await?;
    row.map(
        |(org_name, user_name, client_name, project_name, project_status)| TemplateContextRow {
            org_name: Some(org_name),
            user_name,
            client_name,
            project_name: Some(project_name),
            project_status: Some(project_status),
            ..Default::default()
        },
    )
    .ok_or(DbError::NotFound)
}

/// Quote-flavor context (rides Wave 12 lane 2's quote catalog): quote +
/// client + org, total formatted through `Money` (currency exponent aware).
pub async fn quote_render_context(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
    quote_id: Uuid,
    user_id: Uuid,
) -> DbResult<TemplateContextRow> {
    let row: Option<QuoteContextRow> = sqlx::query_as(
        "SELECT o.name, u.display_name, c.name, ct.name, ct.email, ct.phone, \
                q.number, q.status::text, q.total_minor, q.currency \
         FROM quotes q \
         JOIN orgs o ON o.id = q.org_id \
         LEFT JOIN users u ON u.id = $3 \
         LEFT JOIN clients c ON c.id = q.client_id \
         LEFT JOIN LATERAL ( \
           SELECT ct.name, ct.email, ct.phone FROM contacts ct \
           WHERE ct.client_id = q.client_id AND ct.is_primary LIMIT 1 \
         ) ct ON true \
         WHERE q.org_id = $1 AND q.id = $2",
    )
    .bind(org_id)
    .bind(quote_id)
    .bind(user_id)
    .fetch_optional(executor)
    .await?;
    row.map(
        |(
            org_name,
            user_name,
            client_name,
            contact_name,
            contact_email,
            contact_phone,
            number,
            status,
            total_minor,
            currency,
        )| {
            let quote_total = total_minor
                .as_ref()
                .zip(Some(&currency))
                .and_then(|(minor, cur)| {
                    core_domain::money::CurrencyCode::new(cur)
                        .ok()
                        .map(|c| core_domain::money::Money::new(*minor, c).to_string())
                });
            TemplateContextRow {
                org_name: Some(org_name),
                user_name,
                client_name,
                contact_name,
                contact_email,
                contact_phone,
                quote_number: Some(number),
                quote_status: Some(status),
                quote_total,
                quote_currency: Some(currency),
                ..Default::default()
            }
        },
    )
    .ok_or(DbError::NotFound)
}
