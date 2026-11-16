//! Project delivery persistence: change orders, templates, portfolio reads,
//! pods, client updates and the project-to-ticket link.
//!
//! Query functions live here so the route module stays about HTTP and policy.
//!
//! Contract notes:
//! * Every function is org-scoped; rows are RLS-filtered in transactions bound
//!   via `core_db::set_org_and_user`.
//! * State transitions are guarded in SQL (`WHERE status = ...`) so two
//!   approvers cannot both win, and a refusal is `DbError::Conflict` rather
//!   than a silent overwrite.
//! * Money is a pair: an amount without a currency is refused by the column
//!   CHECK and by the wire validation, in that order.

use crate::{DbError, DbResult};
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;
use sqlx::{Postgres, Row};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Shared decoding helpers
// ---------------------------------------------------------------------------

fn json(value: &Value) -> Value {
    value.clone()
}

// ---------------------------------------------------------------------------
// Template gallery
// ---------------------------------------------------------------------------

/// A template as the gallery reads it: the document plus the catalogue facts
/// the card draws. `current_version` is the published version an instantiation
/// would read, which is what the "newer shipped version" notice compares.
#[derive(Debug, Clone)]
pub struct GalleryTemplate {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub spec: Value,
    pub category: Option<String>,
    pub typical_days: Option<i32>,
    pub effort_min_minutes: Option<i32>,
    pub effort_max_minutes: Option<i32>,
    pub archived_at: Option<DateTime<Utc>>,
    pub is_shipped: bool,
    pub shipped_key: Option<String>,
    pub forked_from_version_id: Option<Uuid>,
    pub current_version: Option<i32>,
    pub current_version_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn row_to_gallery(row: sqlx::postgres::PgRow) -> DbResult<GalleryTemplate> {
    Ok(GalleryTemplate {
        id: row.get("id"),
        org_id: row.get("org_id"),
        name: row.get("name"),
        description: row.get("description"),
        spec: row.get("spec"),
        category: row.get("category"),
        typical_days: row.get("typical_days"),
        effort_min_minutes: row.get("effort_min_minutes"),
        effort_max_minutes: row.get("effort_max_minutes"),
        archived_at: row.get("archived_at"),
        is_shipped: row.get("is_shipped"),
        shipped_key: row.get("shipped_key"),
        forked_from_version_id: row.get("forked_from_version_id"),
        current_version: row.get("current_version"),
        current_version_id: row.get("current_version_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

const GALLERY_SELECT: &str = r#"
    SELECT t.id, t.org_id, t.name, t.description, t.spec, t.category,
           t.typical_days, t.effort_min_minutes, t.effort_max_minutes,
           t.archived_at, t.is_shipped, t.shipped_key, t.forked_from_version_id,
           t.created_at, t.updated_at,
           v.version AS current_version, v.id AS current_version_id
      FROM project_templates t
      LEFT JOIN project_template_versions v ON v.id = t.current_version_id
"#;

/// The gallery: org templates and shipped templates in one list. Archived rows
/// are included only when asked for, because hiding them by default is the
/// decision S11.6 draws.
pub async fn list_template_gallery<'e, E>(
    executor: E,
    org_id: Uuid,
    include_archived: bool,
    category: Option<&str>,
) -> DbResult<Vec<GalleryTemplate>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let mut sql = String::from(GALLERY_SELECT);
    sql.push_str(" WHERE t.org_id = $1");
    if !include_archived {
        sql.push_str(" AND t.archived_at IS NULL");
    }
    if category.is_some() {
        sql.push_str(" AND t.category = $3");
    }
    sql.push_str(" ORDER BY t.is_shipped DESC, t.archived_at NULLS FIRST, t.name");

    let mut query = sqlx::query(&sql).bind(org_id);
    if let Some(category) = category {
        query = query.bind(category);
    }
    let rows = query.fetch_all(executor).await?;
    rows.into_iter().map(row_to_gallery).collect()
}

/// Reads one gallery row (used after a mutation so the card is current).
pub async fn get_gallery_template<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<GalleryTemplate>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("{GALLERY_SELECT} WHERE t.org_id = $1 AND t.id = $2");
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    row_to_gallery(row)
}

/// Archives or restores a template. Archived templates cannot instantiate;
/// they still explain the projects that were created from them.
pub async fn set_template_archived(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    archived: bool,
) -> DbResult<()> {
    let res = sqlx::query(
        r#"UPDATE project_templates
              SET archived_at = CASE WHEN $3 THEN COALESCE(archived_at, now()) ELSE NULL END,
                  updated_at = now()
            WHERE org_id = $1 AND id = $2"#,
    )
    .bind(org_id)
    .bind(id)
    .bind(archived)
    .execute(&mut *conn)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// The catalogue facts a fork or author sets alongside the document. Kept in
/// its own write so a spec edit cannot silently rewrite the card.
pub struct TemplateMeta<'a> {
    pub category: Option<&'a str>,
    pub typical_days: Option<i32>,
    pub effort_min_minutes: Option<i32>,
    pub effort_max_minutes: Option<i32>,
}

pub async fn set_template_meta(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    meta: TemplateMeta<'_>,
) -> DbResult<()> {
    let res = sqlx::query(
        r#"UPDATE project_templates
              SET category = $3, typical_days = $4,
                  effort_min_minutes = $5, effort_max_minutes = $6,
                  updated_at = now()
            WHERE org_id = $1 AND id = $2"#,
    )
    .bind(org_id)
    .bind(id)
    .bind(meta.category)
    .bind(meta.typical_days)
    .bind(meta.effort_min_minutes)
    .bind(meta.effort_max_minutes)
    .execute(&mut *conn)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Template lessons
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TemplateLesson {
    pub id: Uuid,
    pub org_id: Uuid,
    pub template_id: Uuid,
    pub project_id: Option<Uuid>,
    pub kind: String,
    pub message: String,
    pub evidence: Value,
    pub state: String,
    pub dismiss_reason: Option<String>,
    pub decided_by: Option<Uuid>,
    pub decided_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

fn row_to_lesson(row: sqlx::postgres::PgRow) -> TemplateLesson {
    TemplateLesson {
        id: row.get("id"),
        org_id: row.get("org_id"),
        template_id: row.get("template_id"),
        project_id: row.get("project_id"),
        kind: row.get("kind"),
        message: row.get("message"),
        evidence: row.get("evidence"),
        state: row.get("state"),
        dismiss_reason: row.get("dismiss_reason"),
        decided_by: row.get("decided_by"),
        decided_at: row.get("decided_at"),
        created_at: row.get("created_at"),
    }
}

const LESSON_COLS: &str = "id, org_id, template_id, project_id, kind, message, evidence, \
     state, dismiss_reason, decided_by, decided_at, created_at";

pub async fn list_template_lessons<'e, E>(
    executor: E,
    org_id: Uuid,
    template_id: Uuid,
) -> DbResult<Vec<TemplateLesson>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {LESSON_COLS} FROM project_template_lessons
          WHERE org_id = $1 AND template_id = $2
          ORDER BY created_at DESC"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(template_id)
        .fetch_all(executor)
        .await?;
    Ok(rows.into_iter().map(row_to_lesson).collect())
}

/// Records one lesson from a closed project. Deduplicated per template and
/// message: the same observation from the same project is one lesson.
pub struct NewLesson<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub template_id: Uuid,
    pub project_id: Option<Uuid>,
    pub kind: &'a str,
    pub message: &'a str,
    pub evidence: &'a Value,
}

pub async fn insert_lesson(
    conn: &mut sqlx::PgConnection,
    lesson: NewLesson<'_>,
) -> DbResult<TemplateLesson> {
    let sql = format!(
        r#"INSERT INTO project_template_lessons
               (id, org_id, template_id, project_id, kind, message, evidence)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING {LESSON_COLS}"#
    );
    let row = sqlx::query(&sql)
        .bind(lesson.id)
        .bind(lesson.org_id)
        .bind(lesson.template_id)
        .bind(lesson.project_id)
        .bind(lesson.kind)
        .bind(lesson.message)
        .bind(lesson.evidence)
        .fetch_one(&mut *conn)
        .await?;
    Ok(row_to_lesson(row))
}

/// Accepts or dismisses a suggestion. A dismissal must say why (the pack's
/// loop remembers the reason so the same advice is not repeated); an open
/// lesson cannot be re-decided without a new row.
pub async fn decide_lesson(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    template_id: Uuid,
    lesson_id: Uuid,
    state: &str,
    reason: Option<&str>,
    decided_by: Uuid,
) -> DbResult<TemplateLesson> {
    let sql = format!(
        r#"UPDATE project_template_lessons
              SET state = $4,
                  dismiss_reason = CASE WHEN $4 = 'dismissed' THEN $5 ELSE NULL END,
                  decided_by = $6,
                  decided_at = now()
            WHERE org_id = $1 AND template_id = $2 AND id = $3 AND state = 'open'
            RETURNING {LESSON_COLS}"#
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(template_id)
        .bind(lesson_id)
        .bind(state)
        .bind(reason)
        .bind(decided_by)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(DbError::Conflict(
            "the lesson is already decided or not in this template".into(),
        ))?;
    Ok(row_to_lesson(row))
}

// ---------------------------------------------------------------------------
// Change orders
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ChangeOrder {
    pub id: Uuid,
    pub org_id: Uuid,
    pub project_id: Uuid,
    pub reference: String,
    pub title: String,
    pub status: String,
    pub source: String,
    pub reason: String,
    pub scope_changes: Value,
    pub client_summary: Option<String>,
    pub evidence: Value,
    pub coverage_note: Option<String>,
    pub confidence: Option<String>,
    pub contract_clause: Option<String>,
    pub amount_minor: Option<i64>,
    pub currency: Option<String>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub submitted_by: Option<Uuid>,
    pub decided_at: Option<DateTime<Utc>>,
    pub decided_by: Option<Uuid>,
    pub decision_note: Option<String>,
    pub applied_at: Option<DateTime<Utc>>,
    pub applied_by: Option<Uuid>,
    pub applied_baseline_id: Option<Uuid>,
    pub billed_at: Option<DateTime<Utc>>,
    pub billing_release_id: Option<Uuid>,
    pub billing_error: Option<String>,
    pub client_ack_required: bool,
    pub client_ack_at: Option<DateTime<Utc>>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const CHANGE_ORDER_COLS: &str = "id, org_id, project_id, reference, title, status, source, \
     reason, scope_changes, client_summary, evidence, coverage_note, confidence, \
     contract_clause, amount_minor, currency, submitted_at, submitted_by, decided_at, \
     decided_by, decision_note, applied_at, applied_by, applied_baseline_id, billed_at, \
     billing_release_id, billing_error, client_ack_required, client_ack_at, created_by, \
     created_at, updated_at";

fn row_to_change_order(row: sqlx::postgres::PgRow) -> ChangeOrder {
    ChangeOrder {
        id: row.get("id"),
        org_id: row.get("org_id"),
        project_id: row.get("project_id"),
        reference: row.get("reference"),
        title: row.get("title"),
        status: row.get("status"),
        source: row.get("source"),
        reason: row.get("reason"),
        scope_changes: row.get("scope_changes"),
        client_summary: row.get("client_summary"),
        evidence: row.get("evidence"),
        coverage_note: row.get("coverage_note"),
        confidence: row.get("confidence"),
        contract_clause: row.get("contract_clause"),
        amount_minor: row.get("amount_minor"),
        currency: row.get("currency"),
        submitted_at: row.get("submitted_at"),
        submitted_by: row.get("submitted_by"),
        decided_at: row.get("decided_at"),
        decided_by: row.get("decided_by"),
        decision_note: row.get("decision_note"),
        applied_at: row.get("applied_at"),
        applied_by: row.get("applied_by"),
        applied_baseline_id: row.get("applied_baseline_id"),
        billed_at: row.get("billed_at"),
        billing_release_id: row.get("billing_release_id"),
        billing_error: row.get("billing_error"),
        client_ack_required: row.get("client_ack_required"),
        client_ack_at: row.get("client_ack_at"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

pub struct NewChangeOrder<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub project_id: Uuid,
    pub reference: &'a str,
    pub title: &'a str,
    pub source: &'a str,
    pub reason: &'a str,
    pub scope_changes: &'a Value,
    pub client_summary: Option<&'a str>,
    pub evidence: &'a Value,
    pub coverage_note: Option<&'a str>,
    pub confidence: Option<&'a str>,
    pub contract_clause: Option<&'a str>,
    pub amount_minor: Option<i64>,
    pub currency: Option<&'a str>,
    pub client_ack_required: bool,
    pub created_by: Option<Uuid>,
}

pub async fn insert_change_order(
    conn: &mut sqlx::PgConnection,
    co: NewChangeOrder<'_>,
) -> DbResult<ChangeOrder> {
    let sql = format!(
        r#"INSERT INTO project_change_orders
              (id, org_id, project_id, reference, title, source, reason, scope_changes,
               client_summary, evidence, coverage_note, confidence, contract_clause,
               amount_minor, currency, client_ack_required, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15,
                   $16, $17)
           RETURNING {CHANGE_ORDER_COLS}"#
    );
    let row = sqlx::query(&sql)
        .bind(co.id)
        .bind(co.org_id)
        .bind(co.project_id)
        .bind(co.reference)
        .bind(co.title)
        .bind(co.source)
        .bind(co.reason)
        .bind(json(co.scope_changes))
        .bind(co.client_summary)
        .bind(json(co.evidence))
        .bind(co.coverage_note)
        .bind(co.confidence)
        .bind(co.contract_clause)
        .bind(co.amount_minor)
        .bind(co.currency)
        .bind(co.client_ack_required)
        .bind(co.created_by)
        .fetch_one(&mut *conn)
        .await?;
    Ok(row_to_change_order(row))
}

pub async fn get_change_order<'e, E>(
    executor: E,
    org_id: Uuid,
    project_id: Uuid,
    id: Uuid,
) -> DbResult<ChangeOrder>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {CHANGE_ORDER_COLS} FROM project_change_orders
          WHERE org_id = $1 AND project_id = $2 AND id = $3"
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(project_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_change_order(row))
}

pub async fn list_change_orders<'e, E>(
    executor: E,
    org_id: Uuid,
    project_id: Uuid,
) -> DbResult<Vec<ChangeOrder>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {CHANGE_ORDER_COLS} FROM project_change_orders
          WHERE org_id = $1 AND project_id = $2
          ORDER BY created_at DESC"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(project_id)
        .fetch_all(executor)
        .await?;
    Ok(rows.into_iter().map(row_to_change_order).collect())
}

/// Edits a draft only. A submitted order is read-only for its author
/// (S13.5), so the guard is in SQL, not in a UI affordance.
pub struct ChangeOrderPatch<'a> {
    pub title: Option<&'a str>,
    pub reason: Option<&'a str>,
    pub scope_changes: Option<&'a Value>,
    pub client_summary: Option<Option<&'a str>>,
    pub evidence: Option<&'a Value>,
    pub coverage_note: Option<Option<&'a str>>,
    pub confidence: Option<Option<&'a str>>,
    pub contract_clause: Option<Option<&'a str>>,
    pub amount_minor: Option<Option<i64>>,
    pub currency: Option<Option<&'a str>>,
    pub client_ack_required: Option<bool>,
}

pub async fn patch_change_order(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    project_id: Uuid,
    id: Uuid,
    p: ChangeOrderPatch<'_>,
) -> DbResult<ChangeOrder> {
    let mut q = sqlx::QueryBuilder::new("UPDATE project_change_orders SET updated_at = now()");
    if let Some(v) = p.title {
        q.push(", title = ").push_bind(v);
    }
    if let Some(v) = p.reason {
        q.push(", reason = ").push_bind(v);
    }
    if let Some(v) = p.scope_changes {
        q.push(", scope_changes = ").push_bind(json(v));
    }
    if let Some(v) = p.client_summary {
        q.push(", client_summary = ").push_bind(v);
    }
    if let Some(v) = p.evidence {
        q.push(", evidence = ").push_bind(json(v));
    }
    if let Some(v) = p.coverage_note {
        q.push(", coverage_note = ").push_bind(v);
    }
    if let Some(v) = p.confidence {
        q.push(", confidence = ").push_bind(v);
    }
    if let Some(v) = p.contract_clause {
        q.push(", contract_clause = ").push_bind(v);
    }
    if let Some(v) = p.amount_minor {
        q.push(", amount_minor = ").push_bind(v);
    }
    if let Some(v) = p.currency {
        q.push(", currency = ").push_bind(v);
    }
    if let Some(v) = p.client_ack_required {
        q.push(", client_ack_required = ").push_bind(v);
    }
    q.push(" WHERE org_id = ")
        .push_bind(org_id)
        .push(" AND project_id = ")
        .push_bind(project_id)
        .push(" AND id = ")
        .push_bind(id)
        .push(" AND status = 'draft' RETURNING ")
        .push(CHANGE_ORDER_COLS);
    let row = q
        .build()
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(DbError::Conflict(
            "only a draft change order can be edited".into(),
        ))?;
    Ok(row_to_change_order(row))
}

/// The guarded transition. `from` names every status this move may start
/// from; the row is updated only when it is currently in one of them.
#[allow(clippy::too_many_arguments)]
pub async fn transition_change_order(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    project_id: Uuid,
    id: Uuid,
    from: &[&str],
    to: &str,
    actor: Option<Uuid>,
    note: Option<&str>,
    extra_sql: &str,
) -> DbResult<ChangeOrder> {
    let mut q = sqlx::QueryBuilder::new("UPDATE project_change_orders SET updated_at = now()");
    q.push(", status = ").push_bind(to.to_string());

    // The actor column depends on the move: submitted_by, decided_by,
    // applied_by are different facts and are written by different steps.
    match to {
        "submitted" => {
            q.push(", submitted_at = now(), submitted_by = ")
                .push_bind(actor);
        }
        "approved" | "rejected" => {
            q.push(", decided_at = now(), decided_by = ")
                .push_bind(actor);
        }
        "applied" => {
            q.push(", applied_at = now(), applied_by = ")
                .push_bind(actor);
        }
        _ => {}
    }
    if let Some(note) = note {
        if to == "rejected" || to == "approved" {
            q.push(", decision_note = ").push_bind(note.to_string());
        }
    }
    if !extra_sql.is_empty() {
        q.push(extra_sql);
    }

    q.push(" WHERE org_id = ")
        .push_bind(org_id)
        .push(" AND project_id = ")
        .push_bind(project_id)
        .push(" AND id = ")
        .push_bind(id)
        .push(" AND status = ANY(")
        .push_bind(from.iter().map(|s| s.to_string()).collect::<Vec<_>>())
        .push(") RETURNING ")
        .push(CHANGE_ORDER_COLS);

    let row = q
        .build()
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(DbError::Conflict(
            "the change order is not in a state this step can start from".into(),
        ))?;
    Ok(row_to_change_order(row))
}

// ---------------------------------------------------------------------------
// Client updates
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ClientUpdate {
    pub id: Uuid,
    pub org_id: Uuid,
    pub project_id: Uuid,
    pub status: String,
    pub body: String,
    pub facts: Value,
    pub period_start: Option<NaiveDate>,
    pub period_end: Option<NaiveDate>,
    pub withheld_reason: Option<String>,
    pub approved_by: Option<Uuid>,
    pub approved_at: Option<DateTime<Utc>>,
    pub sent_at: Option<DateTime<Utc>>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const CLIENT_UPDATE_COLS: &str = "id, org_id, project_id, status, body, facts, \
     period_start, period_end, withheld_reason, approved_by, approved_at, sent_at, \
     created_by, created_at, updated_at";

fn row_to_client_update(row: sqlx::postgres::PgRow) -> ClientUpdate {
    ClientUpdate {
        id: row.get("id"),
        org_id: row.get("org_id"),
        project_id: row.get("project_id"),
        status: row.get("status"),
        body: row.get("body"),
        facts: row.get("facts"),
        period_start: row.get("period_start"),
        period_end: row.get("period_end"),
        withheld_reason: row.get("withheld_reason"),
        approved_by: row.get("approved_by"),
        approved_at: row.get("approved_at"),
        sent_at: row.get("sent_at"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

pub struct NewClientUpdate<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub project_id: Uuid,
    pub status: &'a str,
    pub body: &'a str,
    pub facts: &'a Value,
    pub period_start: Option<NaiveDate>,
    pub period_end: Option<NaiveDate>,
    pub withheld_reason: Option<&'a str>,
    pub created_by: Option<Uuid>,
}

pub async fn insert_client_update(
    conn: &mut sqlx::PgConnection,
    update: NewClientUpdate<'_>,
) -> DbResult<ClientUpdate> {
    let sql = format!(
        r#"INSERT INTO project_client_updates
              (id, org_id, project_id, status, body, facts, period_start, period_end,
               withheld_reason, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           RETURNING {CLIENT_UPDATE_COLS}"#
    );
    let row = sqlx::query(&sql)
        .bind(update.id)
        .bind(update.org_id)
        .bind(update.project_id)
        .bind(update.status)
        .bind(update.body)
        .bind(json(update.facts))
        .bind(update.period_start)
        .bind(update.period_end)
        .bind(update.withheld_reason)
        .bind(update.created_by)
        .fetch_one(&mut *conn)
        .await?;
    Ok(row_to_client_update(row))
}

pub async fn get_client_update<'e, E>(
    executor: E,
    org_id: Uuid,
    project_id: Uuid,
    id: Uuid,
) -> DbResult<ClientUpdate>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {CLIENT_UPDATE_COLS} FROM project_client_updates
          WHERE org_id = $1 AND project_id = $2 AND id = $3"
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(project_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_client_update(row))
}

pub async fn list_client_updates<'e, E>(
    executor: E,
    org_id: Uuid,
    project_id: Uuid,
) -> DbResult<Vec<ClientUpdate>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {CLIENT_UPDATE_COLS} FROM project_client_updates
          WHERE org_id = $1 AND project_id = $2
          ORDER BY created_at DESC"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(project_id)
        .fetch_all(executor)
        .await?;
    Ok(rows.into_iter().map(row_to_client_update).collect())
}

/// Edits the text of a draft. An approved update is frozen: the client sees
/// what the person approved, so editing after approval would send a different
/// document than the one reviewed.
pub async fn update_client_update_body(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    project_id: Uuid,
    id: Uuid,
    body: &str,
) -> DbResult<ClientUpdate> {
    let sql = format!(
        r#"UPDATE project_client_updates
              SET body = $4, updated_at = now()
            WHERE org_id = $1 AND project_id = $2 AND id = $3 AND status = 'draft'
            RETURNING {CLIENT_UPDATE_COLS}"#
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(project_id)
        .bind(id)
        .bind(body)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(DbError::Conflict(
            "only a draft update can be edited".into(),
        ))?;
    Ok(row_to_client_update(row))
}

/// Approves a draft: from here the text is fixed and only a send may follow.
pub async fn approve_client_update(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    project_id: Uuid,
    id: Uuid,
    approved_by: Uuid,
) -> DbResult<ClientUpdate> {
    let sql = format!(
        r#"UPDATE project_client_updates
              SET status = 'approved', approved_by = $4, approved_at = now(),
                  updated_at = now()
            WHERE org_id = $1 AND project_id = $2 AND id = $3 AND status = 'draft'
            RETURNING {CLIENT_UPDATE_COLS}"#
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(project_id)
        .bind(id)
        .bind(approved_by)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(DbError::Conflict("only a draft can be approved".into()))?;
    Ok(row_to_client_update(row))
}

/// Withholds a draft, keeping the reason so the next draft does not invent
/// movement.
pub async fn withhold_client_update(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    project_id: Uuid,
    id: Uuid,
    reason: &str,
) -> DbResult<ClientUpdate> {
    let sql = format!(
        r#"UPDATE project_client_updates
              SET status = 'withheld', withheld_reason = $4, updated_at = now()
            WHERE org_id = $1 AND project_id = $2 AND id = $3 AND status = 'draft'
            RETURNING {CLIENT_UPDATE_COLS}"#
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(project_id)
        .bind(id)
        .bind(reason)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(DbError::Conflict("only a draft can be withheld".into()))?;
    Ok(row_to_client_update(row))
}

/// Sends exactly the approved text, once. The guard is the status, so a double
/// click cannot email twice and a draft cannot be sent without review.
pub async fn send_client_update(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    project_id: Uuid,
    id: Uuid,
) -> DbResult<ClientUpdate> {
    let sql = format!(
        r#"UPDATE project_client_updates
              SET status = 'sent', sent_at = now(), updated_at = now()
            WHERE org_id = $1 AND project_id = $2 AND id = $3 AND status = 'approved'
            RETURNING {CLIENT_UPDATE_COLS}"#
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(project_id)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(DbError::Conflict(
            "only an approved update can be sent".into(),
        ))?;
    Ok(row_to_client_update(row))
}

/// Records the client's view of a change order in the portal.
pub async fn acknowledge_change_order(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    project_id: Uuid,
    id: Uuid,
    by: Option<Uuid>,
) -> DbResult<ChangeOrder> {
    let sql = format!(
        r#"UPDATE project_change_orders
              SET client_ack_at = COALESCE(client_ack_at, now()),
                  client_ack_by = COALESCE(client_ack_by, $4),
                  status = CASE WHEN status = 'awaiting_client' THEN 'approved' ELSE status END,
                  updated_at = now()
            WHERE org_id = $1 AND project_id = $2 AND id = $3
              AND client_ack_required
              AND status IN ('approved','awaiting_client')
            RETURNING {CHANGE_ORDER_COLS}"#
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(project_id)
        .bind(id)
        .bind(by)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(DbError::Conflict(
            "this change order is not waiting for the client".into(),
        ))?;
    Ok(row_to_change_order(row))
}

// ---------------------------------------------------------------------------
// Blockers and pods
// ---------------------------------------------------------------------------

pub struct TaskBlocker<'a> {
    pub kind: &'a str,
    pub note: Option<&'a str>,
    pub owner_id: Option<Uuid>,
}

/// Sets a typed blocker on a task and moves it to `blocked` in one statement:
/// a blocker that leaves the task `pending` is two facts that disagree.
pub async fn set_task_blocker(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    project_id: Uuid,
    task_id: Uuid,
    blocker: TaskBlocker<'_>,
) -> DbResult<bool> {
    let res = sqlx::query(
        r#"UPDATE project_tasks
              SET blocker_type = $4,
                  blocker_note = $5,
                  blocker_owner_id = $6,
                  blocked_at = now(),
                  status = 'blocked',
                  updated_at = now()
            WHERE org_id = $1 AND project_id = $2 AND id = $3
              AND status NOT IN ('completed','cancelled')"#,
    )
    .bind(org_id)
    .bind(project_id)
    .bind(task_id)
    .bind(blocker.kind)
    .bind(blocker.note)
    .bind(blocker.owner_id)
    .execute(&mut *conn)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Clears the blocker. The task returns to `pending` only when it was blocked;
/// a task someone moved on while blocked keeps its newer status.
pub async fn clear_task_blocker(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    project_id: Uuid,
    task_id: Uuid,
) -> DbResult<bool> {
    let res = sqlx::query(
        r#"UPDATE project_tasks
              SET blocker_type = NULL, blocker_note = NULL, blocker_owner_id = NULL,
                  blocked_at = NULL,
                  status = CASE WHEN status = 'blocked' THEN 'pending' ELSE status END,
                  updated_at = now()
            WHERE org_id = $1 AND project_id = $2 AND id = $3
              AND blocker_type IS NOT NULL"#,
    )
    .bind(org_id)
    .bind(project_id)
    .bind(task_id)
    .execute(&mut *conn)
    .await?;
    Ok(res.rows_affected() > 0)
}

#[derive(Debug, Clone)]
pub struct PodSettings {
    pub id: Uuid,
    pub org_id: Uuid,
    pub surface: String,
    pub role_key: String,
    pub pods: Vec<String>,
    pub updated_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn row_to_pod_settings(row: sqlx::postgres::PgRow) -> PodSettings {
    PodSettings {
        id: row.get("id"),
        org_id: row.get("org_id"),
        surface: row.get("surface"),
        role_key: row.get("role_key"),
        pods: row.get("pods"),
        updated_by: row.get("updated_by"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

pub async fn list_pod_settings<'e, E>(
    executor: E,
    org_id: Uuid,
    surface: &str,
) -> DbResult<Vec<PodSettings>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
        r#"SELECT id, org_id, surface, role_key, pods, updated_by, created_at, updated_at
           FROM project_pod_settings
           WHERE org_id = $1 AND surface = $2
           ORDER BY role_key"#,
    )
    .bind(org_id)
    .bind(surface)
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(row_to_pod_settings).collect())
}

pub async fn upsert_pod_settings(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    surface: &str,
    role_key: &str,
    pods: &[String],
    updated_by: Uuid,
) -> DbResult<PodSettings> {
    let row = sqlx::query(
        r#"INSERT INTO project_pod_settings (id, org_id, surface, role_key, pods, updated_by)
           VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (org_id, surface, role_key)
           DO UPDATE SET pods = EXCLUDED.pods, updated_by = EXCLUDED.updated_by,
                         updated_at = now()
           RETURNING id, org_id, surface, role_key, pods, updated_by, created_at, updated_at"#,
    )
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(surface)
    .bind(role_key)
    .bind(pods)
    .bind(updated_by)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row_to_pod_settings(row))
}

#[derive(Debug, Clone)]
pub struct PodPrefs {
    pub user_id: Uuid,
    pub surface: String,
    pub hidden: Vec<String>,
    pub pod_order: Vec<String>,
}

pub async fn get_pod_prefs<'e, E>(
    executor: E,
    org_id: Uuid,
    user_id: Uuid,
    surface: &str,
) -> DbResult<Option<PodPrefs>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(
        r#"SELECT user_id, surface, hidden, pod_order FROM project_pod_prefs
           WHERE org_id = $1 AND user_id = $2 AND surface = $3"#,
    )
    .bind(org_id)
    .bind(user_id)
    .bind(surface)
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|row| PodPrefs {
        user_id: row.get("user_id"),
        surface: row.get("surface"),
        hidden: row.get("hidden"),
        pod_order: row.get("pod_order"),
    }))
}

pub async fn upsert_pod_prefs(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    surface: &str,
    hidden: &[String],
    pod_order: &[String],
) -> DbResult<()> {
    sqlx::query(
        r#"INSERT INTO project_pod_prefs
              (id, org_id, user_id, surface, hidden, pod_order)
           VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (org_id, user_id, surface)
           DO UPDATE SET hidden = EXCLUDED.hidden, pod_order = EXCLUDED.pod_order,
                         updated_at = now()"#,
    )
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(user_id)
    .bind(surface)
    .bind(hidden)
    .bind(pod_order)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Project-to-ticket link
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TicketLink {
    pub id: Uuid,
    pub org_id: Uuid,
    pub project_id: Uuid,
    pub task_id: Uuid,
    pub ticket_id: Uuid,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

fn row_to_ticket_link(row: sqlx::postgres::PgRow) -> TicketLink {
    TicketLink {
        id: row.get("id"),
        org_id: row.get("org_id"),
        project_id: row.get("project_id"),
        task_id: row.get("task_id"),
        ticket_id: row.get("ticket_id"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
    }
}

pub async fn link_exists_for_task<'e, E>(
    executor: E,
    org_id: Uuid,
    task_id: Uuid,
) -> DbResult<Option<TicketLink>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(
        r#"SELECT id, org_id, project_id, task_id, ticket_id, created_by, created_at
           FROM project_ticket_links WHERE org_id = $1 AND task_id = $2"#,
    )
    .bind(org_id)
    .bind(task_id)
    .fetch_optional(executor)
    .await?;
    Ok(row.map(row_to_ticket_link))
}

pub async fn link_by_ticket<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<Option<TicketLink>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(
        r#"SELECT id, org_id, project_id, task_id, ticket_id, created_by, created_at
           FROM project_ticket_links WHERE org_id = $1 AND ticket_id = $2"#,
    )
    .bind(org_id)
    .bind(ticket_id)
    .fetch_optional(executor)
    .await?;
    Ok(row.map(row_to_ticket_link))
}

pub async fn insert_ticket_link(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    project_id: Uuid,
    task_id: Uuid,
    ticket_id: Uuid,
    created_by: Option<Uuid>,
) -> DbResult<TicketLink> {
    let row = sqlx::query(
        r#"INSERT INTO project_ticket_links
              (id, org_id, project_id, task_id, ticket_id, created_by)
           VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (task_id) DO UPDATE SET task_id = EXCLUDED.task_id
           RETURNING id, org_id, project_id, task_id, ticket_id, created_by, created_at"#,
    )
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(project_id)
    .bind(task_id)
    .bind(ticket_id)
    .bind(created_by)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row_to_ticket_link(row))
}

// ---------------------------------------------------------------------------
// Portfolio
// ---------------------------------------------------------------------------

/// One project row with the phase facts the quarter view draws. A project with
/// no dated phases still appears once, with `phase_id` null, so "not
/// scheduled" is a readable state rather than a missing row.
#[derive(Debug, Clone)]
pub struct PortfolioRow {
    pub project_id: Uuid,
    pub project_name: String,
    pub project_status: String,
    pub client_id: Option<Uuid>,
    pub start_date: Option<NaiveDate>,
    pub due_date: Option<NaiveDate>,
    pub phase_id: Option<Uuid>,
    pub phase_name: Option<String>,
    pub phase_status: Option<String>,
    pub phase_start: Option<NaiveDate>,
    pub phase_end: Option<NaiveDate>,
}

pub async fn list_portfolio_rows<'e, E>(
    executor: E,
    org_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> DbResult<Vec<PortfolioRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
        r#"SELECT p.id AS project_id, p.name AS project_name, p.status AS project_status,
                  p.client_id, p.start_date, p.due_date,
                  ph.id AS phase_id, ph.name AS phase_name, ph.status AS phase_status,
                  ph.start_date AS phase_start, ph.end_date AS phase_end
           FROM projects p
           LEFT JOIN project_phases ph
             ON ph.project_id = p.id
            AND ph.start_date IS NOT NULL AND ph.end_date IS NOT NULL
            AND ph.end_date >= $2 AND ph.start_date <= $3
           WHERE p.org_id = $1
             AND p.status NOT IN ('cancelled')
             AND (p.start_date IS NULL OR p.start_date <= $3)
             AND (p.due_date IS NULL OR p.due_date >= $2)
           ORDER BY p.name, ph.sort_order, ph.id"#,
    )
    .bind(org_id)
    .bind(from)
    .bind(to)
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| PortfolioRow {
            project_id: row.get("project_id"),
            project_name: row.get("project_name"),
            project_status: row.get("project_status"),
            client_id: row.get("client_id"),
            start_date: row.get("start_date"),
            due_date: row.get("due_date"),
            phase_id: row.get("phase_id"),
            phase_name: row.get("phase_name"),
            phase_status: row.get("phase_status"),
            phase_start: row.get("phase_start"),
            phase_end: row.get("phase_end"),
        })
        .collect())
}

/// Projects outside the window (no dates or entirely outside it) — the tray
/// S7.8 draws, never on the chart.
pub async fn list_unscheduled_projects<'e, E>(
    executor: E,
    org_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> DbResult<Vec<(Uuid, String, String, Option<NaiveDate>, Option<NaiveDate>)>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
        r#"SELECT id, name, status, start_date, due_date FROM projects
           WHERE org_id = $1
             AND status NOT IN ('cancelled','completed')
             AND (start_date IS NULL OR due_date IS NULL
                  OR due_date < $2 OR start_date > $3)
           ORDER BY name"#,
    )
    .bind(org_id)
    .bind(from)
    .bind(to)
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.get("id"),
                row.get("name"),
                row.get("status"),
                row.get("start_date"),
                row.get("due_date"),
            )
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Shared portfolio views
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PortfolioShare {
    pub id: Uuid,
    pub org_id: Uuid,
    pub token: String,
    pub spec: Value,
    pub snapshot: Value,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

fn row_to_share(row: sqlx::postgres::PgRow) -> PortfolioShare {
    PortfolioShare {
        id: row.get("id"),
        org_id: row.get("org_id"),
        token: row.get("token"),
        spec: row.get("spec"),
        snapshot: row.get("snapshot"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
        expires_at: row.get("expires_at"),
    }
}

pub async fn insert_portfolio_share(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    token: &str,
    spec: &Value,
    snapshot: &Value,
    created_by: Option<Uuid>,
) -> DbResult<PortfolioShare> {
    let row = sqlx::query(
        r#"INSERT INTO project_portfolio_shares (id, org_id, token, spec, snapshot, created_by)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING id, org_id, token, spec, snapshot, created_by, created_at, expires_at"#,
    )
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(token)
    .bind(json(spec))
    .bind(json(snapshot))
    .bind(created_by)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row_to_share(row))
}

pub async fn get_portfolio_share<'e, E>(
    executor: E,
    org_id: Uuid,
    token: &str,
) -> DbResult<Option<PortfolioShare>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(
        r#"SELECT id, org_id, token, spec, snapshot, created_by, created_at, expires_at
           FROM project_portfolio_shares
           WHERE org_id = $1 AND token = $2
             AND (expires_at IS NULL OR expires_at > now())"#,
    )
    .bind(org_id)
    .bind(token)
    .fetch_optional(executor)
    .await?;
    Ok(row.map(row_to_share))
}

/// One task as the project-shaped Today and the "My work" pod read it: the
/// task, the project it belongs to, the phase it sits in, its typed blocker and
/// the ticket it was promoted to. A task with no project is not in this feed —
/// that is service work, and Today already owns it.
#[derive(Debug, Clone)]
pub struct MyTaskRow {
    pub task_id: Uuid,
    pub project_id: Uuid,
    pub project_name: String,
    pub phase_name: Option<String>,
    pub title: String,
    pub status: String,
    pub blocker_type: Option<String>,
    pub blocker_note: Option<String>,
    pub blocked_at: Option<DateTime<Utc>>,
    pub due_at: Option<DateTime<Utc>>,
    pub start_at: Option<DateTime<Utc>>,
    pub duration_minutes: Option<i32>,
    pub effort_remaining_minutes: Option<i32>,
    pub work_type: Option<String>,
    pub is_milestone: bool,
    pub ticket_id: Option<Uuid>,
}

pub async fn list_my_tasks<'e, E>(
    executor: E,
    org_id: Uuid,
    user_id: Uuid,
) -> DbResult<Vec<MyTaskRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
        r#"SELECT t.id AS task_id, p.id AS project_id, p.name AS project_name,
                  ph.name AS phase_name, t.title, t.status, t.blocker_type,
                  t.blocker_note, t.blocked_at, t.due_at, t.start_at,
                  t.duration_minutes, t.effort_remaining_minutes, t.work_type,
                  t.is_milestone, l.ticket_id
             FROM project_tasks t
             JOIN projects p ON p.id = t.project_id
             LEFT JOIN project_phases ph ON ph.id = t.phase_id
             LEFT JOIN project_ticket_links l ON l.task_id = t.id
            WHERE t.org_id = $1
              AND t.assignee_id = $2
              AND t.status NOT IN ('completed','cancelled')
              AND p.status NOT IN ('completed','cancelled')
            ORDER BY t.due_at NULLS LAST, p.name, t.sort_order"#,
    )
    .bind(org_id)
    .bind(user_id)
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| MyTaskRow {
            task_id: row.get("task_id"),
            project_id: row.get("project_id"),
            project_name: row.get("project_name"),
            phase_name: row.get("phase_name"),
            title: row.get("title"),
            status: row.get("status"),
            blocker_type: row.get("blocker_type"),
            blocker_note: row.get("blocker_note"),
            blocked_at: row.get("blocked_at"),
            due_at: row.get("due_at"),
            start_at: row.get("start_at"),
            duration_minutes: row.get("duration_minutes"),
            effort_remaining_minutes: row.get("effort_remaining_minutes"),
            work_type: row.get("work_type"),
            is_milestone: row.get("is_milestone"),
            ticket_id: row.get("ticket_id"),
        })
        .collect())
}

/// The portal's project list: only the caller's client's projects, and only
/// the fields a client may see (never hours, budget, rates or staff names).
pub async fn list_client_projects<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
) -> DbResult<Vec<(Uuid, String, String, Option<NaiveDate>, Option<NaiveDate>)>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
        r#"SELECT id, name, status, start_date, due_date FROM projects
           WHERE org_id = $1 AND client_id = $2 AND status <> 'cancelled'
           ORDER BY due_date NULLS LAST, name"#,
    )
    .bind(org_id)
    .bind(client_id)
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.get("id"),
                row.get("name"),
                row.get("status"),
                row.get("start_date"),
                row.get("due_date"),
            )
        })
        .collect())
}

/// The teammates who can be named as a project's manager. The project row does
/// not carry a manager column yet, so the quarter view groups by the person a
/// task is assigned to and falls back to "Unassigned". This read exists so the
/// grouping has labels rather than ids.
pub async fn list_teammates<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<(Uuid, String)>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
        r#"SELECT id, display_name FROM users
           WHERE org_id = $1 AND is_active = true
           ORDER BY display_name"#,
    )
    .bind(org_id)
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.get("id"), row.get("display_name")))
        .collect())
}
