//! Projects-line queries (migration 0030): projects → phases → tasks,
//! task dependencies, saved views, and templates. Every function is
//! org-scoped; rows are RLS-filtered in transactions bound via `set_org`.
//! Dynamic filters and tri-state patches use runtime-checked queries
//! (`tickets.rs` convention); mappers parse status enums with typed
//! decode errors.

use crate::{DbError, DbResult};
use chrono::{DateTime, Utc};
use core_domain::entities::{
    BillingMethod, PhaseBurn, PhaseBurnAlert, PhaseStatus, Project, ProjectBillingRelease,
    ProjectPhase, ProjectPhaseBudget, ProjectStatus, ProjectTask, ProjectTemplate, ProjectView,
    ReleaseStatus, TaskStatus,
};
use sqlx::{Postgres, Row};
use uuid::Uuid;

const PROJECT_COLS: &str = "id, org_id, client_id, name, description, status, billing_method, \
     budget_minor, currency, start_date, due_date, billing_contract_id, client_hold_reason, \
     pre_hold_status, created_at, updated_at";
const PHASE_COLS: &str = "id, org_id, project_id, name, description, sort_order, status, \
     is_milestone, milestone_amount_minor, milestone_currency, start_date, end_date, \
     completed_at, created_at, updated_at";
const TASK_COLS: &str = "id, org_id, project_id, phase_id, title, description, status, \
     assignee_id, sort_order, is_milestone, milestone_amount_minor, milestone_currency, \
     due_at, start_at, duration_minutes, effort_estimate_minutes, effort_remaining_minutes, \
     work_type, priority, role_key, required_skills, is_critical, baseline_start_at, \
     baseline_duration_minutes, external_owner, calendar_pin, completed_at, created_at, \
     updated_at";
const VIEW_COLS: &str = "id, org_id, project_id, name, spec, is_default, created_at, updated_at";
const TEMPLATE_COLS: &str = "id, org_id, name, description, spec, created_at, updated_at";

fn decode_err(column: &str, e: core_domain::DomainError) -> sqlx::Error {
    sqlx::Error::ColumnDecode {
        index: column.into(),
        source: Box::new(e),
    }
}

fn row_to_project(row: sqlx::postgres::PgRow) -> Result<Project, sqlx::Error> {
    Ok(Project {
        id: row.get("id"),
        org_id: row.get("org_id"),
        client_id: row.get("client_id"),
        name: row.get("name"),
        description: row.get("description"),
        status: row
            .get::<String, _>("status")
            .parse()
            .map_err(|e| decode_err("status", e))?,
        billing_method: row
            .get::<String, _>("billing_method")
            .parse()
            .map_err(|e| decode_err("billing_method", e))?,
        budget_minor: row.get("budget_minor"),
        currency: row.get("currency"),
        start_date: row.get("start_date"),
        due_date: row.get("due_date"),
        billing_contract_id: row.get("billing_contract_id"),
        client_hold_reason: row.get("client_hold_reason"),
        pre_hold_status: row
            .get::<Option<String>, _>("pre_hold_status")
            .map(|raw| raw.parse())
            .transpose()
            .map_err(|e| decode_err("pre_hold_status", e))?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn row_to_phase(row: sqlx::postgres::PgRow) -> Result<ProjectPhase, sqlx::Error> {
    Ok(ProjectPhase {
        id: row.get("id"),
        org_id: row.get("org_id"),
        project_id: row.get("project_id"),
        name: row.get("name"),
        description: row.get("description"),
        sort_order: row.get("sort_order"),
        status: row
            .get::<String, _>("status")
            .parse()
            .map_err(|e| decode_err("status", e))?,
        is_milestone: row.get("is_milestone"),
        milestone_amount_minor: row.get("milestone_amount_minor"),
        milestone_currency: row.get("milestone_currency"),
        start_date: row.get("start_date"),
        end_date: row.get("end_date"),
        completed_at: row.get("completed_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn row_to_task(row: sqlx::postgres::PgRow) -> Result<ProjectTask, sqlx::Error> {
    Ok(ProjectTask {
        id: row.get("id"),
        org_id: row.get("org_id"),
        project_id: row.get("project_id"),
        phase_id: row.get("phase_id"),
        title: row.get("title"),
        description: row.get("description"),
        status: row
            .get::<String, _>("status")
            .parse()
            .map_err(|e| decode_err("status", e))?,
        assignee_id: row.get("assignee_id"),
        sort_order: row.get("sort_order"),
        is_milestone: row.get("is_milestone"),
        milestone_amount_minor: row.get("milestone_amount_minor"),
        milestone_currency: row.get("milestone_currency"),
        due_at: row.get("due_at"),
        start_at: row.get("start_at"),
        duration_minutes: row.get("duration_minutes"),
        effort_estimate_minutes: row.get("effort_estimate_minutes"),
        effort_remaining_minutes: row.get("effort_remaining_minutes"),
        work_type: row
            .get::<Option<String>, _>("work_type")
            .map(|s| s.parse().map_err(|e| decode_err("work_type", e)))
            .transpose()?,
        priority: row
            .get::<Option<String>, _>("priority")
            .map(|s| s.parse().map_err(|e| decode_err("priority", e)))
            .transpose()?,
        role_key: row.get("role_key"),
        required_skills: row.get("required_skills"),
        is_critical: row.get("is_critical"),
        baseline_start_at: row.get("baseline_start_at"),
        baseline_duration_minutes: row.get("baseline_duration_minutes"),
        external_owner: row.get("external_owner"),
        calendar_pin: row.get("calendar_pin"),
        completed_at: row.get("completed_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn row_to_view(row: sqlx::postgres::PgRow) -> Result<ProjectView, sqlx::Error> {
    use core_domain::entities::ProjectView as V;
    Ok(V {
        id: row.get("id"),
        org_id: row.get("org_id"),
        project_id: row.get("project_id"),
        name: row.get("name"),
        spec: row.get("spec"),
        is_default: row.get("is_default"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn row_to_template(row: sqlx::postgres::PgRow) -> Result<ProjectTemplate, sqlx::Error> {
    use core_domain::entities::ProjectTemplate as T;
    Ok(T {
        id: row.get("id"),
        org_id: row.get("org_id"),
        name: row.get("name"),
        description: row.get("description"),
        spec: row.get("spec"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

// --- projects ------------------------------------------------------------------

pub struct NewProject<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub client_id: Option<Uuid>,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub billing_method: BillingMethod,
    pub budget_minor: Option<i64>,
    pub currency: Option<&'a str>,
    pub start_date: Option<chrono::NaiveDate>,
    pub due_date: Option<chrono::NaiveDate>,
    /// The contract the project bills through (Wave 39 lane 4, report
    /// P2). `None` = non-billable or unlinked.
    pub billing_contract_id: Option<Uuid>,
}

pub async fn insert_project<'e, E>(executor: E, p: NewProject<'_>) -> DbResult<Project>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"INSERT INTO projects (id, org_id, client_id, name, description, billing_method,
                                 budget_minor, currency, start_date, due_date, billing_contract_id)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
           RETURNING {PROJECT_COLS}"#
    );
    let row = sqlx::query(&sql)
        .bind(p.id)
        .bind(p.org_id)
        .bind(p.client_id)
        .bind(p.name)
        .bind(p.description)
        .bind(p.billing_method.as_str())
        .bind(p.budget_minor)
        .bind(p.currency)
        .bind(p.start_date)
        .bind(p.due_date)
        .bind(p.billing_contract_id)
        .fetch_one(executor)
        .await?;
    Ok(row_to_project(row)?)
}

/// The project row under a write lock, for a patch that decides from the
/// current status (the hold and resume transition). The lock is held until
/// the transaction ends, so the value the decision used is the value the
/// update writes over.
pub async fn get_project_for_update<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<Project>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql =
        format!("SELECT {PROJECT_COLS} FROM projects WHERE org_id = $1 AND id = $2 FOR UPDATE");
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_project(row)?)
}

pub async fn get_project<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<Project>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {PROJECT_COLS} FROM projects WHERE org_id = $1 AND id = $2");
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_project(row)?)
}

/// Pre-insert name-uniqueness check for a typed 409 (the UNIQUE(org_id,
/// name) constraint stays as the concurrent-write backstop).
pub async fn project_name_taken<'e, E>(executor: E, org_id: Uuid, name: &str) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let taken: Option<i32> =
        sqlx::query_scalar("SELECT 1 FROM projects WHERE org_id = $1 AND name = $2")
            .bind(org_id)
            .bind(name)
            .fetch_optional(executor)
            .await?;
    Ok(taken.is_some())
}

pub struct ProjectFilter {
    pub status: Option<ProjectStatus>,
    pub client_id: Option<Uuid>,
    pub limit: i64,
    pub offset: i64,
}

pub async fn list_projects<'e, E>(
    executor: E,
    org_id: Uuid,
    f: ProjectFilter,
) -> DbResult<Vec<Project>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"SELECT {PROJECT_COLS} FROM projects
           WHERE org_id = $1
             AND ($2::text IS NULL OR status = $2)
             AND ($3::uuid IS NULL OR client_id = $3)
           ORDER BY created_at DESC
           LIMIT $4 OFFSET $5"#
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(f.status.map(|s| s.as_str().to_string()))
        .bind(f.client_id)
        .bind(f.limit.clamp(1, 200))
        .bind(f.offset.max(0))
        .fetch_all(executor)
        .await?;
    Ok(rows
        .into_iter()
        .map(row_to_project)
        .collect::<Result<Vec<_>, _>>()?)
}

pub struct ProjectPatch<'a> {
    pub name: Option<&'a str>,
    pub description: Option<Option<&'a str>>,
    pub client_id: Option<Option<Uuid>>,
    pub status: Option<ProjectStatus>,
    pub billing_method: Option<BillingMethod>,
    pub budget_minor: Option<Option<i64>>,
    pub currency: Option<Option<&'a str>>,
    pub start_date: Option<Option<chrono::NaiveDate>>,
    pub due_date: Option<Option<chrono::NaiveDate>>,
    /// `Some(Some(v))` links; `Some(None)` unlinks; `None` = no change.
    pub billing_contract_id: Option<Option<Uuid>>,
    /// `Some(Some(v))` sets the client-safe hold reason; `Some(None)` clears
    /// it; `None` = no change. The route decides when a change is legal (set
    /// while on hold, cleared on the way out of hold).
    pub client_hold_reason: Option<Option<&'a str>>,
    /// `Some(Some(v))` records the status a pause interrupted; `Some(None)`
    /// clears it on resume; `None` = no change.
    pub pre_hold_status: Option<Option<&'a str>>,
}

/// Tri-state patch (QueryBuilder pattern from `tickets::update_ticket`):
/// `None` = no change, `Some(None)` = clear, `Some(Some(v))` = set.
/// Returns the updated row so mutation handlers get read-your-write.
pub async fn update_project<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    p: ProjectPatch<'_>,
) -> DbResult<Project>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let mut q = sqlx::QueryBuilder::new("UPDATE projects SET updated_at = now()");
    if let Some(v) = p.name {
        q.push(", name = ").push_bind(v.to_string());
    }
    if let Some(v) = p.description {
        q.push(", description = ").push_bind(v);
    }
    if let Some(v) = p.client_id {
        q.push(", client_id = ").push_bind(v);
    }
    if let Some(v) = p.status {
        q.push(", status = ").push_bind(v.as_str().to_string());
    }
    if let Some(v) = p.billing_method {
        q.push(", billing_method = ")
            .push_bind(v.as_str().to_string());
    }
    if let Some(v) = p.budget_minor {
        q.push(", budget_minor = ").push_bind(v);
    }
    if let Some(v) = p.currency {
        q.push(", currency = ").push_bind(v);
    }
    if let Some(v) = p.start_date {
        q.push(", start_date = ").push_bind(v);
    }
    if let Some(v) = p.due_date {
        q.push(", due_date = ").push_bind(v);
    }
    if let Some(v) = p.billing_contract_id {
        q.push(", billing_contract_id = ").push_bind(v);
    }
    if let Some(v) = p.client_hold_reason {
        q.push(", client_hold_reason = ").push_bind(v);
    }
    if let Some(v) = p.pre_hold_status {
        q.push(", pre_hold_status = ").push_bind(v);
    }
    q.push(" WHERE org_id = ")
        .push_bind(org_id)
        .push(" AND id = ")
        .push_bind(id)
        .push(" RETURNING ")
        .push(PROJECT_COLS);
    let row = q
        .build()
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_project(row)?)
}

// --- phases ---------------------------------------------------------------------
pub struct NewPhase<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub project_id: Uuid,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub sort_order: i32,
    pub is_milestone: bool,
    pub milestone_amount_minor: Option<i64>,
    pub milestone_currency: Option<&'a str>,
    /// The planned window; `None` on both ends keeps the phase off the Gantt.
    pub start_date: Option<chrono::NaiveDate>,
    pub end_date: Option<chrono::NaiveDate>,
}

/// The phase's billing record (Wave 39 lane 4): budget denominator
/// and/or the fixed fee released on phase completion. Written via
/// [`upsert_phase_budget`], keyed per phase.
pub struct NewPhaseBudget<'a> {
    pub budget_minor: Option<i64>,
    pub currency: Option<&'a str>,
    pub fee_minor: Option<i64>,
    pub fee_currency: Option<&'a str>,
}

pub async fn insert_phase<'e, E>(executor: E, p: NewPhase<'_>) -> DbResult<ProjectPhase>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"INSERT INTO project_phases (id, org_id, project_id, name, description, sort_order,
                                       is_milestone, milestone_amount_minor, milestone_currency,
                                       start_date, end_date)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
           RETURNING {PHASE_COLS}"#
    );
    let row = sqlx::query(&sql)
        .bind(p.id)
        .bind(p.org_id)
        .bind(p.project_id)
        .bind(p.name)
        .bind(p.description)
        .bind(p.sort_order)
        .bind(p.is_milestone)
        .bind(p.milestone_amount_minor)
        .bind(p.milestone_currency)
        .bind(p.start_date)
        .bind(p.end_date)
        .fetch_one(executor)
        .await?;
    Ok(row_to_phase(row)?)
}

pub async fn get_phase<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<ProjectPhase>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {PHASE_COLS} FROM project_phases WHERE org_id = $1 AND id = $2");
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_phase(row)?)
}

pub async fn list_phases<'e, E>(
    executor: E,
    org_id: Uuid,
    project_id: Uuid,
) -> DbResult<Vec<ProjectPhase>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"SELECT {PHASE_COLS} FROM project_phases
           WHERE org_id = $1 AND project_id = $2
           ORDER BY sort_order, created_at"#
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(project_id)
        .fetch_all(executor)
        .await?;
    Ok(rows
        .into_iter()
        .map(row_to_phase)
        .collect::<Result<Vec<_>, _>>()?)
}

pub struct PhasePatch<'a> {
    pub name: Option<&'a str>,
    pub description: Option<Option<&'a str>>,
    pub sort_order: Option<i32>,
    pub status: Option<PhaseStatus>,
    pub is_milestone: Option<bool>,
    pub milestone_amount_minor: Option<Option<i64>>,
    pub milestone_currency: Option<Option<&'a str>>,
    /// `Some(None)` CLEARS a date, which is how a planner takes a phase
    /// off the Gantt. `None` (the field absent) leaves it untouched, so a
    /// rename cannot silently undate a phase.
    pub start_date: Option<Option<chrono::NaiveDate>>,
    pub end_date: Option<Option<chrono::NaiveDate>>,
}

pub async fn update_phase<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    p: PhasePatch<'_>,
) -> DbResult<ProjectPhase>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let mut q = sqlx::QueryBuilder::new("UPDATE project_phases SET updated_at = now()");
    if let Some(v) = p.name {
        q.push(", name = ").push_bind(v.to_string());
    }
    if let Some(v) = p.description {
        q.push(", description = ").push_bind(v);
    }
    if let Some(v) = p.sort_order {
        q.push(", sort_order = ").push_bind(v);
    }
    if let Some(v) = p.status {
        q.push(", status = ").push_bind(v.as_str().to_string());
    }
    if let Some(v) = p.is_milestone {
        q.push(", is_milestone = ").push_bind(v);
    }
    if let Some(v) = p.milestone_amount_minor {
        q.push(", milestone_amount_minor = ").push_bind(v);
    }
    if let Some(v) = p.milestone_currency {
        q.push(", milestone_currency = ").push_bind(v);
    }
    // `Some(None)` clears a date, `None` leaves it alone. The distinction is
    // what lets a rename preserve the Gantt window while a deliberate
    // "undate this phase" removes it.
    if let Some(v) = p.start_date {
        q.push(", start_date = ").push_bind(v);
    }
    if let Some(v) = p.end_date {
        q.push(", end_date = ").push_bind(v);
    }
    q.push(" WHERE org_id = ")
        .push_bind(org_id)
        .push(" AND id = ")
        .push_bind(id)
        .push(" RETURNING ")
        .push(PHASE_COLS);
    let row = q
        .build()
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_phase(row)?)
}

/// The completion write: stamps status + `completed_at` atomically.
pub async fn complete_phase<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    at: chrono::DateTime<chrono::Utc>,
) -> DbResult<ProjectPhase>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"UPDATE project_phases SET status = 'completed', completed_at = $3, updated_at = now()
           WHERE org_id = $1 AND id = $2 RETURNING {PHASE_COLS}"#
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .bind(at)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_phase(row)?)
}

// --- tasks -----------------------------------------------------------------------

pub struct NewTask<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub project_id: Uuid,
    pub phase_id: Option<Uuid>,
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub assignee_id: Option<Uuid>,
    pub sort_order: i32,
    pub is_milestone: bool,
    pub milestone_amount_minor: Option<i64>,
    pub milestone_currency: Option<&'a str>,
    pub due_at: Option<chrono::DateTime<chrono::Utc>>,
    pub start_at: Option<chrono::DateTime<chrono::Utc>>,
    pub duration_minutes: Option<i32>,
    pub effort_estimate_minutes: Option<i32>,
    pub effort_remaining_minutes: Option<i32>,
    pub work_type: Option<core_domain::entities::WorkType>,
    pub priority: Option<core_domain::status::Priority>,
    pub role_key: Option<&'a str>,
    pub required_skills: &'a [String],
    pub is_critical: bool,
    pub external_owner: Option<&'a str>,
    pub calendar_pin: Option<Uuid>,
}

pub async fn insert_task<'e, E>(executor: E, t: NewTask<'_>) -> DbResult<ProjectTask>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"INSERT INTO project_tasks (id, org_id, project_id, phase_id, title, description,
                                      assignee_id, sort_order, is_milestone,
                                      milestone_amount_minor, milestone_currency, due_at,
                                      start_at, duration_minutes, effort_estimate_minutes,
                                      effort_remaining_minutes, work_type, priority, role_key,
                                      required_skills, is_critical, external_owner, calendar_pin)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
                   $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23)
           RETURNING {TASK_COLS}"#
    );
    let row = sqlx::query(&sql)
        .bind(t.id)
        .bind(t.org_id)
        .bind(t.project_id)
        .bind(t.phase_id)
        .bind(t.title)
        .bind(t.description)
        .bind(t.assignee_id)
        .bind(t.sort_order)
        .bind(t.is_milestone)
        .bind(t.milestone_amount_minor)
        .bind(t.milestone_currency)
        .bind(t.due_at)
        .bind(t.start_at)
        .bind(t.duration_minutes)
        .bind(t.effort_estimate_minutes)
        .bind(t.effort_remaining_minutes)
        .bind(t.work_type.map(|w| w.as_str()))
        .bind(t.priority.map(|p| p.as_str()))
        .bind(t.role_key)
        .bind(t.required_skills)
        .bind(t.is_critical)
        .bind(t.external_owner)
        .bind(t.calendar_pin)
        .fetch_one(executor)
        .await?;
    Ok(row_to_task(row)?)
}

pub async fn get_task<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<ProjectTask>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {TASK_COLS} FROM project_tasks WHERE org_id = $1 AND id = $2");
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_task(row)?)
}

pub struct TaskFilter {
    pub phase_id: Option<Uuid>,
    pub status: Option<TaskStatus>,
    pub assignee_id: Option<Uuid>,
    pub milestone: Option<bool>,
    pub limit: i64,
    pub offset: i64,
}

pub async fn list_tasks<'e, E>(
    executor: E,
    org_id: Uuid,
    project_id: Uuid,
    f: TaskFilter,
) -> DbResult<Vec<ProjectTask>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"SELECT {TASK_COLS} FROM project_tasks
           WHERE org_id = $1 AND project_id = $2
             AND ($3::uuid IS NULL OR phase_id = $3)
             AND ($4::text IS NULL OR status = $4)
             AND ($5::uuid IS NULL OR assignee_id = $5)
             AND ($6::boolean IS NULL OR is_milestone = $6)
           ORDER BY sort_order, created_at
           LIMIT $7 OFFSET $8"#
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(project_id)
        .bind(f.phase_id)
        .bind(f.status.map(|s| s.as_str().to_string()))
        .bind(f.assignee_id)
        .bind(f.milestone)
        .bind(f.limit.clamp(1, 200))
        .bind(f.offset.max(0))
        .fetch_all(executor)
        .await?;
    Ok(rows
        .into_iter()
        .map(row_to_task)
        .collect::<Result<Vec<_>, _>>()?)
}

#[derive(Default)]
pub struct TaskPatch<'a> {
    pub title: Option<&'a str>,
    pub description: Option<Option<&'a str>>,
    pub phase_id: Option<Option<Uuid>>,
    pub status: Option<TaskStatus>,
    pub assignee_id: Option<Option<Uuid>>,
    pub sort_order: Option<i32>,
    pub is_milestone: Option<bool>,
    pub milestone_amount_minor: Option<Option<i64>>,
    pub milestone_currency: Option<Option<&'a str>>,
    pub due_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    pub start_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    pub duration_minutes: Option<Option<i32>>,
    pub effort_estimate_minutes: Option<Option<i32>>,
    pub effort_remaining_minutes: Option<Option<i32>>,
    pub work_type: Option<Option<core_domain::entities::WorkType>>,
    pub priority: Option<Option<core_domain::status::Priority>>,
    pub role_key: Option<Option<&'a str>>,
    pub required_skills: Option<&'a [String]>,
    pub is_critical: Option<bool>,
    pub external_owner: Option<Option<&'a str>>,
    pub calendar_pin: Option<Option<Uuid>>,
}

/// Tri-state task patch. `completed` is NOT settable here — completion runs
/// through `complete_task` so dependency gating + billing-seam events
/// cannot be skipped.
pub async fn update_task<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    p: TaskPatch<'_>,
) -> DbResult<ProjectTask>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let mut q = sqlx::QueryBuilder::new("UPDATE project_tasks SET updated_at = now()");
    if let Some(v) = p.title {
        q.push(", title = ").push_bind(v.to_string());
    }
    if let Some(v) = p.description {
        q.push(", description = ").push_bind(v);
    }
    if let Some(v) = p.phase_id {
        q.push(", phase_id = ").push_bind(v);
    }
    if let Some(v) = p.status {
        q.push(", status = ").push_bind(v.as_str().to_string());
    }
    if let Some(v) = p.assignee_id {
        q.push(", assignee_id = ").push_bind(v);
    }
    if let Some(v) = p.sort_order {
        q.push(", sort_order = ").push_bind(v);
    }
    if let Some(v) = p.is_milestone {
        q.push(", is_milestone = ").push_bind(v);
    }
    if let Some(v) = p.milestone_amount_minor {
        q.push(", milestone_amount_minor = ").push_bind(v);
    }
    if let Some(v) = p.milestone_currency {
        q.push(", milestone_currency = ").push_bind(v);
    }
    if let Some(v) = p.due_at {
        q.push(", due_at = ").push_bind(v);
    }
    if let Some(v) = p.start_at {
        q.push(", start_at = ").push_bind(v);
    }
    if let Some(v) = p.duration_minutes {
        q.push(", duration_minutes = ").push_bind(v);
    }
    if let Some(v) = p.effort_estimate_minutes {
        q.push(", effort_estimate_minutes = ").push_bind(v);
    }
    if let Some(v) = p.effort_remaining_minutes {
        q.push(", effort_remaining_minutes = ").push_bind(v);
    }
    if let Some(v) = p.work_type {
        q.push(", work_type = ")
            .push_bind(v.map(|w| w.as_str().to_string()));
    }
    if let Some(v) = p.priority {
        q.push(", priority = ")
            .push_bind(v.map(|p| p.as_str().to_string()));
    }
    if let Some(v) = p.role_key {
        q.push(", role_key = ").push_bind(v);
    }
    if let Some(v) = p.required_skills {
        q.push(", required_skills = ").push_bind(v);
    }
    if let Some(v) = p.is_critical {
        q.push(", is_critical = ").push_bind(v);
    }
    if let Some(v) = p.external_owner {
        q.push(", external_owner = ").push_bind(v);
    }
    if let Some(v) = p.calendar_pin {
        q.push(", calendar_pin = ").push_bind(v);
    }
    q.push(" WHERE org_id = ")
        .push_bind(org_id)
        .push(" AND id = ")
        .push_bind(id)
        .push(" RETURNING ")
        .push(TASK_COLS);
    let row = q
        .build()
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_task(row)?)
}

/// Rewrite a project's phase order in one statement.
///
/// `ids` IS the order: the first id becomes 0, the second 1, and so on. The
/// statement is scoped to the project, so an id from another project matches
/// nothing and is never touched — the caller compares the returned row count
/// with `ids.len()` to tell a stranger from a no-op. Callers validate that the
/// list is a permutation of the project's phases; this function only writes.
pub async fn set_phase_order<'e, E>(
    executor: E,
    org_id: Uuid,
    project_id: Uuid,
    ids: &[Uuid],
) -> DbResult<u64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query(
        r#"UPDATE project_phases AS p
              SET sort_order = o.ord - 1, updated_at = now()
             FROM unnest($1::uuid[]) WITH ORDINALITY AS o(id, ord)
            WHERE p.id = o.id AND p.org_id = $2 AND p.project_id = $3"#,
    )
    .bind(ids)
    .bind(org_id)
    .bind(project_id)
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Rewrite the order of one group of tasks, and place them in `phase_id`.
///
/// `start` is where the group begins in the plan's ONE sequence: groups are
/// numbered one after another in phase order, not each from zero. A global
/// sequence is what makes the task list read in plan order, because the read is
/// `ORDER BY sort_order, created_at` across the whole project; per-phase
/// numbering would leave every phase's first task at zero and the list would
/// fall back to creation time.
///
/// The tasks of a plan are one list per phase (`None` is the unphased group),
/// so a drag across a phase boundary is the same statement as a drag within
/// one: the phase is part of what the order says. Scoped like
/// [`set_phase_order`], and the caller compares the row count.
pub async fn set_task_order<'e, E>(
    executor: E,
    org_id: Uuid,
    project_id: Uuid,
    phase_id: Option<Uuid>,
    ids: &[Uuid],
    start: i32,
) -> DbResult<u64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query(
        r#"UPDATE project_tasks AS t
              SET sort_order = $5::int + o.ord - 1, phase_id = $4, updated_at = now()
             FROM unnest($1::uuid[]) WITH ORDINALITY AS o(id, ord)
            WHERE t.id = o.id AND t.org_id = $2 AND t.project_id = $3"#,
    )
    .bind(ids)
    .bind(org_id)
    .bind(project_id)
    .bind(phase_id)
    .bind(start)
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Every task of a project with the phase it sits in.
///
/// The order endpoints need the whole set, and `list_tasks` is a page (its
/// limit is clamped for the list route), so this read is deliberately bare:
/// two columns, no filter, no cap.
pub async fn task_phase_map<'e, E>(
    executor: E,
    org_id: Uuid,
    project_id: Uuid,
) -> DbResult<Vec<(Uuid, Option<Uuid>)>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
        r#"SELECT id, phase_id FROM project_tasks
            WHERE org_id = $1 AND project_id = $2"#,
    )
    .bind(org_id)
    .bind(project_id)
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            use sqlx::Row;
            (
                row.get::<Uuid, _>("id"),
                row.get::<Option<Uuid>, _>("phase_id"),
            )
        })
        .collect())
}

/// The completion write: stamps status + `completed_at` atomically. Callers
/// MUST have validated dependency readiness first (see `incomplete_deps`).
pub async fn complete_task<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    at: chrono::DateTime<chrono::Utc>,
) -> DbResult<ProjectTask>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"UPDATE project_tasks SET status = 'completed', completed_at = $3, updated_at = now()
           WHERE org_id = $1 AND id = $2 RETURNING {TASK_COLS}"#
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .bind(at)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_task(row)?)
}

// --- dependencies ------------------------------------------------------------------

/// Adds a finish-to-start edge (`task` waits for `depends_on`). Idempotent
/// (PK dedupe). The caller validates same-project membership + acyclicity
/// first via `core_projects::deps`.
pub async fn add_task_dep<'e, E>(
    executor: E,
    org_id: Uuid,
    task_id: Uuid,
    depends_on_task_id: Uuid,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"INSERT INTO project_task_deps (org_id, task_id, depends_on_task_id)
           VALUES ($1, $2, $3)
           ON CONFLICT (task_id, depends_on_task_id) DO NOTHING"#,
    )
    .bind(org_id)
    .bind(task_id)
    .bind(depends_on_task_id)
    .execute(executor)
    .await?;
    Ok(())
}

/// Replaces the whole dependency set of one task inside the caller's
/// transaction (PUT semantics). `core_projects::deps` validates the new
/// set BEFORE this runs. Multi-statement: takes `&mut PgConnection` so
/// transactions reborrow via `&mut *tx`.
pub async fn set_task_deps(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    task_id: Uuid,
    depends_on: &[Uuid],
) -> DbResult<()> {
    sqlx::query("DELETE FROM project_task_deps WHERE org_id = $1 AND task_id = $2")
        .bind(org_id)
        .bind(task_id)
        .execute(&mut *conn)
        .await?;
    for dep in depends_on {
        sqlx::query(
            r#"INSERT INTO project_task_deps (org_id, task_id, depends_on_task_id)
               VALUES ($1, $2, $3)"#,
        )
        .bind(org_id)
        .bind(task_id)
        .bind(dep)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// Removes one edge; `false` = the edge did not exist (typed not-found).
pub async fn remove_task_dep<'e, E>(
    executor: E,
    org_id: Uuid,
    task_id: Uuid,
    depends_on_task_id: Uuid,
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query(
        r#"DELETE FROM project_task_deps
           WHERE org_id = $1 AND task_id = $2 AND depends_on_task_id = $3"#,
    )
    .bind(org_id)
    .bind(task_id)
    .bind(depends_on_task_id)
    .execute(executor)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// The task's current direct dependencies, as full rows.
pub async fn list_task_deps<'e, E>(
    executor: E,
    org_id: Uuid,
    task_id: Uuid,
) -> DbResult<Vec<ProjectTask>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"SELECT {TASK_COLS} FROM project_tasks
           WHERE org_id = $1 AND id IN (
             SELECT depends_on_task_id FROM project_task_deps
             WHERE org_id = $1 AND task_id = $2)
           ORDER BY sort_order, created_at"#
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(task_id)
        .fetch_all(executor)
        .await?;
    Ok(rows
        .into_iter()
        .map(row_to_task)
        .collect::<Result<Vec<_>, _>>()?)
}

/// The whole project's edge list, for graph validation + template expansion.
pub async fn list_project_dep_edges<'e, E>(
    executor: E,
    org_id: Uuid,
    project_id: Uuid,
) -> DbResult<Vec<(Uuid, Uuid)>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
        r#"SELECT d.task_id, d.depends_on_task_id
           FROM project_task_deps d
           JOIN project_tasks t ON t.id = d.task_id AND t.org_id = d.org_id
           WHERE d.org_id = $1 AND t.project_id = $2
           ORDER BY d.task_id, d.depends_on_task_id"#,
    )
    .bind(org_id)
    .bind(project_id)
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// IDs of the task's dependencies that are NOT completed — the typed 409
/// payload for a premature completion attempt.
pub async fn incomplete_deps<'e, E>(executor: E, org_id: Uuid, task_id: Uuid) -> DbResult<Vec<Uuid>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows: Vec<Uuid> = sqlx::query_scalar(
        r#"SELECT depends_on_task_id FROM project_task_deps d
           JOIN project_tasks t ON t.id = d.depends_on_task_id AND t.org_id = d.org_id
           WHERE d.org_id = $1 AND d.task_id = $2
             AND t.status <> 'completed'
           ORDER BY depends_on_task_id"#,
    )
    .bind(org_id)
    .bind(task_id)
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// All task ids of one project — the membership set dependency validation
/// checks against (deps never cross projects).
pub async fn list_project_task_ids<'e, E>(
    executor: E,
    org_id: Uuid,
    project_id: Uuid,
) -> DbResult<Vec<Uuid>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM project_tasks WHERE org_id = $1 AND project_id = $2 ORDER BY id",
    )
    .bind(org_id)
    .bind(project_id)
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

// --- views --------------------------------------------------------------------------

pub struct NewView<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub project_id: Option<Uuid>,
    pub name: &'a str,
    pub spec: &'a serde_json::Value,
    pub is_default: bool,
}

pub async fn insert_view<'e, E>(executor: E, v: NewView<'_>) -> DbResult<ProjectView>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"INSERT INTO project_views (id, org_id, project_id, name, spec, is_default)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING {VIEW_COLS}"#
    );
    let row = sqlx::query(&sql)
        .bind(v.id)
        .bind(v.org_id)
        .bind(v.project_id)
        .bind(v.name)
        .bind(v.spec)
        .bind(v.is_default)
        .fetch_one(executor)
        .await?;
    Ok(row_to_view(row)?)
}

pub async fn get_view<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<ProjectView>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {VIEW_COLS} FROM project_views WHERE org_id = $1 AND id = $2");
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_view(row)?)
}

/// Views scoped to one project, or the org's ENTIRE view set (project +
/// org-wide) when `project_id` is `None`.
pub async fn list_views<'e, E>(
    executor: E,
    org_id: Uuid,
    project_id: Option<Uuid>,
) -> DbResult<Vec<ProjectView>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = match project_id {
        Some(_) => format!(
            r#"SELECT {VIEW_COLS} FROM project_views
               WHERE org_id = $1 AND project_id = $2
               ORDER BY is_default DESC, name"#
        ),
        None => format!(
            r#"SELECT {VIEW_COLS} FROM project_views
               WHERE org_id = $1
               ORDER BY project_id NULLS LAST, is_default DESC, name"#
        ),
    };
    let mut q = sqlx::query(&sql).bind(org_id);
    if let Some(pid) = project_id {
        q = q.bind(pid);
    }
    let rows = q.fetch_all(executor).await?;
    Ok(rows
        .into_iter()
        .map(row_to_view)
        .collect::<Result<Vec<_>, _>>()?)
}

pub struct ViewPatch<'a> {
    pub name: Option<&'a str>,
    pub spec: Option<&'a serde_json::Value>,
    pub is_default: Option<bool>,
}

pub async fn update_view<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    p: ViewPatch<'_>,
) -> DbResult<ProjectView>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let mut q = sqlx::QueryBuilder::new("UPDATE project_views SET updated_at = now()");
    if let Some(v) = p.name {
        q.push(", name = ").push_bind(v.to_string());
    }
    if let Some(v) = p.spec {
        q.push(", spec = ").push_bind(v);
    }
    if let Some(v) = p.is_default {
        q.push(", is_default = ").push_bind(v);
    }
    q.push(" WHERE org_id = ")
        .push_bind(org_id)
        .push(" AND id = ")
        .push_bind(id)
        .push(" RETURNING ")
        .push(VIEW_COLS);
    let row = q
        .build()
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_view(row)?)
}

pub async fn delete_view<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query("DELETE FROM project_views WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(executor)
        .await?;
    Ok(res.rows_affected() > 0)
}

// --- templates ------------------------------------------------------------------------

pub struct NewTemplate<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub spec: &'a serde_json::Value,
}

pub async fn insert_template<'e, E>(executor: E, t: NewTemplate<'_>) -> DbResult<ProjectTemplate>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"INSERT INTO project_templates (id, org_id, name, description, spec)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING {TEMPLATE_COLS}"#
    );
    let row = sqlx::query(&sql)
        .bind(t.id)
        .bind(t.org_id)
        .bind(t.name)
        .bind(t.description)
        .bind(t.spec)
        .fetch_one(executor)
        .await?;
    Ok(row_to_template(row)?)
}

pub async fn get_template<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<ProjectTemplate>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql =
        format!("SELECT {TEMPLATE_COLS} FROM project_templates WHERE org_id = $1 AND id = $2");
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_template(row)?)
}

pub async fn list_templates<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<ProjectTemplate>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql =
        format!("SELECT {TEMPLATE_COLS} FROM project_templates WHERE org_id = $1 ORDER BY name");
    let rows = sqlx::query(&sql).bind(org_id).fetch_all(executor).await?;
    Ok(rows
        .into_iter()
        .map(row_to_template)
        .collect::<Result<Vec<_>, _>>()?)
}

pub struct TemplatePatch<'a> {
    pub name: Option<&'a str>,
    pub description: Option<Option<&'a str>>,
    pub spec: Option<&'a serde_json::Value>,
}

pub async fn update_template<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    p: TemplatePatch<'_>,
) -> DbResult<ProjectTemplate>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let mut q = sqlx::QueryBuilder::new("UPDATE project_templates SET updated_at = now()");
    if let Some(v) = p.name {
        q.push(", name = ").push_bind(v.to_string());
    }
    if let Some(v) = p.description {
        q.push(", description = ").push_bind(v);
    }
    if let Some(v) = p.spec {
        q.push(", spec = ").push_bind(v);
    }
    q.push(" WHERE org_id = ")
        .push_bind(org_id)
        .push(" AND id = ")
        .push_bind(id)
        .push(" RETURNING ")
        .push(TEMPLATE_COLS);
    let row = q
        .build()
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_template(row)?)
}

pub async fn delete_template<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query("DELETE FROM project_templates WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(executor)
        .await?;
    Ok(res.rows_affected() > 0)
}

// --- phase budgets (Wave 39 lane 4, report C/D) -----------------------------

const BUDGET_COLS: &str = "id, org_id, phase_id, budget_minor, currency, fee_minor, \
     fee_currency, created_at, updated_at";

fn row_to_budget(row: sqlx::postgres::PgRow) -> Result<ProjectPhaseBudget, sqlx::Error> {
    Ok(ProjectPhaseBudget {
        id: row.get("id"),
        org_id: row.get("org_id"),
        phase_id: row.get("phase_id"),
        budget_minor: row.get("budget_minor"),
        currency: row.get("currency"),
        fee_minor: row.get("fee_minor"),
        fee_currency: row.get("fee_currency"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

/// Upserts the phase's billing record (PUT semantics: the route resolves
/// the full record, this writes it). Idempotent per phase.
pub async fn upsert_phase_budget<'e, E>(
    executor: E,
    org_id: Uuid,
    phase_id: Uuid,
    budget: NewPhaseBudget<'_>,
) -> DbResult<ProjectPhaseBudget>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        r#"INSERT INTO project_phase_budgets
             (id, org_id, phase_id, budget_minor, currency, fee_minor, fee_currency)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           ON CONFLICT (phase_id) DO UPDATE
             SET budget_minor = EXCLUDED.budget_minor,
                 currency = EXCLUDED.currency,
                 fee_minor = EXCLUDED.fee_minor,
                 fee_currency = EXCLUDED.fee_currency,
                 updated_at = now()
           RETURNING {BUDGET_COLS}"#
    ))
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(phase_id)
    .bind(budget.budget_minor)
    .bind(budget.currency)
    .bind(budget.fee_minor)
    .bind(budget.fee_currency)
    .fetch_one(executor)
    .await?;
    Ok(row_to_budget(row)?)
}

/// Removes the phase's billing record entirely (used when a patch clears
/// both the budget and the fee — an empty record carries nothing).
pub async fn delete_phase_budget<'e, E>(executor: E, org_id: Uuid, phase_id: Uuid) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result =
        sqlx::query("DELETE FROM project_phase_budgets WHERE org_id = $1 AND phase_id = $2")
            .bind(org_id)
            .bind(phase_id)
            .execute(executor)
            .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn get_phase_budget<'e, E>(
    executor: E,
    org_id: Uuid,
    phase_id: Uuid,
) -> DbResult<Option<ProjectPhaseBudget>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {BUDGET_COLS} FROM project_phase_budgets WHERE org_id = $1 AND phase_id = $2"
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(phase_id)
        .fetch_optional(executor)
        .await?;
    match row {
        Some(row) => Ok(Some(row_to_budget(row)?)),
        None => Ok(None),
    }
}

/// The phase-burn read model: consumed approved billable
/// minutes per phase folded against the phase's budget and CARD-priced
/// at the linked contract's rate card (default rate fallback), computed
/// in one query. `consumed` prices the card rate per work_type bucket
/// exactly like the engine's T&M rule so preview and burn agree.
#[allow(clippy::too_many_arguments)]
pub async fn phase_burn<'e, E>(
    executor: E,
    org_id: Uuid,
    project_id: Uuid,
) -> DbResult<Vec<PhaseBurn>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT pp.id AS phase_id, pp.name AS phase_name, pp.status AS phase_status,
                  b.budget_minor, COALESCE(b.currency, p.currency) AS "currency?",
                  COALESCE(burn.minutes, 0)::bigint AS "consumed_minutes!",
                  COALESCE(burn.consumed_minor, 0)::bigint AS "consumed_minor!"
           FROM project_phases pp
           JOIN projects p ON p.id = pp.project_id
           LEFT JOIN project_phase_budgets b ON b.phase_id = pp.id
           LEFT JOIN (
               SELECT te.phase_id,
                      SUM(te.minutes)::bigint AS minutes,
                      SUM(
                        (te.minutes::bigint *
                          COALESCE(cr.rate_minor, c.hourly_rate_minor, 0) + 30)
                        / 60
                      )::bigint AS consumed_minor
               FROM time_entries te
               JOIN project_phases p2 ON p2.id = te.phase_id
               JOIN projects p3 ON p3.id = p2.project_id
               LEFT JOIN contracts c
                 ON c.id = p3.billing_contract_id AND c.org_id = te.org_id
               LEFT JOIN contract_rates cr
                 ON cr.contract_id = c.id
                AND cr.org_id = te.org_id
                AND cr.work_type IS NOT DISTINCT FROM te.work_type
               WHERE te.org_id = $1
                 AND p3.id = $2
                 AND te.billable
                 AND te.approval_status = 'approved'
               GROUP BY te.phase_id
           ) burn ON burn.phase_id = pp.id
           WHERE pp.org_id = $1 AND pp.project_id = $2
           ORDER BY pp.sort_order, pp.created_at"#,
        org_id,
        project_id,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let budget_minor = row.budget_minor;
            let consumed_minor = row.consumed_minor;
            let alert = match budget_minor {
                Some(budget) if budget > 0 => {
                    let pct = (consumed_minor * 100) / budget;
                    if pct >= 100 {
                        PhaseBurnAlert::Over
                    } else if pct >= 80 {
                        PhaseBurnAlert::Warn
                    } else {
                        PhaseBurnAlert::None
                    }
                }
                _ => PhaseBurnAlert::None,
            };
            PhaseBurn {
                phase_id: row.phase_id,
                phase_name: row.phase_name,
                phase_status: row.phase_status.parse().unwrap_or(PhaseStatus::Pending),
                consumed_minutes: row.consumed_minutes,
                consumed_minor,
                budget_minor,
                currency: row.currency,
                alert,
            }
        })
        .collect())
}

// --- billing releases ----------------------------------------

pub struct NewRelease {
    pub org_id: Uuid,
    pub project_id: Uuid,
    pub phase_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub label: String,
    pub amount_minor: i64,
    pub currency: String,
    pub released_on: chrono::NaiveDate,
    pub source_event_id: Option<Uuid>,
}

/// Stages one release ledger row. Idempotent per (project, phase) via the
/// partial unique index — the consumer's exactly-once seatbelt.
pub async fn insert_release<'e, E>(executor: E, r: &NewRelease) -> DbResult<ProjectBillingRelease>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(
        r#"INSERT INTO project_billing_releases
             (id, org_id, project_id, phase_id, task_id, label, amount_minor,
              currency, released_on, source_event_id, status)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'pending')
           ON CONFLICT (project_id, phase_id) WHERE status <> 'cancelled'
           DO UPDATE SET updated_at = now()
           RETURNING id, org_id, project_id, phase_id, task_id, label,
                     amount_minor, currency, released_on, status,
                     invoice_id, invoice_line_id, created_at, updated_at"#,
    )
    .bind(Uuid::now_v7())
    .bind(r.org_id)
    .bind(r.project_id)
    .bind(r.phase_id)
    .bind(r.task_id)
    .bind(&r.label)
    .bind(r.amount_minor)
    .bind(&r.currency)
    .bind(r.released_on)
    .bind(r.source_event_id)
    .fetch_one(executor)
    .await?;
    release_from_row(row)
}

fn release_from_row(row: sqlx::postgres::PgRow) -> DbResult<ProjectBillingRelease> {
    use sqlx::Row;
    Ok(ProjectBillingRelease {
        id: row.get("id"),
        org_id: row.get("org_id"),
        project_id: row.get("project_id"),
        phase_id: row.get("phase_id"),
        task_id: row.get("task_id"),
        label: row.get("label"),
        amount_minor: row.get("amount_minor"),
        currency: row.get("currency"),
        released_on: row.get("released_on"),
        status: row
            .get::<String, _>("status")
            .parse()
            .unwrap_or(ReleaseStatus::Pending),
        invoice_id: row.get("invoice_id"),
        invoice_line_id: row.get("invoice_line_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

/// The pending releases billable through `contract_id` inside the window:
/// reads the org's projects whose billing link points at this contract,
/// joins their pending releases, and filters the date window. Feeds
/// `load_eval_inputs`.
pub async fn list_pending_releases_for_contract<'e, E>(
    executor: E,
    org_id: Uuid,
    contract_id: Uuid,
    from: chrono::NaiveDate,
    to: chrono::NaiveDate,
) -> DbResult<Vec<ProjectBillingRelease>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
        r#"SELECT r.id, r.org_id, r.project_id, r.phase_id, r.task_id, r.label,
                  r.amount_minor, r.currency, r.released_on, r.status,
                  r.invoice_id, r.invoice_line_id, r.created_at, r.updated_at
           FROM project_billing_releases r
           JOIN projects p ON p.id = r.project_id
           WHERE r.org_id = $1
             AND p.billing_contract_id = $2
             AND r.status = 'pending'
             AND r.released_on >= $3 AND r.released_on <= $4
           ORDER BY r.released_on, r.id"#,
    )
    .bind(org_id)
    .bind(contract_id)
    .bind(from)
    .bind(to)
    .fetch_all(executor)
    .await?;
    rows.into_iter().map(release_from_row).collect()
}

/// Lists a project's releases (client surface for the burn view).
pub async fn list_releases_for_project<'e, E>(
    executor: E,
    org_id: Uuid,
    project_id: Uuid,
) -> DbResult<Vec<ProjectBillingRelease>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
        r#"SELECT id, org_id, project_id, phase_id, task_id, label,
                  amount_minor, currency, released_on, status,
                  invoice_id, invoice_line_id, created_at, updated_at
           FROM project_billing_releases
           WHERE org_id = $1 AND project_id = $2
           ORDER BY released_on, id"#,
    )
    .bind(org_id)
    .bind(project_id)
    .fetch_all(executor)
    .await?;
    rows.into_iter().map(release_from_row).collect()
}

/// Cancels a pending release (admin correction; terminal).
pub async fn cancel_release(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    release_id: Uuid,
) -> DbResult<ProjectBillingRelease> {
    let row = sqlx::query(
        r#"UPDATE project_billing_releases SET status = 'cancelled', updated_at = now()
           WHERE org_id = $1 AND id = $2 AND status = 'pending'
           RETURNING id, org_id, project_id, phase_id, task_id, label,
                     amount_minor, currency, released_on, status,
                     invoice_id, invoice_line_id, created_at, updated_at"#,
    )
    .bind(org_id)
    .bind(release_id)
    .fetch_one(&mut *conn)
    .await?;
    release_from_row(row)
}

/// The run's flip: pending → billed with the consuming invoice and line.
/// Guarded on status = 'pending', so a concurrent run cannot double-flip
/// (the loser reads NotFound and surfaces as a typed conflict).
pub async fn mark_release_billed(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    release_id: Uuid,
    invoice_id: Uuid,
    invoice_line_id: Uuid,
) -> DbResult<()> {
    sqlx::query(
        r#"UPDATE project_billing_releases
              SET status = 'billed', invoice_id = $3, invoice_line_id = $4, updated_at = now()
            WHERE org_id = $1 AND id = $2 AND status = 'pending'"#,
    )
    .bind(org_id)
    .bind(release_id)
    .bind(invoice_id)
    .bind(invoice_line_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

// --- dated Gantt + phase budget rollup (Wave 34 claims close-out, claim 6) --

/// One phase as the Gantt renders it: the planned window joined to its
/// budget burn, so a bar can carry its own overspend label and the planner
/// does not need a second request to see that a phase is over budget.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PhaseGanttRow {
    pub id: Uuid,
    pub name: String,
    pub sort_order: i32,
    /// `PhaseStatus` cast to text in SQL: the column is a Postgres enum, and
    /// the API layer renders the label, so the engine hands back a String
    /// rather than making every caller decode the enum.
    pub status: String,
    pub is_milestone: bool,
    pub start_date: Option<chrono::NaiveDate>,
    pub end_date: Option<chrono::NaiveDate>,
    pub completed_at: Option<DateTime<Utc>>,
    pub budget_minor: Option<i64>,
    pub currency: Option<String>,
    pub consumed_minor: i64,
    pub consumed_minutes: i64,
}

/// The Gantt read model for one project.
///
/// DATED and UNDATED phases are returned separately by the caller rather
/// than interleaved, because they mean different things: a dated phase is a
/// plan, an undated one is a list item. A Gantt that drew an undated phase
/// at "today" would be asserting a date nobody chose, which is the specific
/// dishonesty the nullable columns exist to avoid.
pub async fn phase_gantt<'e, E>(
    executor: E,
    org_id: Uuid,
    project_id: Uuid,
) -> DbResult<Vec<PhaseGanttRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    // This file's convention: a formatted SQL string bound with
    // `sqlx::query`, then an explicit `Row` mapper. The Gantt query needs a
    // subquery and a five-way join, and mapping by hand is what every other
    // multi-join read in here does — `query_as!` would have made this the
    // one query in the file whose columns are invisible at the call site.
    let sql = r#"SELECT pp.id, pp.name, pp.sort_order, pp.status, pp.is_milestone,
                  pp.start_date, pp.end_date, pp.completed_at,
                  b.budget_minor, b.currency,
                  COALESCE(burn.consumed_minor, 0)::bigint AS consumed_minor,
                  COALESCE(burn.minutes, 0)::bigint AS consumed_minutes
           FROM project_phases pp
           LEFT JOIN project_phase_budgets b ON b.phase_id = pp.id
           LEFT JOIN (
               -- Mirrors `phase_burn`'s pricing exactly (contract rate
               -- first, then the contract's hourly rate, +30 for the
               -- write-up allowance, over 60) so a Gantt bar and the budget
               -- pane can never disagree about what a phase has consumed.
               -- Two copies of this arithmetic would drift, and a bar
               -- labelled with one number beside a pane showing another is
               -- worse than no bar.
               SELECT te.phase_id,
                      SUM(te.minutes)::bigint AS minutes,
                      SUM(
                        (te.minutes::bigint *
                          COALESCE(cr.rate_minor, c.hourly_rate_minor, 0) + 30)
                        / 60
                      )::bigint AS consumed_minor
               FROM time_entries te
               JOIN project_phases p2 ON p2.id = te.phase_id
               JOIN projects p3 ON p3.id = p2.project_id
               LEFT JOIN contracts c
                 ON c.id = p3.billing_contract_id AND c.org_id = te.org_id
               LEFT JOIN contract_rates cr
                 ON cr.contract_id = c.id
                AND cr.org_id = te.org_id
                AND cr.work_type IS NOT DISTINCT FROM te.work_type
               WHERE te.org_id = $1
                 AND p3.id = $2
                 AND te.billable
                 AND te.approval_status = 'approved'
               GROUP BY te.phase_id
           ) burn ON burn.phase_id = pp.id
           WHERE pp.org_id = $1 AND pp.project_id = $2
           ORDER BY pp.sort_order, pp.created_at"#;
    let rows = sqlx::query(sql)
        .bind(org_id)
        .bind(project_id)
        .fetch_all(executor)
        .await?;
    rows.into_iter()
        .map(|row| {
            Ok(PhaseGanttRow {
                id: row.get("id"),
                name: row.get("name"),
                sort_order: row.get("sort_order"),
                status: row.get("status"),
                is_milestone: row.get("is_milestone"),
                start_date: row.get("start_date"),
                end_date: row.get("end_date"),
                completed_at: row.get("completed_at"),
                budget_minor: row.get("budget_minor"),
                currency: row.get("currency"),
                consumed_minor: row.get("consumed_minor"),
                consumed_minutes: row.get("consumed_minutes"),
            })
        })
        .collect()
}

// --- project working calendar ------------------------------------------------

const CALENDAR_COLS: &str =
    "project_id, org_id, weekdays, minutes_per_day, note, created_at, updated_at";

fn row_to_calendar(
    row: sqlx::postgres::PgRow,
) -> Result<core_domain::entities::ProjectWorkingCalendar, sqlx::Error> {
    Ok(core_domain::entities::ProjectWorkingCalendar {
        project_id: row.get("project_id"),
        org_id: row.get("org_id"),
        weekdays: row.get("weekdays"),
        minutes_per_day: row.get("minutes_per_day"),
        note: row.get("note"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

/// The project's own working pattern, when it has one. `None` means the
/// organisation's calendar applies.
pub async fn get_working_calendar<'e, E>(
    executor: E,
    org_id: Uuid,
    project_id: Uuid,
) -> DbResult<Option<core_domain::entities::ProjectWorkingCalendar>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {CALENDAR_COLS} FROM project_working_calendars \
         WHERE org_id = $1 AND project_id = $2"
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(project_id)
        .fetch_optional(executor)
        .await?;
    Ok(row.map(row_to_calendar).transpose()?)
}

/// Sets or replaces the project's working pattern. One row per project: a
/// second calendar would make "which pattern wins" a question again.
pub async fn upsert_working_calendar<'e, E>(
    executor: E,
    org_id: Uuid,
    project_id: Uuid,
    weekdays: i16,
    minutes_per_day: i32,
    note: Option<&str>,
) -> DbResult<core_domain::entities::ProjectWorkingCalendar>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"INSERT INTO project_working_calendars
               (project_id, org_id, weekdays, minutes_per_day, note)
           VALUES ($1, $2, $3, $4, $5)
           ON CONFLICT (project_id) DO UPDATE
             SET weekdays = EXCLUDED.weekdays,
                 minutes_per_day = EXCLUDED.minutes_per_day,
                 note = EXCLUDED.note,
                 updated_at = now()
           RETURNING {CALENDAR_COLS}"#
    );
    let row = sqlx::query(&sql)
        .bind(project_id)
        .bind(org_id)
        .bind(weekdays)
        .bind(minutes_per_day)
        .bind(note)
        .fetch_one(executor)
        .await?;
    Ok(row_to_calendar(row)?)
}

/// Drops the project's override so the organisation's calendar applies again.
pub async fn delete_working_calendar<'e, E>(
    executor: E,
    org_id: Uuid,
    project_id: Uuid,
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res =
        sqlx::query("DELETE FROM project_working_calendars WHERE org_id = $1 AND project_id = $2")
            .bind(org_id)
            .bind(project_id)
            .execute(executor)
            .await?;
    Ok(res.rows_affected() > 0)
}

// --- organisation working calendar -------------------------------------------

/// The organisation's working week. `None` means the product's standard
/// Monday-to-Friday, eight-hour day; a row is an organisation that works a
/// different week.
pub async fn get_org_working_calendar<'e, E>(
    executor: E,
    org_id: Uuid,
) -> DbResult<Option<core_domain::entities::ProjectWorkingCalendar>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(
        r#"SELECT org_id, org_id AS project_id, weekdays, minutes_per_day, note,
                  created_at, updated_at
           FROM org_working_calendars WHERE org_id = $1"#,
    )
    .bind(org_id)
    .fetch_optional(executor)
    .await?;
    Ok(row.map(row_to_calendar).transpose()?)
}

/// Sets the organisation's working week. One row per org.
pub async fn upsert_org_working_calendar<'e, E>(
    executor: E,
    org_id: Uuid,
    weekdays: i16,
    minutes_per_day: i32,
    note: Option<&str>,
) -> DbResult<core_domain::entities::ProjectWorkingCalendar>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(
        r#"INSERT INTO org_working_calendars (org_id, weekdays, minutes_per_day, note)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (org_id) DO UPDATE
             SET weekdays = EXCLUDED.weekdays,
                 minutes_per_day = EXCLUDED.minutes_per_day,
                 note = EXCLUDED.note,
                 updated_at = now()
           RETURNING org_id, org_id AS project_id, weekdays, minutes_per_day, note,
                     created_at, updated_at"#,
    )
    .bind(org_id)
    .bind(weekdays)
    .bind(minutes_per_day)
    .bind(note)
    .fetch_one(executor)
    .await?;
    Ok(row_to_calendar(row)?)
}

// --- baselines ----------------------------------------------------------------

/// One captured baseline.
#[derive(Debug, Clone)]
pub struct ProjectBaseline {
    pub id: Uuid,
    pub org_id: Uuid,
    pub project_id: Uuid,
    pub captured_by: Option<Uuid>,
    pub captured_at: chrono::DateTime<chrono::Utc>,
    pub note: Option<String>,
    pub snapshot: serde_json::Value,
    pub dated_tasks: i32,
    pub total_tasks: i32,
}

fn row_to_baseline(row: sqlx::postgres::PgRow) -> Result<ProjectBaseline, sqlx::Error> {
    Ok(ProjectBaseline {
        id: row.get("id"),
        org_id: row.get("org_id"),
        project_id: row.get("project_id"),
        captured_by: row.get("captured_by"),
        captured_at: row.get("captured_at"),
        note: row.get("note"),
        snapshot: row.get("snapshot"),
        dated_tasks: row.get("dated_tasks"),
        total_tasks: row.get("total_tasks"),
    })
}

/// Copies every dated task's start and length into its baseline columns, then
/// records the capture. The two writes belong together: a baseline row whose
/// task columns were not stamped describes a plan nobody had.
pub async fn capture_baseline(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    project_id: Uuid,
    captured_by: Option<Uuid>,
    note: Option<&str>,
) -> DbResult<ProjectBaseline> {
    let tasks: Vec<(Uuid, Option<chrono::DateTime<chrono::Utc>>, Option<i32>)> = sqlx::query_as(
        r#"SELECT id, start_at, duration_minutes FROM project_tasks
           WHERE org_id = $1 AND project_id = $2
           ORDER BY sort_order, id"#,
    )
    .bind(org_id)
    .bind(project_id)
    .fetch_all(&mut *conn)
    .await?;

    sqlx::query(
        r#"UPDATE project_tasks
              SET baseline_start_at = start_at,
                  baseline_duration_minutes = duration_minutes
            WHERE org_id = $1 AND project_id = $2 AND start_at IS NOT NULL"#,
    )
    .bind(org_id)
    .bind(project_id)
    .execute(&mut *conn)
    .await?;

    let dated = tasks.iter().filter(|t| t.1.is_some()).count() as i32;
    let total = tasks.len() as i32;
    let snapshot = serde_json::json!(tasks
        .iter()
        .map(|(id, start, minutes)| serde_json::json!({
            "task_id": id,
            "start_at": start,
            "duration_minutes": minutes,
        }))
        .collect::<Vec<_>>());

    let row = sqlx::query(
        r#"INSERT INTO project_baselines
               (id, org_id, project_id, captured_by, note, snapshot, dated_tasks, total_tasks)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           RETURNING id, org_id, project_id, captured_by, captured_at, note, snapshot,
                     dated_tasks, total_tasks"#,
    )
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(project_id)
    .bind(captured_by)
    .bind(note)
    .bind(&snapshot)
    .bind(dated)
    .bind(total)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row_to_baseline(row)?)
}

/// The most recent baseline, when a project has one.
pub async fn latest_baseline<'e, E>(
    executor: E,
    org_id: Uuid,
    project_id: Uuid,
) -> DbResult<Option<ProjectBaseline>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(
        r#"SELECT id, org_id, project_id, captured_by, captured_at, note, snapshot,
                  dated_tasks, total_tasks
           FROM project_baselines
           WHERE org_id = $1 AND project_id = $2
           ORDER BY captured_at DESC
           LIMIT 1"#,
    )
    .bind(org_id)
    .bind(project_id)
    .fetch_optional(executor)
    .await?;
    Ok(row.map(row_to_baseline).transpose()?)
}

// --- template versions --------------------------------------------------------

/// One version of a project template.
#[derive(Debug, Clone)]
pub struct TemplateVersion {
    pub id: Uuid,
    pub org_id: Uuid,
    pub template_id: Uuid,
    pub version: i32,
    pub spec: serde_json::Value,
    pub status: String,
    pub source: String,
    pub note: Option<String>,
    pub published_by: Option<Uuid>,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

const VERSION_COLS: &str = "id, org_id, template_id, version, spec, status, source, note, \
     published_by, published_at, created_at";

fn row_to_version(row: sqlx::postgres::PgRow) -> Result<TemplateVersion, sqlx::Error> {
    Ok(TemplateVersion {
        id: row.get("id"),
        org_id: row.get("org_id"),
        template_id: row.get("template_id"),
        version: row.get("version"),
        spec: row.get("spec"),
        status: row.get("status"),
        source: row.get("source"),
        note: row.get("note"),
        published_by: row.get("published_by"),
        published_at: row.get("published_at"),
        created_at: row.get("created_at"),
    })
}

/// Every version of one template, newest first.
pub async fn list_template_versions<'e, E>(
    executor: E,
    org_id: Uuid,
    template_id: Uuid,
) -> DbResult<Vec<TemplateVersion>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {VERSION_COLS} FROM project_template_versions \
         WHERE org_id = $1 AND template_id = $2 ORDER BY version DESC"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(template_id)
        .fetch_all(executor)
        .await?;
    Ok(rows
        .into_iter()
        .map(row_to_version)
        .collect::<Result<Vec<_>, _>>()?)
}

/// Appends a draft version built from the template's current spec, or from a
/// spec the caller supplies. Versions are append-only: a published version is
/// never rewritten, which is what lets a fork stay on the plan it was tuned
/// from.
pub async fn append_template_version(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    template_id: Uuid,
    spec: Option<&serde_json::Value>,
    note: Option<&str>,
) -> DbResult<TemplateVersion> {
    let sql = format!(
        r#"INSERT INTO project_template_versions
               (id, org_id, template_id, version, spec, status, source, note)
           SELECT $1, $2, $3,
                  COALESCE((SELECT MAX(version) FROM project_template_versions
                             WHERE template_id = $3), 0) + 1,
                  COALESCE($4, (SELECT spec FROM project_templates
                                 WHERE org_id = $2 AND id = $3)),
                  'draft', 'org', $5
           RETURNING {VERSION_COLS}"#
    );
    let row = sqlx::query(&sql)
        .bind(Uuid::now_v7())
        .bind(org_id)
        .bind(template_id)
        .bind(spec)
        .bind(note)
        .fetch_one(&mut *executor)
        .await?;
    Ok(row_to_version(row)?)
}

/// Publishes a version: it becomes the template's current document and the one
/// an instantiation reads.
pub async fn publish_template_version(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    template_id: Uuid,
    version: i32,
    published_by: Option<Uuid>,
) -> DbResult<TemplateVersion> {
    let sql = format!(
        r#"UPDATE project_template_versions
              SET status = 'published', published_by = $4, published_at = now()
            WHERE org_id = $1 AND template_id = $2 AND version = $3
            RETURNING {VERSION_COLS}"#
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(template_id)
        .bind(version)
        .bind(published_by)
        .fetch_optional(&mut *executor)
        .await?
        .ok_or(DbError::NotFound)?;
    let published = row_to_version(row)?;
    sqlx::query(
        r#"UPDATE project_templates
              SET current_version_id = $3, spec = $4, updated_at = now()
            WHERE org_id = $1 AND id = $2"#,
    )
    .bind(org_id)
    .bind(template_id)
    .bind(published.id)
    .bind(&published.spec)
    .execute(&mut *executor)
    .await?;
    Ok(published)
}

/// The version an instantiation reads: the template's current one.
pub async fn current_template_version<'e, E>(
    executor: E,
    org_id: Uuid,
    template_id: Uuid,
) -> DbResult<Option<TemplateVersion>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"SELECT {VERSION_COLS} FROM project_template_versions
            WHERE org_id = $1 AND id = (
                SELECT current_version_id FROM project_templates
                 WHERE org_id = $1 AND id = $2
            )"#
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(template_id)
        .fetch_optional(executor)
        .await?;
    Ok(row.map(row_to_version).transpose()?)
}

/// Copies a template (and its current version) into a new org-owned template
/// whose first version records where it came from.
pub async fn fork_template(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    template_id: Uuid,
    name: &str,
) -> DbResult<core_domain::entities::ProjectTemplate> {
    let source = current_template_version(&mut *executor, org_id, template_id)
        .await?
        .ok_or(DbError::NotFound)?;
    let new_id = Uuid::now_v7();
    let forked = sqlx::query(
        r#"INSERT INTO project_templates
               (id, org_id, name, spec, forked_from_version_id, is_shipped)
           VALUES ($1, $2, $3, $4, $5, false)
           RETURNING id, org_id, name, description, spec, created_at, updated_at"#,
    )
    .bind(new_id)
    .bind(org_id)
    .bind(name)
    .bind(&source.spec)
    .bind(source.id)
    .fetch_one(&mut *executor)
    .await?;
    let template = project_template_from_row(forked)?;
    let version = append_template_version(
        &mut *executor,
        org_id,
        new_id,
        Some(&source.spec),
        Some("Forked"),
    )
    .await?;
    publish_template_version(&mut *executor, org_id, new_id, version.version, None).await?;
    Ok(template)
}

fn project_template_from_row(
    row: sqlx::postgres::PgRow,
) -> Result<core_domain::entities::ProjectTemplate, sqlx::Error> {
    Ok(core_domain::entities::ProjectTemplate {
        id: row.get("id"),
        org_id: row.get("org_id"),
        name: row.get("name"),
        description: row.get("description"),
        spec: row.get("spec"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

// --- calendar zones -----------------------------------------------------------

/// A stretch of the calendar that constrains where work may sit.
#[derive(Debug, Clone)]
pub struct CalendarZone {
    pub id: Uuid,
    pub org_id: Uuid,
    pub project_id: Uuid,
    pub kind: String,
    pub label: String,
    pub starts_on: chrono::NaiveDate,
    pub ends_on: chrono::NaiveDate,
    pub note: Option<String>,
}

const ZONE_COLS: &str = "id, org_id, project_id, kind, label, starts_on, ends_on, note";

fn row_to_zone(row: sqlx::postgres::PgRow) -> Result<CalendarZone, sqlx::Error> {
    Ok(CalendarZone {
        id: row.get("id"),
        org_id: row.get("org_id"),
        project_id: row.get("project_id"),
        kind: row.get("kind"),
        label: row.get("label"),
        starts_on: row.get("starts_on"),
        ends_on: row.get("ends_on"),
        note: row.get("note"),
    })
}

/// Zones on a project, soonest first.
pub async fn list_calendar_zones<'e, E>(
    executor: E,
    org_id: Uuid,
    project_id: Uuid,
) -> DbResult<Vec<CalendarZone>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {ZONE_COLS} FROM project_calendar_zones \
         WHERE org_id = $1 AND project_id = $2 ORDER BY starts_on"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(project_id)
        .fetch_all(executor)
        .await?;
    Ok(rows
        .into_iter()
        .map(row_to_zone)
        .collect::<Result<Vec<_>, _>>()?)
}

/// Adds a zone. The route validates the range; this refuses nothing a CHECK
/// already refuses.
pub struct NewCalendarZone<'a> {
    pub org_id: Uuid,
    pub project_id: Uuid,
    pub kind: &'a str,
    pub label: &'a str,
    pub starts_on: chrono::NaiveDate,
    pub ends_on: chrono::NaiveDate,
    pub note: Option<&'a str>,
}

pub async fn insert_calendar_zone<'e, E>(
    executor: E,
    zone: NewCalendarZone<'_>,
) -> DbResult<CalendarZone>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"INSERT INTO project_calendar_zones
               (id, org_id, project_id, kind, label, starts_on, ends_on, note)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           RETURNING {ZONE_COLS}"#
    );
    let row = sqlx::query(&sql)
        .bind(Uuid::now_v7())
        .bind(zone.org_id)
        .bind(zone.project_id)
        .bind(zone.kind)
        .bind(zone.label)
        .bind(zone.starts_on)
        .bind(zone.ends_on)
        .bind(zone.note)
        .fetch_one(executor)
        .await?;
    Ok(row_to_zone(row)?)
}

/// Removes a zone.
pub async fn delete_calendar_zone<'e, E>(
    executor: E,
    org_id: Uuid,
    project_id: Uuid,
    id: Uuid,
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query(
        "DELETE FROM project_calendar_zones WHERE org_id = $1 AND project_id = $2 AND id = $3",
    )
    .bind(org_id)
    .bind(project_id)
    .bind(id)
    .execute(executor)
    .await?;
    Ok(res.rows_affected() > 0)
}

// --- cross-project commitments ------------------------------------------------

/// Someone else's dated work that would sit at the same time as this work.
#[derive(Debug, Clone)]
pub struct OverlappingAssignment {
    pub task_id: Uuid,
    pub project_id: Uuid,
    pub project_name: String,
    pub task_title: String,
    pub assignee_id: Option<Uuid>,
    pub start_at: chrono::DateTime<chrono::Utc>,
    pub duration_minutes: i32,
}

/// Dated tasks in OTHER projects assigned to the given people, inside a window.
///
/// This is the read behind "collides with another commitment": a coordinator
/// moving a bar needs to know that the technician is already booked elsewhere,
/// and that fact lives in a different project's rows.
pub async fn list_overlapping_assignments<'e, E>(
    executor: E,
    org_id: Uuid,
    exclude_project_id: Uuid,
    assignee_ids: &[Uuid],
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> DbResult<Vec<OverlappingAssignment>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    if assignee_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        r#"SELECT t.id, t.project_id, p.name AS project_name, t.title, t.assignee_id,
                  t.start_at, t.duration_minutes
             FROM project_tasks t
             JOIN projects p ON p.id = t.project_id
            WHERE t.org_id = $1
              AND t.project_id <> $2
              AND t.assignee_id = ANY($3)
              AND t.start_at IS NOT NULL
              AND t.duration_minutes IS NOT NULL
              AND t.status NOT IN ('completed','cancelled')
              -- A coarse window filter; the caller compares working minutes.
              AND t.start_at < $5
              AND t.start_at + make_interval(mins => t.duration_minutes) > $4
            ORDER BY t.start_at
            LIMIT 50"#,
    )
    .bind(org_id)
    .bind(exclude_project_id)
    .bind(assignee_ids)
    .bind(from)
    .bind(to)
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| OverlappingAssignment {
            task_id: row.get("id"),
            project_id: row.get("project_id"),
            project_name: row.get("project_name"),
            task_title: row.get("title"),
            assignee_id: row.get("assignee_id"),
            start_at: row.get("start_at"),
            duration_minutes: row.get("duration_minutes"),
        })
        .collect())
}
