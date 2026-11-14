use super::*;

// --- tasks ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateTaskRequest {
    pub title: String,
    pub description: Option<String>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub phase_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub assignee_id: Option<Uuid>,
    #[serde(default)]
    pub sort_order: i32,
    #[serde(default)]
    pub is_milestone: bool,
    pub milestone_amount_minor: Option<i64>,
    pub milestone_currency: Option<String>,
    pub due_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Finish-to-start dependencies to create with the task; every id must
    /// be a task of the SAME project. Cycles are impossible at creation
    /// (nothing can depend on the not-yet-existing task) but membership is
    /// validated.
    #[serde(default)]
    pub depends_on: Vec<Uuid>,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PatchTaskRequest {
    pub title: Option<String>,
    pub description: Option<Option<String>>,
    pub phase_id: Option<Option<Uuid>>,
    /// Lifecycle moves other than completion (pending / in_progress /
    /// blocked / cancelled). Reaching `completed` MUST go through the
    /// complete endpoint so dependency gating + billing events cannot be
    /// skipped.
    pub status: Option<TaskStatus>,
    pub assignee_id: Option<Option<Uuid>>,
    pub sort_order: Option<i32>,
    pub is_milestone: Option<bool>,
    pub milestone_amount_minor: Option<Option<i64>>,
    pub milestone_currency: Option<Option<String>>,
    pub due_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct TaskListQuery {
    #[schema(value_type = Option<uuid::Uuid>)]
    pub phase_id: Option<Uuid>,
    pub status: Option<TaskStatus>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub assignee_id: Option<Uuid>,
    pub milestone: Option<bool>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

/// GET /api/projects/{id}/tasks — filtered by phase/status/assignee and
/// the milestone flag, sort_order-ordered.
#[utoipa::path(operation_id = "projects_list_tasks", get, path = "/api/projects/{id}/tasks", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), TaskListQuery),
    responses((status = 200, body = [ProjectTask])))]
pub async fn list_tasks(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Query(q): Query<TaskListQuery>,
) -> ApiResult<Json<Vec<ProjectTask>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let rows = core_db::projects::list_tasks(
        &mut *tx,
        authed.principal.user.org_id,
        id,
        core_db::projects::TaskFilter {
            phase_id: q.phase_id,
            status: q.status,
            assignee_id: q.assignee_id,
            milestone: q.milestone,
            limit: q.limit,
            offset: q.offset,
        },
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// POST /api/projects/{id}/tasks — creates the task (optionally with its
/// dependency set) and emits `project.task_created` in one transaction.
#[utoipa::path(operation_id = "projects_create_task", post, path = "/api/projects/{id}/tasks", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = CreateTaskRequest,
    responses((status = 200, body = TaskMutationResponse),
              (status = 404, description = "project not visible in scope (rls_sparse)"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "validation failed")))]
pub async fn create_task(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateTaskRequest>,
) -> ApiResult<Json<TaskMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Create))?;
    if req.title.trim().is_empty() {
        return Err(ApiError::Validation("title is required".into()));
    }
    if let Some(code) = req.milestone_currency.as_deref() {
        if !valid_currency(code) {
            return Err(ApiError::Validation(format!(
                "invalid ISO 4217 currency code: {code}"
            )));
        }
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "project_task",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let project = core_db::projects::get_project(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if let Some(phase_id) = req.phase_id {
            let phase = core_db::projects::get_phase(&mut *ctx.tx, org_id, phase_id)
                .await
                .map_err(|e| instant::mutation_scope_err(e.into()))?;
            if phase.project_id != id {
                return Err(ApiError::Validation(
                    "phase_id belongs to a different project".into(),
                ));
            }
        }
        let amount = resolve_amount(
            req.milestone_amount_minor,
            req.milestone_currency.as_deref(),
            project.currency.as_deref(),
        )?;
        let new_id = Uuid::now_v7();
        // Dependency membership: every dep must live in the SAME project.
        // A fresh task cannot close a cycle (nothing points at it yet),
        // but the shared validator keeps the rules in one place.
        let existing = core_db::projects::list_project_dep_edges(&mut *ctx.tx, org_id, id).await?;
        let mut deps: Vec<Uuid> = Vec::with_capacity(req.depends_on.len());
        for dep in &req.depends_on {
            let dep_task = core_db::projects::get_task(&mut *ctx.tx, org_id, *dep).await?;
            if dep_task.project_id != id {
                return Err(ApiError::Validation(format!(
                    "dependency {dep} belongs to a different project"
                )));
            }
            if deps.contains(dep) {
                continue;
            }
            core_projects::deps::validate_add(&existing, new_id, *dep, true)
                .map_err(|r| ApiError::Validation(r.to_string()))?;
            deps.push(*dep);
        }
        let task = core_db::projects::insert_task(
            &mut *ctx.tx,
            core_db::projects::NewTask {
                id: new_id,
                org_id,
                project_id: id,
                phase_id: req.phase_id,
                title: req.title.trim(),
                description: req.description.as_deref(),
                assignee_id: req.assignee_id,
                sort_order: req.sort_order,
                is_milestone: req.is_milestone,
                milestone_amount_minor: amount.as_ref().map(|(m, _)| *m),
                milestone_currency: amount.as_ref().map(|(_, c)| c.as_str()),
                due_at: req.due_at,
            },
        )
        .await?;
        if !deps.is_empty() {
            core_db::projects::set_task_deps(&mut ctx.tx, org_id, task.id, &deps).await?;
        }
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "project_task",
                entity_id: task.id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            types::PROJECT_TASK_CREATED,
            "project_task",
            Some(task.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectTaskCreated { task: task.clone() })
                .unwrap_or_default(),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "project_task",
                entity_id: task.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&task).unwrap_or(Value::Null),
                updated_at: task.updated_at,
                key: key.as_deref(),
                payload: &task,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TaskMutationResponse { task, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/projects/{id}/tasks/{task_id} — tri-state partial update.
/// `status: "completed"` is rejected: completion runs through the complete
/// endpoint (dependency gating + the billing seam).
#[utoipa::path(operation_id = "projects_patch_task", patch, path = "/api/projects/{id}/tasks/{task_id}", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("task_id" = Uuid, Path)), request_body = PatchTaskRequest,
    responses((status = 200, body = TaskMutationResponse),
              (status = 404, description = "task not visible in scope (rls_sparse)"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "validation failed")))]
pub async fn patch_task(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, task_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<PatchTaskRequest>,
) -> ApiResult<Json<TaskMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    if let Some(title) = req.title.as_deref() {
        if title.trim().is_empty() {
            return Err(ApiError::Validation("title must not be empty".into()));
        }
    }
    if req.status == Some(TaskStatus::Completed) {
        return Err(ApiError::Validation(
            "task completion goes through POST .../complete (dependency gating + billing events)"
                .into(),
        ));
    }
    if let Some(Some(code)) = req.milestone_currency.as_ref().map(|o| o.as_deref()) {
        if !valid_currency(code) {
            return Err(ApiError::Validation(format!(
                "invalid ISO 4217 currency code: {code}"
            )));
        }
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let version = req.instant.version;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "project_task",
                entity_id: Some(task_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::projects::get_task(&mut *ctx.tx, org_id, task_id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if before.project_id != id {
            return Err(ApiError::RlsSparse);
        }
        if let Some(Some(phase_id)) = req.phase_id {
            let phase = core_db::projects::get_phase(&mut *ctx.tx, org_id, phase_id)
                .await
                .map_err(|e| instant::mutation_scope_err(e.into()))?;
            if phase.project_id != id {
                return Err(ApiError::Validation(
                    "phase_id belongs to a different project".into(),
                ));
            }
        }
        let project = core_db::projects::get_project(&mut *ctx.tx, org_id, id).await?;
        let eff_amount = req
            .milestone_amount_minor
            .unwrap_or(before.milestone_amount_minor);
        let eff_currency: Option<String> = req
            .milestone_currency
            .clone()
            .unwrap_or_else(|| before.milestone_currency.clone());
        let _validated_amount = resolve_amount(
            eff_amount,
            eff_currency.as_deref(),
            project.currency.as_deref(),
        )?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "project_task",
                entity_id: task_id,
                version,
            },
        )
        .await?;
        let after = core_db::projects::update_task(
            &mut *ctx.tx,
            org_id,
            task_id,
            core_db::projects::TaskPatch {
                title: req.title.as_deref().map(str::trim),
                description: req.description.as_ref().map(|o| o.as_deref()),
                phase_id: req.phase_id,
                status: req.status,
                assignee_id: req.assignee_id,
                sort_order: req.sort_order,
                is_milestone: req.is_milestone,
                milestone_amount_minor: req.milestone_amount_minor,
                milestone_currency: req.milestone_currency.as_ref().map(|o| o.as_deref()),
                due_at: req.due_at,
            },
        )
        .await?;
        ctx.emit(
            types::PROJECT_TASK_UPDATED,
            "project_task",
            Some(after.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectTaskUpdated {
                task_id: after.id,
                project_id: after.project_id,
            })
            .unwrap_or_default(),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "project_task",
                entity_id: task_id,
                op: "update",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&after).unwrap_or(Value::Null),
                updated_at: after.updated_at,
                key: key.as_deref(),
                payload: &after,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TaskMutationResponse {
                task: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/projects/{id}/tasks/{task_id}/complete — the guarded
/// completion: every finish-to-start dependency must already be completed
/// (typed 409 `task_dependencies_unsatisfied` naming the blockers), then
/// `project.task_completed` fires — plus `project.milestone_completed`
/// when the task is a billing milestone. Instant-first: the idempotency
/// key makes retried completions replay instead of double-billing.
#[utoipa::path(operation_id = "projects_complete_task", post, path = "/api/projects/{id}/tasks/{task_id}/complete", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("task_id" = Uuid, Path)),
    responses((status = 200, body = TaskMutationResponse),
              (status = 404, description = "task not visible in scope (rls_sparse)"),
              (status = 409, description = "dependencies unsatisfied, already completed, version conflict, or idempotency key reused")))]
pub async fn complete_task(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, task_id)): Path<(Uuid, Uuid)>,
    body: Option<Json<instant::MutationParams>>,
) -> ApiResult<Json<TaskMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let req: instant::MutationParams = body.map(|Json(p)| p).unwrap_or_default();
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.idempotency_key.clone();
    let version = req.version;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "project_task",
                entity_id: Some(task_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::projects::get_task(&mut *ctx.tx, org_id, task_id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if before.project_id != id {
            return Err(ApiError::RlsSparse);
        }
        if before.status == TaskStatus::Completed {
            return Err(ApiError::Conflict("task already completed".into()));
        }
        if before.status == TaskStatus::Cancelled {
            return Err(ApiError::Conflict(
                "cancelled tasks cannot be completed".into(),
            ));
        }
        let blocking = core_db::projects::incomplete_deps(&mut *ctx.tx, org_id, task_id).await?;
        if !blocking.is_empty() {
            return Err(ApiError::TaskDependenciesUnsatisfied { task_id, blocking });
        }
        let project = core_db::projects::get_project(&mut *ctx.tx, org_id, id).await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "project_task",
                entity_id: task_id,
                version,
            },
        )
        .await?;
        let at = chrono::Utc::now();
        let after = core_db::projects::complete_task(&mut *ctx.tx, org_id, task_id, at).await?;
        ctx.emit(
            types::PROJECT_TASK_COMPLETED,
            "project_task",
            Some(after.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectTaskCompleted {
                task: after.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        if after.is_milestone {
            let amount = resolve_amount(
                after.milestone_amount_minor,
                after.milestone_currency.as_deref(),
                project.currency.as_deref(),
            )?;
            emit_milestone_completed(
                &mut ctx,
                &project,
                &after.title,
                None,
                Some(after.id),
                amount,
                at,
            )
            .await?;
        }
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "project_task",
                entity_id: task_id,
                op: "complete",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&after).unwrap_or(Value::Null),
                updated_at: after.updated_at,
                key: key.as_deref(),
                payload: &after,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TaskMutationResponse {
                task: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}
