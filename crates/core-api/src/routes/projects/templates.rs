use super::*;

// --- templates -----------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateTemplateRequest {
    pub name: String,
    pub description: Option<String>,
    /// Versioned template spec (core-projects `template`, spec_version 1).
    /// Validated eagerly so broken blueprints never persist.
    pub spec: Value,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PatchTemplateRequest {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub spec: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TemplateMutationResponse {
    #[serde(flatten)]
    pub template: ProjectTemplate,
}

/// GET /api/project-templates
#[utoipa::path(operation_id = "project_templates_list", get, path = "/api/project-templates", tag = "projects", security(("bearer" = [])),
    responses((status = 200, body = [ProjectTemplate])))]
pub async fn list_templates(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<ProjectTemplate>>> {
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
    let rows = core_db::projects::list_templates(&mut *tx, authed.principal.user.org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// POST /api/project-templates — stores a blueprint (spec validated);
/// emits `project.template_updated`.
#[utoipa::path(operation_id = "project_templates_create", post, path = "/api/project-templates", tag = "projects", security(("bearer" = [])),
    request_body = CreateTemplateRequest,
    responses((status = 200, body = TemplateMutationResponse),
              (status = 409, description = "template name already in use"),
              (status = 422, description = "invalid template spec")))]
pub async fn create_template(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateTemplateRequest>,
) -> ApiResult<Json<TemplateMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Create))?;
    if req.name.trim().is_empty() {
        return Err(ApiError::Validation("name is required".into()));
    }
    // Eager spec validation: templates are data; broken data never ships.
    core_projects::template::parse_spec(&req.spec)
        .map_err(|e| ApiError::Validation(e.to_string()))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let taken = core_db::projects::list_templates(&mut *ctx.tx, org_id)
            .await?
            .iter()
            .any(|t| t.name.eq_ignore_ascii_case(req.name.trim()));
        if taken {
            return Err(ApiError::Conflict(
                "a template with this name already exists".into(),
            ));
        }
        let template = core_db::projects::insert_template(
            &mut *ctx.tx,
            core_db::projects::NewTemplate {
                id: Uuid::now_v7(),
                org_id,
                name: req.name.trim(),
                description: req.description.as_deref(),
                spec: &req.spec,
            },
        )
        .await?;
        ctx.emit(
            types::PROJECT_TEMPLATE_UPDATED,
            "project_template",
            Some(template.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectTemplateUpdated {
                template_id: template.id,
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, TemplateMutationResponse { template }))
    })
    .await
    .map(Json)
}

/// PATCH /api/project-templates/{id} — emits `project.template_updated`.
#[utoipa::path(operation_id = "project_templates_patch", patch, path = "/api/project-templates/{id}", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = PatchTemplateRequest,
    responses((status = 200, body = TemplateMutationResponse),
              (status = 404, description = "template not visible in scope (rls_sparse)"),
              (status = 422, description = "invalid template spec")))]
pub async fn patch_template(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchTemplateRequest>,
) -> ApiResult<Json<TemplateMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    if let Some(name) = req.name.as_deref() {
        if name.trim().is_empty() {
            return Err(ApiError::Validation("name must not be empty".into()));
        }
    }
    if let Some(spec) = &req.spec {
        core_projects::template::parse_spec(spec)
            .map_err(|e| ApiError::Validation(e.to_string()))?;
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let template = core_db::projects::update_template(
            &mut *ctx.tx,
            org_id,
            id,
            core_db::projects::TemplatePatch {
                name: req.name.as_deref().map(str::trim),
                description: req.description.as_ref().map(|o| o.as_deref()),
                spec: req.spec.as_ref(),
            },
        )
        .await?;
        ctx.emit(
            types::PROJECT_TEMPLATE_UPDATED,
            "project_template",
            Some(template.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectTemplateUpdated {
                template_id: template.id,
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, TemplateMutationResponse { template }))
    })
    .await
    .map(Json)
}

/// DELETE /api/project-templates/{id} — blueprints are data, not live
/// references: projects already instantiated are untouched; emits
/// `project.template_removed`.
#[utoipa::path(operation_id = "project_templates_delete", delete, path = "/api/project-templates/{id}", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200),
              (status = 404, description = "template not visible in scope (rls_sparse)")))]
pub async fn delete_template(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let deleted = core_db::projects::delete_template(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if !deleted {
            return Err(ApiError::RlsSparse);
        }
        ctx.emit(
            types::PROJECT_TEMPLATE_REMOVED,
            "project_template",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectTemplateRemoved { template_id: id })
                .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, serde_json::json!({ "deleted": true })))
    })
    .await
    .map(Json)
}

// --- template instantiation ---------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct InstantiateTemplateRequest {
    pub name: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub client_id: Option<Uuid>,
    pub start_date: Option<chrono::NaiveDate>,
    pub due_date: Option<chrono::NaiveDate>,
    /// Resolves template `assignee_key`s to user ids at instantiation
    /// time; unmapped keys simply leave those tasks unassigned.
    #[serde(default)]
    pub assignee_map: HashMap<String, Uuid>,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version (of the newly created project). Absent in legacy clients —
    /// purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// The instantiation result: the applied project plus the expansion
/// counts (clients render "created N phases, M tasks" without refetching).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ProjectInstantiateResponse {
    #[serde(flatten)]
    pub project: Project,
    pub phase_count: i64,
    pub task_count: i64,
    pub mutation: instant::MutationMeta,
}

/// POST /api/project-templates/{id}/instantiate — expands the template
/// into a REAL project: phases, tasks, dependencies, and every lifecycle
/// event, all in ONE transaction (claim → expand → insert → events →
/// record). Rollback on any failure leaves zero partial rows.
#[utoipa::path(operation_id = "project_templates_instantiate", post, path = "/api/project-templates/{id}/instantiate", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = InstantiateTemplateRequest,
    responses((status = 200, body = ProjectInstantiateResponse),
              (status = 404, description = "template not visible in scope (rls_sparse)"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "invalid template spec or request")))]
pub async fn instantiate(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<InstantiateTemplateRequest>,
) -> ApiResult<Json<ProjectInstantiateResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Create))?;
    if req.name.trim().is_empty() {
        return Err(ApiError::Validation("name is required".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "project",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let template = core_db::projects::get_template(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let spec = core_projects::template::parse_spec(&template.spec)
            .map_err(|e| ApiError::Validation(e.to_string()))?;
        let plan = spec.expand();

        let billing_method = plan
            .billing_method
            .as_deref()
            .map(|s| s.parse::<BillingMethod>())
            .transpose()
            .map_err(|e| ApiError::Validation(e.to_string()))?
            .unwrap_or(BillingMethod::NonBillable);
        if let Some(code) = plan.currency.as_deref() {
            if !valid_currency(code) {
                return Err(ApiError::Validation(format!(
                    "template currency is not a valid ISO 4217 code: {code}"
                )));
            }
        }
        if core_db::projects::project_name_taken(&mut *ctx.tx, org_id, req.name.trim()).await? {
            return Err(ApiError::Conflict(
                "a project with this name already exists".into(),
            ));
        }
        let project = core_db::projects::insert_project(
            &mut *ctx.tx,
            core_db::projects::NewProject {
                id: Uuid::now_v7(),
                org_id,
                client_id: req.client_id,
                name: req.name.trim(),
                description: template.description.as_deref(),
                billing_method,
                budget_minor: None,
                currency: plan.currency.as_deref(),
                start_date: req.start_date,
                due_date: req.due_date,
                // Templates stay unlinked: a fresh instantiation bills
                // nothing until someone attaches a client + contract.
                billing_contract_id: None,
            },
        )
        .await?;

        // Phases first (tasks reference their ids).
        let mut phase_ids: HashMap<String, Uuid> = HashMap::new();
        let mut phase_count: i64 = 0;
        for phase in &plan.phases {
            let phase_id = Uuid::now_v7();
            let inserted = core_db::projects::insert_phase(
                &mut *ctx.tx,
                core_db::projects::NewPhase {
                    id: phase_id,
                    org_id,
                    project_id: project.id,
                    name: &phase.name,
                    description: phase.description.as_deref(),
                    sort_order: phase.sort_order,
                    is_milestone: phase.is_milestone,
                    milestone_amount_minor: phase.milestone_amount_minor,
                    milestone_currency: phase.milestone_currency.as_deref(),
                    // A template seeds no dates: the dates a real project
                    // gets depend on when it is scheduled, and baking
                    // template dates in would put a plan on a Gantt that
                    // nobody chose for this engagement.
                    start_date: None,
                    end_date: None,
                },
            )
            .await?;
            ctx.emit(
                types::PROJECT_PHASE_CREATED,
                "project_phase",
                Some(inserted.id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::ProjectPhaseCreated { phase: inserted })
                    .unwrap_or_default(),
            )
            .await?;
            phase_ids.insert(phase.key.clone(), phase_id);
            phase_count += 1;
        }

        // Tasks second, keyed for the dependency pass.
        let mut task_ids: HashMap<String, Uuid> = HashMap::new();
        let mut inserted_tasks: Vec<ProjectTask> = Vec::with_capacity(plan.tasks.len());
        for task in &plan.tasks {
            let task_id = Uuid::now_v7();
            let phase_id = task.phase_key.as_deref().map(|key| {
                phase_ids
                    .get(key)
                    .copied()
                    .expect("spec validation guarantees phase keys resolve")
            });
            let due_at = match (req.start_date, task.due_offset_days) {
                (Some(start), Some(days)) => start
                    .and_hms_opt(0, 0, 0)
                    .map(|t| t.and_utc() + chrono::Duration::days(days)),
                _ => None,
            };
            let assignee_id = task
                .assignee_key
                .as_deref()
                .and_then(|k| req.assignee_map.get(k).copied());
            let inserted = core_db::projects::insert_task(
                &mut *ctx.tx,
                core_db::projects::NewTask {
                    id: task_id,
                    org_id,
                    project_id: project.id,
                    phase_id,
                    title: &task.title,
                    description: task.description.as_deref(),
                    assignee_id,
                    sort_order: task.sort_order,
                    is_milestone: task.is_milestone,
                    milestone_amount_minor: task.milestone_amount_minor,
                    milestone_currency: task.milestone_currency.as_deref(),
                    due_at,
                    // A template step carries no plan of its own yet: dates are
                    // set on the instantiated project, not inherited from the
                    // document.
                    start_at: None,
                    duration_minutes: None,
                    effort_estimate_minutes: None,
                    effort_remaining_minutes: None,
                    work_type: None,
                    priority: None,
                    role_key: None,
                    required_skills: &[],
                    is_critical: false,
                    external_owner: None,
                    calendar_pin: None,
                },
            )
            .await?;
            ctx.emit(
                types::PROJECT_TASK_CREATED,
                "project_task",
                Some(inserted.id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::ProjectTaskCreated {
                    task: inserted.clone(),
                })
                .unwrap_or_default(),
            )
            .await?;
            task_ids.insert(task.key.clone(), inserted.id);
            inserted_tasks.push(inserted);
        }

        // Dependencies last: every edge now resolves to a real task id.
        for task in &plan.tasks {
            let task_id = task_ids
                .get(&task.key)
                .copied()
                .expect("inserted above under its key");
            for dep_key in &task.depends_on {
                let dep_id = task_ids
                    .get(dep_key)
                    .copied()
                    .expect("spec validation guarantees dep keys resolve");
                core_db::projects::add_task_dep(&mut *ctx.tx, org_id, task_id, dep_id).await?;
            }
        }

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "project",
                entity_id: project.id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            types::PROJECT_CREATED,
            "project",
            Some(project.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectCreated {
                project: project.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        let task_count = inserted_tasks.len() as i64;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "project",
                entity_id: project.id,
                op: "instantiate",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&project).unwrap_or(Value::Null),
                updated_at: project.updated_at,
                key: key.as_deref(),
                payload: &project,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ProjectInstantiateResponse {
                project,
                phase_count,
                task_count,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- template versions ----------------------------------------------------------------

/// One version of a template, as a surface reads it.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct TemplateVersionView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub template_id: Uuid,
    pub version: i32,
    pub spec: Value,
    /// `draft`, `published` or `retired`.
    pub status: String,
    /// `shipped`, `org` or `fork`.
    pub source: String,
    pub note: Option<String>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub published_by: Option<Uuid>,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Whether this is the version an instantiation would read.
    pub current: bool,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct TemplateVersionsResponse {
    pub versions: Vec<TemplateVersionView>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub current_version_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct AppendVersionRequest {
    /// The document to store. Absent means "take the template's current spec",
    /// which is how an author starts a version from what shipped.
    pub spec: Option<Value>,
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ForkTemplateRequest {
    pub name: String,
}

fn version_view(
    v: core_db::projects::TemplateVersion,
    current: Option<Uuid>,
) -> TemplateVersionView {
    TemplateVersionView {
        current: current == Some(v.id),
        id: v.id,
        template_id: v.template_id,
        version: v.version,
        spec: v.spec,
        status: v.status,
        source: v.source,
        note: v.note,
        published_by: v.published_by,
        published_at: v.published_at,
        created_at: v.created_at,
    }
}

async fn current_version_id(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    template_id: Uuid,
) -> ApiResult<Option<Uuid>> {
    let row: Option<(Option<Uuid>,)> = sqlx::query_as(
        "SELECT current_version_id FROM project_templates WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(template_id)
    .fetch_optional(&mut *tx)
    .await?;
    match row {
        Some((id,)) => Ok(id),
        None => Err(ApiError::RlsSparse),
    }
}

/// GET /api/project-templates/{id}/versions — every version, newest first,
/// with the current one marked.
#[utoipa::path(operation_id = "projects_template_versions", get, path = "/api/project-templates/{id}/versions", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = TemplateVersionsResponse),
              (status = 404, description = "template not visible in scope (rls_sparse)")))]
pub async fn list_versions(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<TemplateVersionsResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let current = current_version_id(&mut tx, org_id, id).await?;
    let versions = core_db::projects::list_template_versions(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(TemplateVersionsResponse {
        versions: versions
            .into_iter()
            .map(|v| version_view(v, current))
            .collect(),
        current_version_id: current,
    }))
}

/// POST /api/project-templates/{id}/versions — appends a draft. The document
/// is validated as a template spec before it is stored, so a draft that would
/// not instantiate cannot be created.
#[utoipa::path(operation_id = "projects_append_template_version", post, path = "/api/project-templates/{id}/versions", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = AppendVersionRequest,
    responses((status = 200, body = TemplateVersionView),
              (status = 404, description = "template not visible in scope (rls_sparse)"),
              (status = 422, description = "validation failed")))]
pub async fn append_version(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<AppendVersionRequest>,
) -> ApiResult<Json<TemplateVersionView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    if let Some(spec) = req.spec.as_ref() {
        core_projects::template::parse_spec(spec)
            .map_err(|e| ApiError::Validation(e.to_string()))?;
    }
    let note = req.note.as_deref().map(str::trim).filter(|n| !n.is_empty());
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let version = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::projects::get_template(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let version = core_db::projects::append_template_version(
            &mut ctx.tx,
            org_id,
            id,
            req.spec.as_ref(),
            note,
        )
        .await?;
        Ok((ctx, version))
    })
    .await?;

    Ok(Json(version_view(version, None)))
}

/// POST /api/project-templates/{id}/versions/{version}/publish — makes a
/// version the template's current document. Publication is what an
/// instantiation reads, so a draft never changes a project by accident.
#[utoipa::path(operation_id = "projects_publish_template_version", post, path = "/api/project-templates/{id}/versions/{version}/publish", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("version" = i32, Path)),
    responses((status = 200, body = TemplateVersionView),
              (status = 404, description = "template or version not visible in scope (rls_sparse)"),
              (status = 422, description = "the version does not parse as a spec")))]
pub async fn publish_version(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, version)): Path<(Uuid, i32)>,
) -> ApiResult<Json<TemplateVersionView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let published = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let published = core_db::projects::publish_template_version(
            &mut ctx.tx,
            org_id,
            id,
            version,
            Some(user_id),
        )
        .await
        .map_err(|e| instant::mutation_scope_err(e.into()))?;
        core_projects::template::parse_spec(&published.spec)
            .map_err(|e| ApiError::Validation(e.to_string()))?;
        ctx.emit(
            types::PROJECT_TEMPLATE_UPDATED,
            "project_template",
            Some(id.to_string().as_str()),
            &serde_json::json!({ "template_id": id, "version": published.version }),
        )
        .await?;
        Ok((ctx, published))
    })
    .await?;

    Ok(Json(version_view(published, None)))
}

/// POST /api/project-templates/{id}/fork — copies the current version into a
/// new org-owned template. The fork keeps the plan it was tuned from, which is
/// what makes a shipped template safe to improve.
#[utoipa::path(operation_id = "projects_fork_template", post, path = "/api/project-templates/{id}/fork", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = ForkTemplateRequest,
    responses((status = 200, body = ProjectTemplate),
              (status = 404, description = "template not visible in scope (rls_sparse)"),
              (status = 409, description = "a template with this name already exists"),
              (status = 422, description = "validation failed")))]
pub async fn fork_template(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<ForkTemplateRequest>,
) -> ApiResult<Json<core_domain::entities::ProjectTemplate>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Create))?;
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::Validation("name is required".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let template = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::projects::get_template(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let taken = core_db::projects::list_templates(&mut *ctx.tx, org_id)
            .await?
            .iter()
            .any(|t| t.name.eq_ignore_ascii_case(name));
        if taken {
            return Err(ApiError::Conflict(
                "a template with this name already exists".into(),
            ));
        }
        let template = core_db::projects::fork_template(&mut ctx.tx, org_id, id, name).await?;
        Ok((ctx, template))
    })
    .await?;

    Ok(Json(template))
}
