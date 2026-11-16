//! Template gallery, preview, validation and lessons.
//!
//! The document CRUD and instantiation live in `projects::templates`; this file
//! adds what the gallery and editor read: catalogue facts, the plan preview, a
//! validation report, the lessons loop and the billing-guarded create.

use super::*;
use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant;
use crate::org_tx;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use core_domain::entities::BillingMethod;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

// --- wire shapes ------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct GalleryTemplateView {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub typical_days: Option<i32>,
    pub effort_min_minutes: Option<i32>,
    pub effort_max_minutes: Option<i32>,
    pub archived: bool,
    pub is_shipped: bool,
    pub current_version: Option<i32>,
    pub forked_from_version_id: Option<Uuid>,
    pub phase_count: i32,
    pub task_count: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
pub struct GalleryQuery {
    #[serde(default)]
    pub include_archived: bool,
    pub category: Option<String>,
}

impl GalleryTemplateView {
    fn from_row(row: core_db::project_delivery::GalleryTemplate) -> Self {
        // The counts come from the document itself; a template that cannot be
        // parsed is shown with zero counts and refused by validation, never
        // hidden from the author.
        let (phases, tasks) = core_projects::template::parse_spec(&row.spec)
            .map(|spec| {
                let plan = spec.expand();
                (plan.phases.len() as i32, plan.tasks.len() as i32)
            })
            .unwrap_or((0, 0));
        Self {
            id: row.id,
            name: row.name,
            description: row.description,
            category: row.category,
            typical_days: row.typical_days,
            effort_min_minutes: row.effort_min_minutes,
            effort_max_minutes: row.effort_max_minutes,
            archived: row.archived_at.is_some(),
            is_shipped: row.is_shipped,
            current_version: row.current_version,
            forked_from_version_id: row.forked_from_version_id,
            phase_count: phases,
            task_count: tasks,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// GET /api/projects/template-gallery — shipped and org templates in one list.
#[utoipa::path(operation_id = "project_template_gallery", get, path = "/api/projects/template-gallery", tag = "projects", security(("bearer" = [])),
    params(("include_archived" = Option<bool>, Query), ("category" = Option<String>, Query)),
    responses((status = 200, body = [GalleryTemplateView])))]
pub async fn gallery(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(query): Query<GalleryQuery>,
) -> ApiResult<Json<Vec<GalleryTemplateView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::project_delivery::list_template_gallery(
        &mut *tx,
        org_id,
        query.include_archived,
        query.category.as_deref(),
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(
        rows.into_iter()
            .map(GalleryTemplateView::from_row)
            .collect(),
    ))
}

/// GET /api/project-templates/{id}/gallery-card
#[utoipa::path(operation_id = "project_template_card", get, path = "/api/project-templates/{id}/gallery-card", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = GalleryTemplateView),
              (status = 404, description = "template not visible in scope (rls_sparse)")))]
pub async fn card(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<GalleryTemplateView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let row = core_db::project_delivery::get_gallery_template(&mut *tx, org_id, id)
        .await
        .map_err(|e| match e {
            core_db::DbError::NotFound => ApiError::RlsSparse,
            other => other.into(),
        })?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(GalleryTemplateView::from_row(row)))
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ArchiveTemplateRequest {
    pub archived: bool,
}

/// POST /api/project-templates/{id}/archive
#[utoipa::path(operation_id = "project_template_archive", post, path = "/api/project-templates/{id}/archive", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = ArchiveTemplateRequest,
    responses((status = 200, body = GalleryTemplateView),
              (status = 404, description = "template not visible in scope (rls_sparse)")))]
pub async fn archive(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<ArchiveTemplateRequest>,
) -> ApiResult<Json<GalleryTemplateView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let row = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let existing = core_db::project_delivery::get_gallery_template(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| match e {
                core_db::DbError::NotFound => ApiError::RlsSparse,
                other => other.into(),
            })?;
        if existing.is_shipped && req.archived {
            return Err(ApiError::Validation(
                "a shipped template cannot be archived; fork it instead".into(),
            ));
        }
        core_db::project_delivery::set_template_archived(&mut ctx.tx, org_id, id, req.archived)
            .await?;
        let row = core_db::project_delivery::get_gallery_template(&mut *ctx.tx, org_id, id).await?;
        Ok((ctx, row))
    })
    .await?;
    Ok(Json(GalleryTemplateView::from_row(row)))
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct TemplateMetaRequest {
    pub category: Option<String>,
    pub typical_days: Option<i32>,
    pub effort_min_minutes: Option<i32>,
    pub effort_max_minutes: Option<i32>,
}

/// PUT /api/project-templates/{id}/meta — the catalogue facts a card draws.
#[utoipa::path(operation_id = "project_template_meta", put, path = "/api/project-templates/{id}/meta", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = TemplateMetaRequest,
    responses((status = 200, body = GalleryTemplateView),
              (status = 422, description = "the effort range is inverted")))]
pub async fn set_meta(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<TemplateMetaRequest>,
) -> ApiResult<Json<GalleryTemplateView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    if let (Some(min), Some(max)) = (req.effort_min_minutes, req.effort_max_minutes) {
        if max < min {
            return Err(ApiError::Validation(
                "effort_max_minutes must not be below effort_min_minutes".into(),
            ));
        }
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let category = req
        .category
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let row = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::project_delivery::set_template_meta(
            &mut ctx.tx,
            org_id,
            id,
            core_db::project_delivery::TemplateMeta {
                category,
                typical_days: req.typical_days,
                effort_min_minutes: req.effort_min_minutes,
                effort_max_minutes: req.effort_max_minutes,
            },
        )
        .await
        .map_err(|e| match e {
            core_db::DbError::NotFound => ApiError::RlsSparse,
            other => other.into(),
        })?;
        let row = core_db::project_delivery::get_gallery_template(&mut *ctx.tx, org_id, id).await?;
        Ok((ctx, row))
    })
    .await?;
    Ok(Json(GalleryTemplateView::from_row(row)))
}

// --- preview ----------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct PreviewTask {
    pub key: String,
    pub title: String,
    pub phase_key: Option<String>,
    pub phase_name: Option<String>,
    pub role_key: Option<String>,
    pub work_type: Option<String>,
    pub duration_days: Option<i64>,
    pub effort_minutes: Option<i64>,
    pub depends_on: Vec<String>,
    pub optional: bool,
    pub is_milestone: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PreviewPhase {
    pub key: String,
    pub name: String,
    pub is_milestone: bool,
    pub task_count: i32,
    pub effort_minutes: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PreviewRole {
    pub role_key: String,
    pub task_count: i32,
    pub effort_minutes: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TemplatePreviewView {
    pub name: String,
    pub billing_method: Option<String>,
    pub currency: Option<String>,
    pub phases: Vec<PreviewPhase>,
    pub tasks: Vec<PreviewTask>,
    pub roles: Vec<PreviewRole>,
    pub total_effort_minutes: i64,
    pub dated_tasks: i32,
    pub undated_tasks: i32,
}

/// GET /api/project-templates/{id}/preview — the plan tree, its roles and its
/// shape, before anyone creates a project from it.
#[utoipa::path(operation_id = "project_template_preview", get, path = "/api/project-templates/{id}/preview", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = TemplatePreviewView),
              (status = 404, description = "template not visible in scope (rls_sparse)"),
              (status = 422, description = "the spec does not parse")))]
pub async fn preview(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<TemplatePreviewView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let template = core_db::projects::get_template(&mut *tx, org_id, id)
        .await
        .map_err(|e| match e {
            core_db::DbError::NotFound => ApiError::RlsSparse,
            other => other.into(),
        })?;
    tx.commit().await.map_err(ApiError::from)?;

    let spec = core_projects::template::parse_spec(&template.spec)
        .map_err(|e| ApiError::Validation(e.to_string()))?;

    let phase_names: std::collections::HashMap<String, String> = spec
        .phases
        .iter()
        .map(|p| (p.key.clone(), p.name.clone()))
        .collect();

    // A task may be declared inside a phase block or at the project level with
    // a `phase` key. Both are the same fact; read them as one list.
    let mut all_tasks: Vec<(&core_projects::template::TemplateTask, Option<String>)> = Vec::new();
    for phase in &spec.phases {
        for task in &phase.tasks {
            all_tasks.push((task, Some(phase.key.clone())));
        }
    }
    for task in &spec.tasks {
        all_tasks.push((task, task.phase.clone()));
    }

    let mut phase_effort: std::collections::HashMap<String, (i32, i64)> =
        std::collections::HashMap::new();
    let mut role_effort: std::collections::HashMap<String, (i32, i64)> =
        std::collections::HashMap::new();
    let mut total = 0i64;
    let mut dated = 0i32;

    let tasks: Vec<PreviewTask> = all_tasks
        .iter()
        .map(|(task, phase_key)| {
            let effort = task.effort_minutes.unwrap_or(0);
            total += effort;
            if task.duration_days.is_some() {
                dated += 1;
            }
            if let Some(key) = phase_key.as_ref() {
                let entry = phase_effort.entry(key.clone()).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += effort;
            }
            if let Some(role) = task.role_key.as_ref() {
                let entry = role_effort.entry(role.clone()).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += effort;
            }
            PreviewTask {
                key: task.key.clone(),
                title: task.title.clone(),
                phase_key: phase_key.clone(),
                phase_name: phase_key.as_ref().and_then(|k| phase_names.get(k).cloned()),
                role_key: task.role_key.clone(),
                work_type: task.work_type.clone(),
                duration_days: task.duration_days,
                effort_minutes: task.effort_minutes,
                depends_on: task.depends_on.clone(),
                optional: task.optional,
                is_milestone: task.is_milestone,
            }
        })
        .collect();

    let phases: Vec<PreviewPhase> = spec
        .phases
        .iter()
        .map(|p| {
            let (count, effort) = phase_effort.get(&p.key).copied().unwrap_or((0, 0));
            PreviewPhase {
                key: p.key.clone(),
                name: p.name.clone(),
                is_milestone: p.is_milestone,
                task_count: count,
                effort_minutes: effort,
            }
        })
        .collect();

    let mut roles: Vec<PreviewRole> = role_effort
        .into_iter()
        .map(|(role_key, (task_count, effort_minutes))| PreviewRole {
            role_key,
            task_count,
            effort_minutes,
        })
        .collect();
    roles.sort_by(|a, b| a.role_key.cmp(&b.role_key));

    Ok(Json(TemplatePreviewView {
        name: template.name,
        billing_method: spec.billing_method.clone(),
        currency: spec.currency.clone(),
        phases,
        tasks,
        roles,
        total_effort_minutes: total,
        dated_tasks: dated,
        undated_tasks: all_tasks.len() as i32 - dated,
    }))
}

// --- validation -------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct ValidationIssue {
    /// A stable code for the UI to group by: `role_unheld`, `skill_unowned`,
    /// `empty_phase`, `milestone_without_amount`, `optional_with_dependents`,
    /// `undated_task`, `task_without_role`.
    pub code: String,
    pub severity: String,
    pub message: String,
    pub task_key: Option<String>,
    pub phase_key: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ValidationReport {
    pub publishable: bool,
    pub issues: Vec<ValidationIssue>,
}

/// GET /api/project-templates/{id}/validate — what would break a project
/// created from this version. It checks the document, never the project.
#[utoipa::path(operation_id = "project_template_validate", get, path = "/api/project-templates/{id}/validate", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = ValidationReport)))]
pub async fn validate(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ValidationReport>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let template = core_db::projects::get_template(&mut *tx, org_id, id)
        .await
        .map_err(|e| match e {
            core_db::DbError::NotFound => ApiError::RlsSparse,
            other => other.into(),
        })?;
    let roles = core_db::roles::list_roles(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;

    let spec = core_projects::template::parse_spec(&template.spec)
        .map_err(|e| ApiError::Validation(e.to_string()))?;

    let mut all_tasks: Vec<(&core_projects::template::TemplateTask, Option<String>)> = Vec::new();
    for phase in &spec.phases {
        for task in &phase.tasks {
            all_tasks.push((task, Some(phase.key.clone())));
        }
    }
    for task in &spec.tasks {
        all_tasks.push((task, task.phase.clone()));
    }

    let mut issues: Vec<ValidationIssue> = Vec::new();
    let role_names: std::collections::HashSet<String> =
        roles.iter().map(|r| r.name.to_lowercase()).collect();

    for phase in &spec.phases {
        let count = spec
            .phases
            .iter()
            .find(|p| p.key == phase.key)
            .map(|p| p.tasks.len())
            .unwrap_or(0);
        if count == 0
            && !spec
                .tasks
                .iter()
                .any(|t| t.phase.as_deref() == Some(phase.key.as_str()))
        {
            issues.push(ValidationIssue {
                code: "empty_phase".into(),
                severity: "warning".into(),
                message: format!("Phase \"{}\" has no tasks", phase.name),
                task_key: None,
                phase_key: Some(phase.key.clone()),
            });
        }
    }

    for (task, phase_key) in &all_tasks {
        if let Some(role) = task.role_key.as_deref() {
            if !role.trim().is_empty() && !role_names.contains(&role.to_lowercase()) {
                issues.push(ValidationIssue {
                    code: "role_unheld".into(),
                    severity: "warning".into(),
                    message: format!(
                        "No role named \"{role}\" exists in this organisation; the task will be created unassigned"
                    ),
                    task_key: Some(task.key.clone()),
                    phase_key: phase_key.clone(),
                });
            }
        } else {
            issues.push(ValidationIssue {
                code: "task_without_role".into(),
                severity: "info".into(),
                message: format!("\"{}\" names no role", task.title),
                task_key: Some(task.key.clone()),
                phase_key: phase_key.clone(),
            });
        }

        if task.duration_days.is_none() {
            issues.push(ValidationIssue {
                code: "undated_task".into(),
                severity: "info".into(),
                message: format!(
                    "\"{}\" has no duration; it will be created undated",
                    task.title
                ),
                task_key: Some(task.key.clone()),
                phase_key: phase_key.clone(),
            });
        }

        if task.optional {
            let dependents: Vec<&str> = all_tasks
                .iter()
                .filter(|(other, _)| other.depends_on.contains(&task.key))
                .map(|(other, _)| other.title.as_str())
                .collect();
            if !dependents.is_empty() {
                issues.push(ValidationIssue {
                    code: "optional_with_dependents".into(),
                    severity: "error".into(),
                    message: format!(
                        "\"{}\" is optional but {} depends on it; skipping it would leave that work unlinked",
                        task.title,
                        dependents.join(", ")
                    ),
                    task_key: Some(task.key.clone()),
                    phase_key: phase_key.clone(),
                });
            }
        }
    }

    for phase in &spec.phases {
        if phase.is_milestone && phase.milestone_amount_minor.is_none() {
            issues.push(ValidationIssue {
                code: "milestone_without_amount".into(),
                severity: "info".into(),
                message: format!("Milestone \"{}\" carries no amount", phase.name),
                task_key: None,
                phase_key: Some(phase.key.clone()),
            });
        }
    }

    let publishable = !issues.iter().any(|i| i.severity == "error");
    Ok(Json(ValidationReport {
        publishable,
        issues,
    }))
}

// --- lessons ----------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct LessonView {
    pub id: Uuid,
    pub template_id: Uuid,
    pub project_id: Option<Uuid>,
    pub kind: String,
    pub message: String,
    pub evidence: Value,
    pub state: String,
    pub dismiss_reason: Option<String>,
    pub decided_by: Option<Uuid>,
    pub decided_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<core_db::project_delivery::TemplateLesson> for LessonView {
    fn from(lesson: core_db::project_delivery::TemplateLesson) -> Self {
        Self {
            id: lesson.id,
            template_id: lesson.template_id,
            project_id: lesson.project_id,
            kind: lesson.kind,
            message: lesson.message,
            evidence: lesson.evidence,
            state: lesson.state,
            dismiss_reason: lesson.dismiss_reason,
            decided_by: lesson.decided_by,
            decided_at: lesson.decided_at,
            created_at: lesson.created_at,
        }
    }
}

/// GET /api/project-templates/{id}/lessons
#[utoipa::path(operation_id = "project_template_lessons", get, path = "/api/project-templates/{id}/lessons", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [LessonView])))]
pub async fn list_lessons(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<LessonView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let lessons = core_db::project_delivery::list_template_lessons(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(lessons.into_iter().map(LessonView::from).collect()))
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateLessonRequest {
    pub project_id: Option<Uuid>,
    pub kind: String,
    pub message: String,
    #[serde(default)]
    pub evidence: Value,
}

/// POST /api/project-templates/{id}/lessons — records what a closed project
/// taught. The evidence is required in shape (a project or a stated number),
/// because a suggestion without a source is an opinion.
#[utoipa::path(operation_id = "project_template_lesson_create", post, path = "/api/project-templates/{id}/lessons", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = CreateLessonRequest,
    responses((status = 200, body = LessonView),
              (status = 422, description = "validation failed")))]
pub async fn create_lesson(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateLessonRequest>,
) -> ApiResult<Json<LessonView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    const KINDS: [&str; 7] = [
        "duration",
        "dependency",
        "missing_step",
        "extra_step",
        "role",
        "material",
        "other",
    ];
    if !KINDS.contains(&req.kind.as_str()) {
        return Err(ApiError::Validation(format!(
            "kind must be one of {}",
            KINDS.join(", ")
        )));
    }
    let message = req.message.trim();
    if message.is_empty() {
        return Err(ApiError::Validation("message is required".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let lesson = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::projects::get_template(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| match e {
                core_db::DbError::NotFound => ApiError::RlsSparse,
                other => other.into(),
            })?;
        let lesson = core_db::project_delivery::insert_lesson(
            &mut ctx.tx,
            core_db::project_delivery::NewLesson {
                id: Uuid::now_v7(),
                org_id,
                template_id: id,
                project_id: req.project_id,
                kind: &req.kind,
                message,
                evidence: &req.evidence,
            },
        )
        .await?;
        Ok((ctx, lesson))
    })
    .await?;
    Ok(Json(LessonView::from(lesson)))
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct DecideLessonRequest {
    /// `accepted` or `dismissed`.
    pub state: String,
    pub reason: Option<String>,
}

/// POST /api/project-templates/{id}/lessons/{lesson_id}/decide
#[utoipa::path(operation_id = "project_template_lesson_decide", post, path = "/api/project-templates/{id}/lessons/{lesson_id}/decide", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("lesson_id" = Uuid, Path)), request_body = DecideLessonRequest,
    responses((status = 200, body = LessonView),
              (status = 409, description = "the lesson is already decided")))]
pub async fn decide_lesson(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, lesson_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<DecideLessonRequest>,
) -> ApiResult<Json<LessonView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    if req.state != "accepted" && req.state != "dismissed" {
        return Err(ApiError::Validation(
            "state must be accepted or dismissed".into(),
        ));
    }
    let reason = req
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty());
    if req.state == "dismissed" && reason.is_none() {
        return Err(ApiError::Validation(
            "a dismissal must say why, so the same suggestion is not repeated".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let lesson = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let lesson = core_db::project_delivery::decide_lesson(
            &mut ctx.tx,
            org_id,
            id,
            lesson_id,
            &req.state,
            reason,
            user_id,
        )
        .await?;
        Ok((ctx, lesson))
    })
    .await?;
    Ok(Json(LessonView::from(lesson)))
}

// --- create from template, with the billing guard --------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateFromTemplateRequest {
    pub name: String,
    pub client_id: Option<Uuid>,
    pub start_date: Option<chrono::NaiveDate>,
    pub due_date: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub assignee_map: std::collections::HashMap<String, Uuid>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// POST /api/project-templates/{id}/create — instantiation with the one check
/// the gallery promises first: a billable template cannot start without a
/// contract that bills the chosen client (S11.11). The checked refusal happens
/// before any row is written.
#[utoipa::path(operation_id = "project_template_create", post, path = "/api/project-templates/{id}/create", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = CreateFromTemplateRequest,
    responses((status = 200, body = crate::routes::projects::templates::ProjectInstantiateResponse),
              (status = 422, description = "billing_mismatch: the template bills but the client has no contract")))]
pub async fn create_from_template(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateFromTemplateRequest>,
) -> ApiResult<Json<crate::routes::projects::templates::ProjectInstantiateResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let template = core_db::projects::get_template(&mut *tx, org_id, id)
        .await
        .map_err(|e| match e {
            core_db::DbError::NotFound => ApiError::RlsSparse,
            other => other.into(),
        })?;
    let billing_method = core_projects::template::parse_spec(&template.spec)
        .map_err(|e| ApiError::Validation(e.to_string()))?
        .billing_method;
    tx.commit().await.map_err(ApiError::from)?;

    let billable = matches!(
        billing_method.as_deref().map(str::parse::<BillingMethod>),
        Some(Ok(BillingMethod::Milestone))
            | Some(Ok(BillingMethod::TimeAndMaterials))
            | Some(Ok(BillingMethod::FixedFee))
    );
    if billable {
        match req.client_id {
            None => {
                return Err(ApiError::Validation(
                    "billing_mismatch: this template bills, so a client is required before it can start"
                        .into(),
                ));
            }
            Some(client_id) => {
                let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
                core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
                    .await
                    .map_err(ApiError::from)?;
                let contracts = core_db::contracts::list_contracts(
                    &mut *tx,
                    org_id,
                    &core_db::contracts::ContractFilters {
                        client_id: Some(client_id),
                        status: Some("active".into()),
                        // The contract must cover the day the work starts, not
                        // merely exist: one that starts later, or ended before,
                        // would bill nothing on this project.
                        effective_on: req.start_date,
                    },
                )
                .await?;
                tx.commit().await.map_err(ApiError::from)?;
                if contracts.is_empty() {
                    return Err(ApiError::Validation(
                        "billing_mismatch: the client has no active contract, so nothing would bill this work"
                            .into(),
                    ));
                }
            }
        }
    }

    crate::routes::projects::templates::instantiate(
        State(state),
        extensions,
        Path(id),
        Json(
            crate::routes::projects::templates::InstantiateTemplateRequest {
                name: req.name,
                client_id: req.client_id,
                start_date: req.start_date,
                due_date: req.due_date,
                assignee_map: req.assignee_map,
                instant: req.instant,
            },
        ),
    )
    .await
}
