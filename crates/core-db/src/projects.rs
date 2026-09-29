//! Projects-line queries (migration 0030): projects → phases → tasks,
//! task dependencies, saved views, and templates. Every function is
//! org-scoped; rows are RLS-filtered in transactions bound via `set_org`.
//! Dynamic filters and tri-state patches use runtime-checked queries
//! (`tickets.rs` convention); mappers parse status enums with typed
//! decode errors.

use crate::{DbError, DbResult};
use core_domain::entities::{
    BillingMethod, PhaseBurn, PhaseBurnAlert, PhaseStatus, Project, ProjectBillingRelease,
    ProjectPhase, ProjectPhaseBudget, ProjectStatus, ProjectTask, ProjectTemplate, ProjectView,
    ReleaseStatus, TaskStatus,
};
use sqlx::{Postgres, Row};
use uuid::Uuid;

const PROJECT_COLS: &str = "id, org_id, client_id, name, description, status, billing_method, \
     budget_minor, currency, start_date, due_date, billing_contract_id, created_at, updated_at";
const PHASE_COLS: &str = "id, org_id, project_id, name, description, sort_order, status, \
     is_milestone, milestone_amount_minor, milestone_currency, completed_at, created_at, updated_at";
const TASK_COLS: &str = "id, org_id, project_id, phase_id, title, description, status, \
     assignee_id, sort_order, is_milestone, milestone_amount_minor, milestone_currency, \
     due_at, completed_at, created_at, updated_at";
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
                                       is_milestone, milestone_amount_minor, milestone_currency)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
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
}

pub async fn insert_task<'e, E>(executor: E, t: NewTask<'_>) -> DbResult<ProjectTask>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"INSERT INTO project_tasks (id, org_id, project_id, phase_id, title, description,
                                      assignee_id, sort_order, is_milestone,
                                      milestone_amount_minor, milestone_currency, due_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
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

/// The phase-burn read model (report P3): consumed approved billable
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
                  COALESCE(burn.minutes, 0) AS consumed_minutes,
                  COALESCE(burn.consumed_minor, 0) AS consumed_minor
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

// --- billing releases (report P1/P4) ----------------------------------------

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
           ON CONFLICT (project_id, phase_id, status) WHERE status <> 'cancelled'
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
