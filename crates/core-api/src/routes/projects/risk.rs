use super::*;

// --- baselines and risk ---------------------------------------------------------------

/// One captured baseline as a surface reads it.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct BaselineView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub captured_by: Option<Uuid>,
    pub captured_at: chrono::DateTime<chrono::Utc>,
    pub note: Option<String>,
    /// How many tasks carried a date when the baseline was taken, and how many
    /// the project held in total. A baseline with little dated work is a weak
    /// promise, and the pair says so without a judgement.
    pub dated_tasks: i32,
    pub total_tasks: i32,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct BaselineResponse {
    pub baseline: Option<BaselineView>,
}

impl From<core_db::projects::ProjectBaseline> for BaselineView {
    fn from(b: core_db::projects::ProjectBaseline) -> Self {
        Self {
            id: b.id,
            captured_by: b.captured_by,
            captured_at: b.captured_at,
            note: b.note,
            dated_tasks: b.dated_tasks,
            total_tasks: b.total_tasks,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CaptureBaselineRequest {
    pub note: Option<String>,
}

/// POST /api/projects/{id}/baseline — records what the plan says now.
///
/// Every dated task's start and length are copied into its baseline columns and
/// the capture is recorded as an event, so a later slip has something to be
/// late against. Undated tasks stay undated in the baseline too: the capture
/// records the plan, it does not invent one.
#[utoipa::path(operation_id = "projects_capture_baseline", post, path = "/api/projects/{id}/baseline", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = CaptureBaselineRequest,
    responses((status = 200, body = BaselineResponse),
              (status = 404, description = "project not visible in scope (rls_sparse)")))]
pub async fn capture_baseline(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CaptureBaselineRequest>,
) -> ApiResult<Json<BaselineResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let note = req.note.as_deref().map(str::trim).filter(|n| !n.is_empty());

    let baseline = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::projects::get_project(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let baseline =
            core_db::projects::capture_baseline(&mut ctx.tx, org_id, id, Some(user_id), note)
                .await?;
        Ok((ctx, baseline))
    })
    .await?;

    Ok(Json(BaselineResponse {
        baseline: Some(baseline.into()),
    }))
}

/// GET /api/projects/{id}/baseline — the most recent capture, or `null` when
/// the project has never been baselined.
#[utoipa::path(operation_id = "projects_get_baseline", get, path = "/api/projects/{id}/baseline", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = BaselineResponse),
              (status = 404, description = "project not visible in scope (rls_sparse)")))]
pub async fn get_baseline(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<BaselineResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    core_db::projects::get_project(&mut *tx, org_id, id).await?;
    let latest = core_db::projects::latest_baseline(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(BaselineResponse {
        baseline: latest.map(Into::into),
    }))
}

/// One computed signal as a surface reads it.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RiskSignalView {
    /// The stable rule identifier, so a dismissal can be stored against it.
    pub rule: String,
    /// `info`, `warn` or `critical`, assigned by the rule.
    pub severity: String,
    /// `project`, `phase` or `task`.
    pub subject: String,
    #[schema(value_type = uuid::Uuid)]
    pub subject_id: Uuid,
    pub message: String,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RiskResponse {
    pub signals: Vec<RiskSignalView>,
    pub generated_at: chrono::DateTime<chrono::Utc>,
}

/// GET /api/projects/{id}/risk — the deterministic signals this project's
/// facts support, most severe first.
///
/// Severity comes from the rule (ADR 0034 decision 11). An AI surface may rank
/// and narrate these, never invent one or raise one's weight.
#[utoipa::path(operation_id = "projects_risk", get, path = "/api/projects/{id}/risk", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = RiskResponse),
              (status = 404, description = "project not visible in scope (rls_sparse)")))]
pub async fn risk(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<RiskResponse>> {
    use core_projects::risk::{
        evaluate, PhaseFacts, ProjectFacts, RiskWindows, Severity, Subject, TaskFacts,
    };

    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;

    let project = core_db::projects::get_project(&mut *tx, org_id, id).await?;
    let phases = core_db::projects::list_phases(&mut *tx, org_id, id).await?;
    let burns = core_db::projects::phase_burn(&mut *tx, org_id, id).await?;
    let tasks = core_db::projects::list_tasks(
        &mut *tx,
        org_id,
        id,
        core_db::projects::TaskFilter {
            phase_id: None,
            status: None,
            assignee_id: None,
            milestone: None,
            limit: 10_000,
            offset: 0,
        },
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;

    let mut phase_facts = Vec::with_capacity(phases.len());
    for phase in &phases {
        let burn = burns.iter().find(|b| b.phase_id == phase.id);
        let in_phase: Vec<&core_domain::entities::ProjectTask> = tasks
            .iter()
            .filter(|t| t.phase_id == Some(phase.id))
            .collect();
        let done = in_phase
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::Completed))
            .count() as i64;
        phase_facts.push(PhaseFacts {
            id: phase.id,
            status: phase.status.as_str().to_string(),
            is_milestone: phase.is_milestone,
            end_date: phase.end_date,
            budget_minor: burn.and_then(|b| b.budget_minor),
            consumed_minor: burn.map(|b| b.consumed_minor),
            tasks_done: done,
            tasks_total: in_phase.len() as i64,
        });
    }

    let task_facts: Vec<TaskFacts> = tasks
        .iter()
        .map(|t| {
            let end_at = match (t.start_at, t.duration_minutes) {
                (Some(start), Some(minutes)) if minutes >= 0 => {
                    Some(start + chrono::Duration::minutes(i64::from(minutes)))
                }
                _ => None,
            };
            TaskFacts {
                id: t.id,
                status: t.status.as_str().to_string(),
                phase_id: t.phase_id,
                assigned: t.assignee_id.is_some() || t.role_key.is_some(),
                start_at: t.start_at,
                duration_minutes: t.duration_minutes,
                end_at,
                baseline_start_at: t.baseline_start_at,
                blocked_since: matches!(t.status, TaskStatus::Blocked).then_some(t.updated_at),
            }
        })
        .collect();

    let facts = ProjectFacts {
        project_id: project.id,
        project_status: project.status.as_str().to_string(),
        project_due_date: project.due_date,
        // Until status reports land, the project's own last write is the most
        // recent thing the client-visible record knows about.
        last_update_at: Some(project.updated_at),
        phases: phase_facts,
        tasks: task_facts,
    };

    let now = chrono::Utc::now();
    let signals = evaluate(&facts, now, RiskWindows::default());
    Ok(Json(RiskResponse {
        signals: signals
            .into_iter()
            .map(|s| RiskSignalView {
                rule: s.rule.code().to_string(),
                severity: match s.severity {
                    Severity::Info => "info",
                    Severity::Warn => "warn",
                    Severity::Critical => "critical",
                }
                .to_string(),
                subject: match s.subject {
                    Subject::Project => "project",
                    Subject::Phase => "phase",
                    Subject::Task => "task",
                }
                .to_string(),
                subject_id: s.subject_id,
                message: s.message,
            })
            .collect(),
        generated_at: now,
    }))
}
