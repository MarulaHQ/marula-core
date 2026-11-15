use super::*;

// --- projects -------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateProjectRequest {
    pub name: String,
    pub description: Option<String>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub client_id: Option<Uuid>,
    #[serde(default)]
    pub billing_method: BillingMethod,
    pub budget_minor: Option<i64>,
    pub currency: Option<String>,
    pub start_date: Option<chrono::NaiveDate>,
    pub due_date: Option<chrono::NaiveDate>,
    /// The contract this project bills through (Wave 39 lane 4, report
    /// P2). Its rate card/blocks/caps price the project's T&M entries and
    /// its runs carry the project's released milestones. Validated to
    /// belong to the same client.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub billing_contract_id: Option<Uuid>,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PatchProjectRequest {
    pub name: Option<String>,
    /// `Some(Some(v))` sets; `Some(None)` clears; `None` = no change.
    pub description: Option<Option<String>>,
    pub client_id: Option<Option<Uuid>>,
    pub status: Option<ProjectStatus>,
    pub billing_method: Option<BillingMethod>,
    pub budget_minor: Option<Option<i64>>,
    pub currency: Option<Option<String>>,
    pub start_date: Option<Option<chrono::NaiveDate>>,
    pub due_date: Option<Option<chrono::NaiveDate>>,
    /// `Some(Some(v))` links the contract; `Some(None)` unlinks;
    /// `None` = no change (Wave 39 lane 4, report P2).
    pub billing_contract_id: Option<Option<Uuid>>,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct ProjectListQuery {
    pub status: Option<ProjectStatus>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub client_id: Option<Uuid>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

/// GET /api/projects — filtered, newest-first list.
#[utoipa::path(operation_id = "projects_list", get, path = "/api/projects", tag = "projects", security(("bearer" = [])),
    params(ProjectListQuery),
    responses((status = 200, body = [Project])))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ProjectListQuery>,
) -> ApiResult<Json<Vec<Project>>> {
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
    let rows = core_db::projects::list_projects(
        &mut *tx,
        authed.principal.user.org_id,
        core_db::projects::ProjectFilter {
            status: q.status,
            client_id: q.client_id,
            limit: q.limit,
            offset: q.offset,
        },
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// POST /api/projects — creates the project and emits `project.created` in
/// one transaction. Instant-first (IF1/IF2): idempotency-keyed,
/// version-gated, read-your-write response.
#[utoipa::path(operation_id = "projects_create", post, path = "/api/projects", tag = "projects", security(("bearer" = [])),
    request_body = CreateProjectRequest,
    responses((status = 200, body = ProjectMutationResponse),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "validation failed")))]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateProjectRequest>,
) -> ApiResult<Json<ProjectMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Create))?;
    if req.name.trim().is_empty() {
        return Err(ApiError::Validation("name is required".into()));
    }
    if let Some(code) = req.currency.as_deref() {
        if !valid_currency(code) {
            return Err(ApiError::Validation(format!(
                "invalid ISO 4217 currency code: {code}"
            )));
        }
    }
    if req.budget_minor.is_some() && req.currency.is_none() {
        return Err(ApiError::Validation(
            "budget_minor requires currency".into(),
        ));
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
        if let Some(contract_id) = req.billing_contract_id {
            validate_billing_link(&mut ctx.tx, org_id, req.client_id, contract_id).await?;
        }
        let project = core_db::projects::insert_project(
            &mut *ctx.tx,
            core_db::projects::NewProject {
                id: Uuid::now_v7(),
                org_id,
                client_id: req.client_id,
                name: req.name.trim(),
                description: req.description.as_deref(),
                billing_method: req.billing_method,
                budget_minor: req.budget_minor,
                currency: req.currency.as_deref(),
                start_date: req.start_date,
                due_date: req.due_date,
                billing_contract_id: req.billing_contract_id,
            },
        )
        .await?;
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
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "project",
                entity_id: project.id,
                op: "create",
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
            MutationOutcome::Applied(ProjectMutationResponse { project, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// GET /api/projects/{id}

#[utoipa::path(operation_id = "projects_get_one", get, path = "/api/projects/{id}", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Project),
              (status = 404, description = "not found")))]
pub async fn get_one(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Project>> {
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
    let project =
        core_db::projects::get_project(&mut *tx, authed.principal.user.org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(project))
}

/// PATCH /api/projects/{id} — tri-state partial update. Instant-first
/// (IF1/IF2): idempotency-keyed, version-gated, returns prior + new values.
#[utoipa::path(operation_id = "projects_patch", patch, path = "/api/projects/{id}", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = PatchProjectRequest,
    responses((status = 200, body = ProjectMutationResponse),
              (status = 404, description = "project not visible in scope (rls_sparse)"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "validation failed")))]
pub async fn patch(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchProjectRequest>,
) -> ApiResult<Json<ProjectMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    if let Some(name) = req.name.as_deref() {
        if name.trim().is_empty() {
            return Err(ApiError::Validation("name must not be empty".into()));
        }
    }
    if let Some(Some(code)) = req.currency.as_ref().map(|o| o.as_deref()) {
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
                entity: "project",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::projects::get_project(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        // Effective budget/currency pairing after this patch lands.
        let eff_budget = req.budget_minor.unwrap_or(before.budget_minor);
        let eff_currency: Option<String> = req
            .currency
            .clone()
            .unwrap_or_else(|| before.currency.clone());
        if eff_budget.is_some() && eff_currency.is_none() {
            return Err(ApiError::Validation(
                "budget_minor requires currency".into(),
            ));
        }
        // The link lands against the EFFECTIVE client (a patch may move
        // both together), so validate the pair after applying the patch's
        // client election.
        let eff_client = req.client_id.flatten().or(before.client_id);
        if let Some(contract_id) = req.billing_contract_id.flatten() {
            validate_billing_link(&mut ctx.tx, org_id, eff_client, contract_id).await?;
        }
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "project",
                entity_id: id,
                version,
            },
        )
        .await?;
        let after = core_db::projects::update_project(
            &mut *ctx.tx,
            org_id,
            id,
            core_db::projects::ProjectPatch {
                name: req.name.as_deref().map(str::trim),
                description: req.description.as_ref().map(|o| o.as_deref()),
                client_id: req.client_id,
                status: req.status,
                billing_method: req.billing_method,
                budget_minor: req.budget_minor,
                currency: req.currency.as_ref().map(|o| o.as_deref()),
                start_date: req.start_date,
                due_date: req.due_date,
                billing_contract_id: req.billing_contract_id,
            },
        )
        .await?;
        ctx.emit(
            types::PROJECT_UPDATED,
            "project",
            Some(after.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectUpdated {
                project_id: after.id,
            })
            .unwrap_or_default(),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "project",
                entity_id: id,
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
            MutationOutcome::Applied(ProjectMutationResponse {
                project: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- phases ------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreatePhaseRequest {
    pub name: String,
    pub description: Option<String>,
    /// The planned window (Wave 34 claims close-out, claim 6). Both ends
    /// optional: a phase created without dates lands in the ordered list,
    /// NOT on the Gantt, and the UI says which it is. Supplying only one
    /// end is rejected at the database CHECK, because a phase with a start
    /// and no length has no bar to draw.
    pub start_date: Option<chrono::NaiveDate>,
    pub end_date: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub sort_order: i32,
    #[serde(default)]
    pub is_milestone: bool,
    pub milestone_amount_minor: Option<i64>,
    pub milestone_currency: Option<String>,
    /// Wave 39 lane 4 (report C/D): the phase's billing budget and/or the
    /// fixed fee a COMPLETED non-milestone phase releases. Milestone
    /// phases keep using `milestone_amount_*`.
    #[serde(default)]
    pub budget_minor: Option<i64>,
    #[serde(default)]
    pub budget_currency: Option<String>,
    #[serde(default)]
    pub fee_minor: Option<i64>,
    #[serde(default)]
    pub fee_currency: Option<String>,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PatchPhaseRequest {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    /// Tri-state, matching the budget fields below: `Some(Some(d))` sets a
    /// date, `Some(None)` takes the phase off the Gantt, and an absent field
    /// leaves it alone. Without the distinction, renaming a phase would
    /// quietly erase its planned window.
    pub start_date: Option<Option<chrono::NaiveDate>>,
    pub end_date: Option<Option<chrono::NaiveDate>>,
    pub sort_order: Option<i32>,
    pub status: Option<PhaseStatus>,
    pub is_milestone: Option<bool>,
    pub milestone_amount_minor: Option<Option<i64>>,
    pub milestone_currency: Option<Option<String>>,
    /// Wave 39 lane 4: tri-state budget/fee updates on the phase's
    /// billing record (`Some(Some(v))` sets, `Some(None)` clears,
    /// `None` = no change; `clear_*` drops the whole record when both
    /// fields would be empty).
    pub budget_minor: Option<Option<i64>>,
    pub budget_currency: Option<Option<String>>,
    pub fee_minor: Option<Option<i64>>,
    pub fee_currency: Option<Option<String>>,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// GET /api/projects/{id}/phases — ordered by sort_order.
#[utoipa::path(operation_id = "projects_list_phases", get, path = "/api/projects/{id}/phases", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [ProjectPhase])))]
pub async fn list_phases(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<ProjectPhase>>> {
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
    let rows = core_db::projects::list_phases(&mut *tx, authed.principal.user.org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// POST /api/projects/{id}/phases — adds a phase; emits
/// `project.phase_created`.
#[utoipa::path(operation_id = "projects_create_phase", post, path = "/api/projects/{id}/phases", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = CreatePhaseRequest,
    responses((status = 200, body = PhaseMutationResponse),
              (status = 404, description = "project not visible in scope (rls_sparse)"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "validation failed")))]
pub async fn create_phase(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreatePhaseRequest>,
) -> ApiResult<Json<PhaseMutationResponse>> {
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
                entity: "project_phase",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        // Parent must be visible (mutation target's parent left scope ⇒
        // rls_sparse per IF3).
        let project = core_db::projects::get_project(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let amount = resolve_amount(
            req.milestone_amount_minor,
            req.milestone_currency.as_deref(),
            project.currency.as_deref(),
        )?;
        validate_money_pair(req.budget_minor, req.budget_currency.as_deref(), "budget")?;
        validate_money_pair(req.fee_minor, req.fee_currency.as_deref(), "fee")?;
        validate_phase_dates(req.start_date, req.end_date)?;
        let phase = core_db::projects::insert_phase(
            &mut *ctx.tx,
            core_db::projects::NewPhase {
                id: Uuid::now_v7(),
                org_id,
                project_id: id,
                name: req.name.trim(),
                description: req.description.as_deref(),
                sort_order: req.sort_order,
                is_milestone: req.is_milestone,
                milestone_amount_minor: amount.as_ref().map(|(m, _)| *m),
                milestone_currency: amount.as_ref().map(|(_, c)| c.as_str()),
                start_date: req.start_date,
                end_date: req.end_date,
            },
        )
        .await?;
        if req.budget_minor.is_some() || req.fee_minor.is_some() {
            core_db::projects::upsert_phase_budget(
                &mut *ctx.tx,
                org_id,
                phase.id,
                core_db::projects::NewPhaseBudget {
                    budget_minor: req.budget_minor,
                    currency: req.budget_currency.as_deref(),
                    fee_minor: req.fee_minor,
                    fee_currency: req.fee_currency.as_deref(),
                },
            )
            .await?;
        }
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "project_phase",
                entity_id: phase.id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            types::PROJECT_PHASE_CREATED,
            "project_phase",
            Some(phase.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectPhaseCreated {
                phase: phase.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "project_phase",
                entity_id: phase.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&phase).unwrap_or(Value::Null),
                updated_at: phase.updated_at,
                key: key.as_deref(),
                payload: &phase,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(PhaseMutationResponse { phase, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/projects/{id}/phases/{phase_id} — tri-state partial update.
#[utoipa::path(operation_id = "projects_patch_phase", patch, path = "/api/projects/{id}/phases/{phase_id}", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("phase_id" = Uuid, Path)), request_body = PatchPhaseRequest,
    responses((status = 200, body = PhaseMutationResponse),
              (status = 404, description = "phase not visible in scope (rls_sparse)"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "validation failed")))]
pub async fn patch_phase(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, phase_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<PatchPhaseRequest>,
) -> ApiResult<Json<PhaseMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    if let Some(name) = req.name.as_deref() {
        if name.trim().is_empty() {
            return Err(ApiError::Validation("name must not be empty".into()));
        }
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
                entity: "project_phase",
                entity_id: Some(phase_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::projects::get_phase(&mut *ctx.tx, org_id, phase_id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if before.project_id != id {
            return Err(ApiError::RlsSparse);
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
        // Effective budget/fee record after this patch lands (tri-state
        // over the existing row; a cleared pair drops the whole record).
        let existing = core_db::projects::get_phase_budget(&mut *ctx.tx, org_id, phase_id).await?;
        let eff_budget: Option<i64> = req
            .budget_minor
            .unwrap_or(existing.as_ref().and_then(|b| b.budget_minor));
        let eff_budget_ccy: Option<String> = req
            .budget_currency
            .clone()
            .flatten()
            .or_else(|| existing.as_ref().and_then(|b| b.currency.clone()));
        let eff_fee: Option<i64> = req
            .fee_minor
            .unwrap_or(existing.as_ref().and_then(|b| b.fee_minor));
        let eff_fee_ccy: Option<String> = req
            .fee_currency
            .clone()
            .flatten()
            .or_else(|| existing.as_ref().and_then(|b| b.fee_currency.clone()));
        validate_money_pair(eff_budget, eff_budget_ccy.as_deref(), "budget")?;
        validate_money_pair(eff_fee, eff_fee_ccy.as_deref(), "fee")?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "project_phase",
                entity_id: phase_id,
                version,
            },
        )
        .await?;
        let after = core_db::projects::update_phase(
            &mut *ctx.tx,
            org_id,
            phase_id,
            core_db::projects::PhasePatch {
                name: req.name.as_deref().map(str::trim),
                description: req.description.as_ref().map(|o| o.as_deref()),
                sort_order: req.sort_order,
                status: req.status,
                is_milestone: req.is_milestone,
                milestone_amount_minor: req.milestone_amount_minor,
                milestone_currency: req.milestone_currency.as_ref().map(|o| o.as_deref()),
                start_date: req.start_date,
                end_date: req.end_date,
            },
        )
        .await?;
        // Budget/fee record maintenance rides the same transaction:
        // both cleared → drop the row; any value present → upsert.
        if eff_budget.is_none() && eff_fee.is_none() {
            core_db::projects::delete_phase_budget(&mut *ctx.tx, org_id, phase_id).await?;
        } else {
            core_db::projects::upsert_phase_budget(
                &mut *ctx.tx,
                org_id,
                phase_id,
                core_db::projects::NewPhaseBudget {
                    budget_minor: eff_budget,
                    currency: eff_budget_ccy.as_deref(),
                    fee_minor: eff_fee,
                    fee_currency: eff_fee_ccy.as_deref(),
                },
            )
            .await?;
        }
        ctx.emit(
            types::PROJECT_PHASE_UPDATED,
            "project_phase",
            Some(after.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectPhaseUpdated {
                phase_id: after.id,
                project_id: after.project_id,
            })
            .unwrap_or_default(),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "project_phase",
                entity_id: phase_id,
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
            MutationOutcome::Applied(PhaseMutationResponse {
                phase: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/projects/{id}/phases/{phase_id}/complete — stamps the phase
/// completed and, when it is a billing milestone, emits
/// `project.milestone_completed` in the same transaction (the core-billing
/// seam). Instant-first: the idempotency key guarantees a retried
/// completion can never double-emit the billing event.
#[utoipa::path(operation_id = "projects_complete_phase", post, path = "/api/projects/{id}/phases/{phase_id}/complete", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("phase_id" = Uuid, Path)),
    responses((status = 200, body = PhaseMutationResponse),
              (status = 404, description = "phase not visible in scope (rls_sparse)"),
              (status = 409, description = "already completed, version conflict, or idempotency key reused")))]
pub async fn complete_phase(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, phase_id)): Path<(Uuid, Uuid)>,
    body: Option<Json<instant::MutationParams>>,
) -> ApiResult<Json<PhaseMutationResponse>> {
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
                entity: "project_phase",
                entity_id: Some(phase_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::projects::get_phase(&mut *ctx.tx, org_id, phase_id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if before.project_id != id {
            return Err(ApiError::RlsSparse);
        }
        if before.status == PhaseStatus::Completed {
            return Err(ApiError::Conflict("phase already completed".into()));
        }
        let project = core_db::projects::get_project(&mut *ctx.tx, org_id, id).await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "project_phase",
                entity_id: phase_id,
                version,
            },
        )
        .await?;
        let at = chrono::Utc::now();
        let after = core_db::projects::complete_phase(&mut *ctx.tx, org_id, phase_id, at).await?;
        ctx.emit(
            types::PROJECT_PHASE_COMPLETED,
            "project_phase",
            Some(after.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectPhaseCompleted {
                phase: after.clone(),
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
                &after.name,
                Some(after.id),
                None,
                amount,
                at,
            )
            .await?;
        } else {
            // Fixed-fee phase (report P4): a NON-milestone phase whose
            // budget record carries a fee releases it on completion —
            // directly into the ledger in this same transaction (the
            // phase-completion idempotency replay guard covers this too:
            // a retried completion replays the same mutation without
            // double-staging, guarded by the ledger's unique index).
            if let Some(budget) =
                core_db::projects::get_phase_budget(&mut *ctx.tx, org_id, phase_id).await?
            {
                if let (Some(fee_minor), Some(currency)) = (budget.fee_minor, budget.fee_currency) {
                    core_db::projects::insert_release(
                        &mut *ctx.tx,
                        &core_db::projects::NewRelease {
                            org_id,
                            project_id: after.project_id,
                            phase_id: Some(after.id),
                            task_id: None,
                            label: after.name.clone(),
                            amount_minor: fee_minor,
                            currency: currency.to_string(),
                            released_on: at.date_naive(),
                            source_event_id: None,
                        },
                    )
                    .await?;
                }
            }
        }
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "project_phase",
                entity_id: phase_id,
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
            MutationOutcome::Applied(PhaseMutationResponse {
                phase: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- task dependencies -----------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct SetDependenciesRequest {
    /// The FULL finish-to-start dependency set for the task (PUT
    /// semantics): every id must be a task of the same project; the set
    /// must not create a cycle (typed 422 names the offending edge).
    pub depends_on: Vec<Uuid>,
}

/// GET /api/projects/{id}/tasks/{task_id}/dependencies
#[utoipa::path(operation_id = "projects_get_dependencies", get, path = "/api/projects/{id}/tasks/{task_id}/dependencies", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("task_id" = Uuid, Path)),
    responses((status = 200, body = DependencyListResponse),
              (status = 404, description = "not found")))]
pub async fn get_deps(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, task_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DependencyListResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let org_id = authed.principal.user.org_id;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let task = core_db::projects::get_task(&mut *tx, org_id, task_id).await?;
    if task.project_id != id {
        return Err(ApiError::NotFound);
    }
    let depends_on = core_db::projects::list_task_deps(&mut *tx, org_id, task_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(DependencyListResponse {
        task_id,
        project_id: id,
        depends_on,
    }))
}

/// PUT /api/projects/{id}/tasks/{task_id}/dependencies — replaces the
/// whole dependency set, cycle-checked over the project graph BEFORE any
/// write; emits `project.task_updated`.
#[utoipa::path(operation_id = "projects_set_dependencies", put, path = "/api/projects/{id}/tasks/{task_id}/dependencies", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("task_id" = Uuid, Path)), request_body = SetDependenciesRequest,
    responses((status = 200, body = DependencyListResponse),
              (status = 404, description = "task not visible in scope (rls_sparse)"),
              (status = 422, description = "dependency set rejected (self-dep, cross-project, or cycle)")))]
pub async fn set_deps(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, task_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<SetDependenciesRequest>,
) -> ApiResult<Json<DependencyListResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let task = core_db::projects::get_task(&mut *ctx.tx, org_id, task_id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if task.project_id != id {
            return Err(ApiError::RlsSparse);
        }
        let existing = core_db::projects::list_project_dep_edges(&mut *ctx.tx, org_id, id).await?;
        let members: std::collections::HashSet<Uuid> =
            core_db::projects::list_project_task_ids(&mut *ctx.tx, org_id, id)
                .await?
                .into_iter()
                .collect();
        let validated =
            core_projects::deps::validate_replace(&existing, task_id, &req.depends_on, &members)
                .map_err(|r| ApiError::Validation(r.to_string()))?;
        core_db::projects::set_task_deps(&mut ctx.tx, org_id, task_id, &validated).await?;
        ctx.emit(
            types::PROJECT_TASK_UPDATED,
            "project_task",
            Some(task.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectTaskUpdated {
                task_id: task.id,
                project_id: task.project_id,
            })
            .unwrap_or_default(),
        )
        .await?;
        let depends_on = core_db::projects::list_task_deps(&mut *ctx.tx, org_id, task_id).await?;
        Ok((
            ctx,
            DependencyListResponse {
                task_id,
                project_id: id,
                depends_on,
            },
        ))
    })
    .await
    .map(Json)
}

// --- views ------------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateViewRequest {
    pub name: String,
    /// `None` = org-wide view (spans every project).
    #[schema(value_type = Option<uuid::Uuid>)]
    pub project_id: Option<Uuid>,
    /// Versioned, additive-only view spec (filter/group/sort). Consumers
    /// tolerate unknown fields.
    #[serde(default)]
    pub spec: Value,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PatchViewRequest {
    pub name: Option<String>,
    pub spec: Option<Value>,
    pub is_default: Option<bool>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct ViewListQuery {
    /// Scope to one project's views; omit for the org's ENTIRE view set.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub project_id: Option<Uuid>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ViewMutationResponse {
    #[serde(flatten)]
    pub view: ProjectView,
}

/// GET /api/project-views — one project's views, or all org views when
/// `project_id` is omitted.
#[utoipa::path(operation_id = "project_views_list", get, path = "/api/project-views", tag = "projects", security(("bearer" = [])),
    params(ViewListQuery),
    responses((status = 200, body = [ProjectView])))]
pub async fn list_views(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ViewListQuery>,
) -> ApiResult<Json<Vec<ProjectView>>> {
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
    let rows =
        core_db::projects::list_views(&mut *tx, authed.principal.user.org_id, q.project_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// POST /api/project-views — saves a view; emits `project.view_updated`.
#[utoipa::path(operation_id = "project_views_create", post, path = "/api/project-views", tag = "projects", security(("bearer" = [])),
    request_body = CreateViewRequest,
    responses((status = 200, body = ViewMutationResponse),
              (status = 404, description = "project not visible in scope (rls_sparse)"),
              (status = 409, description = "view name already in use in the scope"),
              (status = 422, description = "validation failed")))]
pub async fn create_view(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateViewRequest>,
) -> ApiResult<Json<ViewMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Create))?;
    if req.name.trim().is_empty() {
        return Err(ApiError::Validation("name is required".into()));
    }
    if !req.spec.is_object() {
        return Err(ApiError::Validation("spec must be a JSON object".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let Some(pid) = req.project_id {
            core_db::projects::get_project(&mut *ctx.tx, org_id, pid)
                .await
                .map_err(|e| instant::mutation_scope_err(e.into()))?;
        }
        // Name uniqueness per scope (org-wide views have no DB UNIQUE —
        // Postgres treats NULLs as distinct — so the app enforces both).
        let scope_taken = core_db::projects::list_views(&mut *ctx.tx, org_id, req.project_id)
            .await?
            .iter()
            .any(|v| v.name.eq_ignore_ascii_case(req.name.trim()));
        if scope_taken {
            return Err(ApiError::Conflict(
                "a view with this name already exists in the scope".into(),
            ));
        }
        let view = core_db::projects::insert_view(
            &mut *ctx.tx,
            core_db::projects::NewView {
                id: Uuid::now_v7(),
                org_id,
                project_id: req.project_id,
                name: req.name.trim(),
                spec: &req.spec,
                is_default: req.is_default,
            },
        )
        .await?;
        ctx.emit(
            types::PROJECT_VIEW_UPDATED,
            "project_view",
            Some(view.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectViewUpdated {
                view_id: view.id,
                project_id: view.project_id,
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, ViewMutationResponse { view }))
    })
    .await
    .map(Json)
}

/// PATCH /api/project-views/{id} — renames / respecs a view; emits
/// `project.view_updated`.
#[utoipa::path(operation_id = "project_views_patch", patch, path = "/api/project-views/{id}", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = PatchViewRequest,
    responses((status = 200, body = ViewMutationResponse),
              (status = 404, description = "view not visible in scope (rls_sparse)"),
              (status = 422, description = "validation failed")))]
pub async fn patch_view(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchViewRequest>,
) -> ApiResult<Json<ViewMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    if let Some(name) = req.name.as_deref() {
        if name.trim().is_empty() {
            return Err(ApiError::Validation("name must not be empty".into()));
        }
    }
    if let Some(spec) = &req.spec {
        if !spec.is_object() {
            return Err(ApiError::Validation("spec must be a JSON object".into()));
        }
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let view = core_db::projects::update_view(
            &mut *ctx.tx,
            org_id,
            id,
            core_db::projects::ViewPatch {
                name: req.name.as_deref().map(str::trim),
                spec: req.spec.as_ref(),
                is_default: req.is_default,
            },
        )
        .await?;
        ctx.emit(
            types::PROJECT_VIEW_UPDATED,
            "project_view",
            Some(view.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectViewUpdated {
                view_id: view.id,
                project_id: view.project_id,
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, ViewMutationResponse { view }))
    })
    .await
    .map(Json)
}

/// DELETE /api/project-views/{id} — emits `project.view_removed`.
#[utoipa::path(operation_id = "project_views_delete", delete, path = "/api/project-views/{id}", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200),
              (status = 404, description = "view not visible in scope (rls_sparse)")))]
pub async fn delete_view(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let deleted = core_db::projects::delete_view(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if !deleted {
            return Err(ApiError::RlsSparse);
        }
        ctx.emit(
            types::PROJECT_VIEW_REMOVED,
            "project_view",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ProjectViewRemoved { view_id: id })
                .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, serde_json::json!({ "deleted": true })))
    })
    .await
    .map(Json)
}

// --- dated Gantt (Wave 34 claims close-out, claim 6) -----------------------

/// One phase as the Gantt draws it, with its budget burn attached.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PhaseGanttView {
    pub id: Uuid,
    pub name: String,
    pub sort_order: i32,
    pub status: String,
    pub is_milestone: bool,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    /// Planned days, inclusive of both ends. `None` when undated.
    pub planned_days: Option<i64>,
    pub completed_at: Option<String>,
    pub budget_minor: Option<i64>,
    pub currency: Option<String>,
    pub consumed_minor: i64,
    pub consumed_minutes: i64,
    /// `none` under budget, `warn` at/over 80%, `over` at/over 100%.
    /// `None` for an unbudgeted phase, because "no budget" is not "under
    /// budget" and rendering it green would be a false reassurance.
    pub burn_alert: Option<String>,
}

/// The project's Gantt: dated phases as bars, undated ones as list items.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProjectGanttView {
    pub project_id: Uuid,
    /// Phases with BOTH dates set, ordered by start then sort_order.
    pub dated: Vec<PhaseGanttView>,
    /// Phases with a missing end, in sort_order. The UI shows these in the
    /// ordered list and says they are not on the timeline, rather than
    /// placing them at an invented date.
    pub undated: Vec<PhaseGanttView>,
    /// The union of dated windows, so the chart can scale to the plan
    /// instead of to "today".
    pub window_from: Option<String>,
    pub window_to: Option<String>,
}

/// GET /api/projects/{id}/gantt — the dated timeline plus its undated
/// remainder, each phase carrying its budget burn.
#[utoipa::path(operation_id = "projects_gantt", get, path = "/api/projects/{id}/gantt",
    tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = ProjectGanttView)))]
pub async fn gantt(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ProjectGanttView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let org_id = authed.principal.user.org_id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::projects::phase_gantt(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;

    let to_view = |r: core_db::projects::PhaseGanttRow| PhaseGanttView {
        id: r.id,
        name: r.name,
        sort_order: r.sort_order,
        status: r.status,
        is_milestone: r.is_milestone,
        planned_days: match (r.start_date, r.end_date) {
            (Some(s), Some(e)) => Some((e - s).num_days() + 1),
            _ => None,
        },
        start_date: r.start_date.map(|d| d.to_string()),
        end_date: r.end_date.map(|d| d.to_string()),
        completed_at: r.completed_at.map(|t| t.to_rfc3339()),
        // The alert is computed here rather than in SQL so the thresholds
        // live in one place next to the rendering that depends on them.
        burn_alert: r.budget_minor.map(|budget| {
            if budget <= 0 {
                // A zero or negative budget has no meaningful percentage, and
                // dividing by it would be the classic "infinite overspend".
                "none"
            } else if r.consumed_minor >= budget {
                "over"
            } else if r.consumed_minor.saturating_mul(100) >= budget.saturating_mul(80) {
                "warn"
            } else {
                "none"
            }
            .to_string()
        }),
        budget_minor: r.budget_minor,
        currency: r.currency,
        consumed_minor: r.consumed_minor,
        consumed_minutes: r.consumed_minutes,
    };

    // The split is the point: a bar is a plan, a list row is not a plan.
    // Everything the chart draws is dated, and everything undated is
    // reported as undated rather than being quietly placed.
    let mut dated = Vec::new();
    let mut undated = Vec::new();
    for r in rows {
        let view = to_view(r);
        if view.start_date.is_some() && view.end_date.is_some() {
            dated.push(view);
        } else {
            undated.push(view);
        }
    }
    dated.sort_by(|a, b| {
        a.start_date
            .cmp(&b.start_date)
            .then(a.sort_order.cmp(&b.sort_order))
    });

    let window_from = dated.iter().filter_map(|v| v.start_date.clone()).min();
    let window_to = dated.iter().filter_map(|v| v.end_date.clone()).max();

    Ok(Json(ProjectGanttView {
        project_id: id,
        dated,
        undated,
        window_from,
        window_to,
    }))
}

// --- working calendar ----------------------------------------------------------------

/// A project's own working pattern. Absent means the organisation's calendar
/// applies, which is the state of every project that has never needed one.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ProjectCalendarView {
    pub calendar: Option<core_domain::entities::ProjectWorkingCalendar>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct SetProjectCalendarRequest {
    /// Monday at bit 0, Sunday at bit 6. Monday to Friday is 31.
    pub weekdays: i16,
    /// Minutes of work in one working day, lunch excluded.
    pub minutes_per_day: i32,
    pub note: Option<String>,
}

/// GET /api/projects/{id}/calendar — the project's override, or `calendar:
/// null` when the organisation's calendar applies.
#[utoipa::path(operation_id = "projects_get_calendar", get, path = "/api/projects/{id}/calendar", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = ProjectCalendarView),
              (status = 404, description = "project not visible in scope (rls_sparse)")))]
pub async fn get_calendar(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ProjectCalendarView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    // Reading the project first means a missing project is a 404 rather than
    // "no override", which would read as an answer.
    core_db::projects::get_project(&mut *tx, authed.principal.user.org_id, id).await?;
    let calendar =
        core_db::projects::get_working_calendar(&mut *tx, authed.principal.user.org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(ProjectCalendarView { calendar }))
}

/// PUT /api/projects/{id}/calendar — sets the project's working pattern. The
/// project's pattern wins for its own dates; nothing else changes.
///
/// No outbox event: the calendar is read when a date is computed, so nothing
/// downstream holds a stale copy of it.
#[utoipa::path(operation_id = "projects_set_calendar", put, path = "/api/projects/{id}/calendar", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = SetProjectCalendarRequest,
    responses((status = 200, body = ProjectCalendarView),
              (status = 404, description = "project not visible in scope (rls_sparse)"),
              (status = 422, description = "validation failed")))]
pub async fn set_calendar(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<SetProjectCalendarRequest>,
) -> ApiResult<Json<ProjectCalendarView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    if !(1..=127).contains(&req.weekdays) {
        return Err(ApiError::Validation(
            "weekdays must be a non-empty mask between 1 and 127".into(),
        ));
    }
    if !(1..=1440).contains(&req.minutes_per_day) {
        return Err(ApiError::Validation(
            "minutes_per_day must be between 1 and 1440".into(),
        ));
    }
    let note = req.note.as_deref().map(str::trim).filter(|n| !n.is_empty());
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let calendar = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::projects::get_project(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let calendar = core_db::projects::upsert_working_calendar(
            &mut *ctx.tx,
            org_id,
            id,
            req.weekdays,
            req.minutes_per_day,
            note,
        )
        .await?;
        Ok((ctx, calendar))
    })
    .await?;

    Ok(Json(ProjectCalendarView {
        calendar: Some(calendar),
    }))
}

/// DELETE /api/projects/{id}/calendar — drops the override so the
/// organisation's calendar applies again.
#[utoipa::path(operation_id = "projects_clear_calendar", delete, path = "/api/projects/{id}/calendar", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Value),
              (status = 404, description = "no override to clear")))]
pub async fn clear_calendar(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let deleted = core_db::projects::delete_working_calendar(&mut *ctx.tx, org_id, id).await?;
        if !deleted {
            return Err(ApiError::RlsSparse);
        }
        Ok((ctx, serde_json::json!({ "deleted": true })))
    })
    .await
    .map(Json)
}

// --- schedule: preview, apply, revert, scale -----------------------------------------

/// Resolves the calendar a project's dates are computed against: the project's
/// own pattern where it has one, otherwise the organisation's week, otherwise
/// the standard Monday-to-Friday day.
pub(crate) async fn project_work_calendar(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    project_id: Uuid,
) -> ApiResult<core_projects::schedule::WorkCalendar> {
    let project = core_db::projects::get_working_calendar(&mut *tx, org_id, project_id).await?;
    if let Some(cal) = project {
        return Ok(core_projects::schedule::WorkCalendar::new(
            cal.weekdays as u8,
            cal.minutes_per_day as u32,
        ));
    }
    let org = core_db::projects::get_org_working_calendar(&mut *tx, org_id).await?;
    Ok(org
        .map(|cal| {
            core_projects::schedule::WorkCalendar::new(
                cal.weekdays as u8,
                cal.minutes_per_day as u32,
            )
        })
        .unwrap_or(core_projects::schedule::WorkCalendar::STANDARD))
}

/// Loads a project's task graph in the shape the engine consumes.
async fn load_task_nodes(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    project_id: Uuid,
) -> ApiResult<Vec<core_projects::schedule::TaskNode>> {
    let tasks = core_db::projects::list_tasks(
        &mut *tx,
        org_id,
        project_id,
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
    let edges = core_db::projects::list_project_dep_edges(&mut *tx, org_id, project_id).await?;
    let mut deps: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for (task_id, depends_on) in edges {
        deps.entry(task_id).or_default().push(depends_on);
    }
    Ok(tasks
        .into_iter()
        .map(|t| core_projects::schedule::TaskNode {
            id: t.id,
            start_at: t.start_at,
            duration_minutes: t.duration_minutes,
            deps: deps.remove(&t.id).unwrap_or_default(),
        })
        .collect())
}

/// One task the coordinator placed. `duration_minutes: None` means "leave the
/// length alone"; `Some(None)` clears it together with the start.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ScheduleMove {
    #[schema(value_type = uuid::Uuid)]
    pub task_id: Uuid,
    pub start_at: Option<chrono::DateTime<chrono::Utc>>,
    pub duration_minutes: Option<Option<i32>>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct SchedulePlanRequest {
    pub moves: Vec<ScheduleMove>,
}

/// One change the engine proposes or applied.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ScheduleChangeView {
    #[schema(value_type = uuid::Uuid)]
    pub task_id: Uuid,
    pub from_start: Option<chrono::DateTime<chrono::Utc>>,
    pub to_start: Option<chrono::DateTime<chrono::Utc>>,
    /// `moved` for the task the coordinator placed, `pushed` for a successor
    /// the move displaced.
    pub reason: String,
}

/// Someone else's work that would sit at the same time as the proposed move.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ScheduleCollisionView {
    #[schema(value_type = uuid::Uuid)]
    pub task_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub project_id: Uuid,
    pub project_name: String,
    pub task_title: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub assignee_id: Option<Uuid>,
    pub start_at: chrono::DateTime<chrono::Utc>,
    pub end_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct SchedulePlanResponse {
    pub changes: Vec<ScheduleChangeView>,
    /// The longest chain through the plan, in order. Empty when nothing is
    /// dated.
    pub critical_path: Vec<Uuid>,
    /// Work in other projects, assigned to the same people, that would overlap
    /// the moved window. Empty means nothing collides.
    #[serde(default)]
    pub collisions: Vec<ScheduleCollisionView>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ScheduleApplyResponse {
    pub changes: Vec<ScheduleChangeView>,
    pub critical_path: Vec<Uuid>,
    #[serde(default)]
    pub collisions: Vec<ScheduleCollisionView>,
    /// What each changed task held before the apply, so a revert restores
    /// exactly these values.
    pub revert: Vec<ScheduleRestore>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ScheduleRestore {
    #[schema(value_type = uuid::Uuid)]
    pub task_id: Uuid,
    pub start_at: Option<chrono::DateTime<chrono::Utc>>,
    pub duration_minutes: Option<i32>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ScheduleRevertRequest {
    pub restore: Vec<ScheduleRestore>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ScheduleScaleRequest {
    /// Multiplier applied to every dated task in range.
    pub factor: f64,
    pub from_start_at: Option<chrono::DateTime<chrono::Utc>>,
    pub to_start_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Scale durations without moving anything, or also reschedule the plan
    /// from the earliest changed task.
    #[serde(default)]
    pub reschedule: bool,
}

fn to_view(change: &core_projects::schedule::Change) -> ScheduleChangeView {
    ScheduleChangeView {
        task_id: change.task_id,
        from_start: change.from_start,
        to_start: change.to_start,
        reason: match change.reason {
            core_projects::schedule::ChangeReason::Moved => "moved".into(),
            core_projects::schedule::ChangeReason::Pushed => "pushed".into(),
        },
    }
}

/// Runs every move through the engine in the order the coordinator made them,
/// so a two-step drag previews the same result a two-step drag applies.
fn compute_plan(
    nodes: &[core_projects::schedule::TaskNode],
    calendar: core_projects::schedule::WorkCalendar,
    moves: &[ScheduleMove],
) -> Vec<core_projects::schedule::Change> {
    let mut current: Vec<core_projects::schedule::TaskNode> = nodes.to_vec();
    let mut changes: Vec<core_projects::schedule::Change> = Vec::new();
    for mv in moves {
        let step = core_projects::schedule::plan_move(&current, calendar, mv.task_id, mv.start_at);
        for change in &step {
            if let Some(node) = current.iter_mut().find(|n| n.id == change.task_id) {
                node.start_at = change.to_start;
                if change.task_id == mv.task_id {
                    if let Some(duration) = mv.duration_minutes {
                        node.duration_minutes = duration;
                    }
                }
            }
            match changes.iter_mut().find(|c| c.task_id == change.task_id) {
                // A task moved twice in one plan keeps its original `from`, so
                // the revert list still restores the state before the plan.
                Some(existing) => existing.to_start = change.to_start,
                None => changes.push(change.clone()),
            }
        }
    }
    changes
}

/// Finds the other projects' commitments that the proposed changes would run
/// into. Only people, not roles: a role's placeholder is not a booking.
async fn find_collisions(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    project_id: Uuid,
    calendar: core_projects::schedule::WorkCalendar,
    nodes: &[core_projects::schedule::TaskNode],
    changes: &[core_projects::schedule::Change],
) -> ApiResult<Vec<ScheduleCollisionView>> {
    use chrono::Duration;
    let tasks = core_db::projects::list_tasks(
        &mut *tx,
        org_id,
        project_id,
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
    let mut assignees: Vec<Uuid> = Vec::new();
    let mut from: Option<chrono::DateTime<chrono::Utc>> = None;
    let mut to: Option<chrono::DateTime<chrono::Utc>> = None;
    for change in changes {
        let Some(task) = tasks.iter().find(|t| t.id == change.task_id) else {
            continue;
        };
        if let Some(assignee) = task.assignee_id {
            if !assignees.contains(&assignee) {
                assignees.push(assignee);
            }
        }
        let Some(start) = change.to_start else {
            continue;
        };
        let duration = task.duration_minutes.unwrap_or(0).max(0);
        let end = core_projects::schedule::add_working_minutes(start, duration as u32, calendar);
        from = Some(from.map_or(start, |current| current.min(start)));
        to = Some(to.map_or(end, |current| current.max(end)));
    }
    let _ = nodes;
    let (Some(from), Some(to)) = (from, to) else {
        return Ok(Vec::new());
    };
    // A day of slack either side catches work that starts just outside the
    // window but still overlaps it.
    let rows = core_db::projects::list_overlapping_assignments(
        tx,
        org_id,
        project_id,
        &assignees,
        from - Duration::days(1),
        to + Duration::days(1),
    )
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| ScheduleCollisionView {
            task_id: row.task_id,
            project_id: row.project_id,
            project_name: row.project_name,
            task_title: row.task_title,
            assignee_id: row.assignee_id,
            start_at: row.start_at,
            end_at: core_projects::schedule::add_working_minutes(
                row.start_at,
                row.duration_minutes.max(0) as u32,
                calendar,
            ),
        })
        .collect())
}

/// POST /api/projects/{id}/schedule/preview — what a set of moves would do,
/// computed by the same function the apply calls. Nothing is written.
#[utoipa::path(operation_id = "projects_schedule_preview", post, path = "/api/projects/{id}/schedule/preview", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = SchedulePlanRequest,
    responses((status = 200, body = SchedulePlanResponse),
              (status = 404, description = "project not visible in scope (rls_sparse)"),
              (status = 422, description = "validation failed")))]
pub async fn schedule_preview(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<SchedulePlanRequest>,
) -> ApiResult<Json<SchedulePlanResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let org_id = authed.principal.user.org_id;
    core_db::projects::get_project(&mut *tx, org_id, id).await?;
    let calendar = project_work_calendar(&mut tx, org_id, id).await?;
    let nodes = load_task_nodes(&mut tx, org_id, id).await?;
    let changes = compute_plan(&nodes, calendar, &req.moves);
    let critical_path = core_projects::schedule::critical_path(&nodes);
    let collisions = find_collisions(&mut tx, org_id, id, calendar, &nodes, &changes).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(SchedulePlanResponse {
        changes: changes.iter().map(to_view).collect(),
        critical_path,
        collisions,
    }))
}

/// POST /api/projects/{id}/schedule/apply — applies the same computation the
/// preview returned, in one transaction, and hands back what to send to
/// `/schedule/revert` to undo it.
#[utoipa::path(operation_id = "projects_schedule_apply", post, path = "/api/projects/{id}/schedule/apply", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = SchedulePlanRequest,
    responses((status = 200, body = ScheduleApplyResponse),
              (status = 404, description = "a task or the project is not visible in scope (rls_sparse)"),
              (status = 422, description = "validation failed")))]
pub async fn schedule_apply(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<SchedulePlanRequest>,
) -> ApiResult<Json<ScheduleApplyResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    for mv in &req.moves {
        if let Some(duration) = mv.duration_minutes {
            validate_task_time(mv.start_at.is_some(), duration)?;
        }
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let response = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::projects::get_project(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let calendar = project_work_calendar(&mut ctx.tx, org_id, id).await?;
        let nodes = load_task_nodes(&mut ctx.tx, org_id, id).await?;
        let changes = compute_plan(&nodes, calendar, &req.moves);

        let mut revert = Vec::with_capacity(changes.len());
        for change in &changes {
            let before = nodes
                .iter()
                .find(|n| n.id == change.task_id)
                .ok_or(ApiError::RlsSparse)?;
            revert.push(ScheduleRestore {
                task_id: change.task_id,
                start_at: before.start_at,
                duration_minutes: before.duration_minutes,
            });
            let duration = req
                .moves
                .iter()
                .find(|m| m.task_id == change.task_id)
                .and_then(|m| m.duration_minutes)
                .unwrap_or(before.duration_minutes);
            core_db::projects::update_task(
                &mut *ctx.tx,
                org_id,
                change.task_id,
                core_db::projects::TaskPatch {
                    start_at: Some(change.to_start),
                    duration_minutes: Some(duration),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
            ctx.emit(
                types::PROJECT_TASK_UPDATED,
                "project_task",
                Some(change.task_id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::ProjectTaskUpdated {
                    task_id: change.task_id,
                    project_id: id,
                })
                .unwrap_or_default(),
            )
            .await?;
        }
        let critical_path = core_projects::schedule::critical_path(&nodes);
        Ok((
            ctx,
            ScheduleApplyResponse {
                changes: changes.iter().map(to_view).collect(),
                critical_path,
                collisions: Vec::new(),
                revert,
            },
        ))
    })
    .await?;

    Ok(Json(response))
}

/// POST /api/projects/{id}/schedule/revert — restores the values an apply
/// handed back. A task outside the project is refused rather than skipped.
#[utoipa::path(operation_id = "projects_schedule_revert", post, path = "/api/projects/{id}/schedule/revert", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = ScheduleRevertRequest,
    responses((status = 200, body = Value),
              (status = 404, description = "a task or the project is not visible in scope (rls_sparse)"),
              (status = 422, description = "validation failed")))]
pub async fn schedule_revert(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<ScheduleRevertRequest>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    for restore in &req.restore {
        validate_task_time(restore.start_at.is_some(), restore.duration_minutes)?;
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::projects::get_project(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let mut restored = 0_usize;
        for point in &req.restore {
            let task = core_db::projects::get_task(&mut *ctx.tx, org_id, point.task_id)
                .await
                .map_err(|e| instant::mutation_scope_err(e.into()))?;
            if task.project_id != id {
                return Err(ApiError::Validation(
                    "a task in the revert list belongs to a different project".into(),
                ));
            }
            core_db::projects::update_task(
                &mut *ctx.tx,
                org_id,
                point.task_id,
                core_db::projects::TaskPatch {
                    start_at: Some(point.start_at),
                    duration_minutes: Some(point.duration_minutes),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
            ctx.emit(
                types::PROJECT_TASK_UPDATED,
                "project_task",
                Some(point.task_id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::ProjectTaskUpdated {
                    task_id: point.task_id,
                    project_id: id,
                })
                .unwrap_or_default(),
            )
            .await?;
            restored += 1;
        }
        Ok((ctx, serde_json::json!({ "restored": restored })))
    })
    .await
    .map(Json)
}

/// POST /api/projects/{id}/schedule/scale — multiplies the length of every
/// dated task in a window. `reschedule` re-ripples the plan from each changed
/// task so successors follow the new lengths.
#[utoipa::path(operation_id = "projects_schedule_scale", post, path = "/api/projects/{id}/schedule/scale", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = ScheduleScaleRequest,
    responses((status = 200, body = ScheduleApplyResponse),
              (status = 404, description = "project not visible in scope (rls_sparse)"),
              (status = 422, description = "validation failed")))]
pub async fn schedule_scale(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<ScheduleScaleRequest>,
) -> ApiResult<Json<ScheduleApplyResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    if !(req.factor.is_finite() && req.factor > 0.0 && req.factor <= 10.0) {
        return Err(ApiError::Validation(
            "factor must be greater than zero and at most ten".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let response = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::projects::get_project(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let calendar = project_work_calendar(&mut ctx.tx, org_id, id).await?;
        let nodes = load_task_nodes(&mut ctx.tx, org_id, id).await?;
        let mut changes: Vec<core_projects::schedule::Change> = Vec::new();
        let mut revert: Vec<ScheduleRestore> = Vec::new();

        for node in &nodes {
            let (Some(start), Some(duration)) = (node.start_at, node.duration_minutes) else {
                continue;
            };
            if let Some(from) = req.from_start_at {
                if start < from {
                    continue;
                }
            }
            if let Some(to) = req.to_start_at {
                if start > to {
                    continue;
                }
            }
            let scaled = core_projects::schedule::scale_minutes(duration, req.factor);
            if scaled == duration {
                continue;
            }
            revert.push(ScheduleRestore {
                task_id: node.id,
                start_at: node.start_at,
                duration_minutes: node.duration_minutes,
            });
            core_db::projects::update_task(
                &mut *ctx.tx,
                org_id,
                node.id,
                core_db::projects::TaskPatch {
                    duration_minutes: Some(Some(scaled)),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
            changes.push(core_projects::schedule::Change {
                task_id: node.id,
                from_start: node.start_at,
                to_start: node.start_at,
                reason: core_projects::schedule::ChangeReason::Moved,
            });
        }

        if req.reschedule && !changes.is_empty() {
            // Re-read with the new lengths, then ripple each scaled task in
            // plan order so the successors land where the plan says.
            let scaled_nodes = load_task_nodes(&mut ctx.tx, org_id, id).await?;
            let mut rippled: Vec<core_projects::schedule::TaskNode> = scaled_nodes.clone();
            let order: Vec<Uuid> = changes.iter().map(|c| c.task_id).collect();
            for task_id in order {
                let start = rippled
                    .iter()
                    .find(|n| n.id == task_id)
                    .and_then(|n| n.start_at);
                let step = core_projects::schedule::plan_move(&rippled, calendar, task_id, start);
                for change in step {
                    if let Some(node) = rippled.iter_mut().find(|n| n.id == change.task_id) {
                        node.start_at = change.to_start;
                    }
                }
            }
            for node in &rippled {
                let before = scaled_nodes.iter().find(|n| n.id == node.id);
                let Some(before) = before else { continue };
                if before.start_at == node.start_at {
                    continue;
                }
                changes.push(core_projects::schedule::Change {
                    task_id: node.id,
                    from_start: before.start_at,
                    to_start: node.start_at,
                    reason: core_projects::schedule::ChangeReason::Pushed,
                });
                core_db::projects::update_task(
                    &mut *ctx.tx,
                    org_id,
                    node.id,
                    core_db::projects::TaskPatch {
                        start_at: Some(node.start_at),
                        ..Default::default()
                    },
                )
                .await
                .map_err(|e| instant::mutation_scope_err(e.into()))?;
            }
            for change in &changes {
                if change.reason == core_projects::schedule::ChangeReason::Pushed {
                    revert.push(ScheduleRestore {
                        task_id: change.task_id,
                        start_at: change.from_start,
                        duration_minutes: scaled_nodes
                            .iter()
                            .find(|n| n.id == change.task_id)
                            .and_then(|n| n.duration_minutes),
                    });
                }
                ctx.emit(
                    types::PROJECT_TASK_UPDATED,
                    "project_task",
                    Some(change.task_id.to_string().as_str()),
                    &serde_json::to_value(core_domain::events::ProjectTaskUpdated {
                        task_id: change.task_id,
                        project_id: id,
                    })
                    .unwrap_or_default(),
                )
                .await?;
            }
        } else {
            for change in &changes {
                ctx.emit(
                    types::PROJECT_TASK_UPDATED,
                    "project_task",
                    Some(change.task_id.to_string().as_str()),
                    &serde_json::to_value(core_domain::events::ProjectTaskUpdated {
                        task_id: change.task_id,
                        project_id: id,
                    })
                    .unwrap_or_default(),
                )
                .await?;
            }
        }

        let critical_path = core_projects::schedule::critical_path(&nodes);
        Ok((
            ctx,
            ScheduleApplyResponse {
                changes: changes.iter().map(to_view).collect(),
                critical_path,
                collisions: Vec::new(),
                revert,
            },
        ))
    })
    .await?;

    Ok(Json(response))
}

// --- calendar zones ------------------------------------------------------------------

/// A stretch of the calendar that constrains where work may sit.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CalendarZoneView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    /// `window` for an agreed change window, `closed` for a client closure.
    pub kind: String,
    pub label: String,
    pub starts_on: chrono::NaiveDate,
    pub ends_on: chrono::NaiveDate,
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CalendarZonesResponse {
    pub zones: Vec<CalendarZoneView>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateCalendarZoneRequest {
    /// `window` or `closed`.
    pub kind: String,
    pub label: String,
    pub starts_on: chrono::NaiveDate,
    pub ends_on: chrono::NaiveDate,
    pub note: Option<String>,
}

fn zone_view(zone: core_db::projects::CalendarZone) -> CalendarZoneView {
    CalendarZoneView {
        id: zone.id,
        kind: zone.kind,
        label: zone.label,
        starts_on: zone.starts_on,
        ends_on: zone.ends_on,
        note: zone.note,
    }
}

/// GET /api/projects/{id}/zones — the change windows and client closures the
/// timeline draws under the bars.
#[utoipa::path(operation_id = "projects_calendar_zones", get, path = "/api/projects/{id}/zones", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = CalendarZonesResponse),
              (status = 404, description = "project not visible in scope (rls_sparse)")))]
pub async fn list_zones(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<CalendarZonesResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    core_db::projects::get_project(&mut *tx, org_id, id).await?;
    let zones = core_db::projects::list_calendar_zones(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(CalendarZonesResponse {
        zones: zones.into_iter().map(zone_view).collect(),
    }))
}

/// POST /api/projects/{id}/zones — states a window or a closure.
#[utoipa::path(operation_id = "projects_create_calendar_zone", post, path = "/api/projects/{id}/zones", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = CreateCalendarZoneRequest,
    responses((status = 200, body = CalendarZoneView),
              (status = 404, description = "project not visible in scope (rls_sparse)"),
              (status = 422, description = "validation failed")))]
pub async fn create_zone(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateCalendarZoneRequest>,
) -> ApiResult<Json<CalendarZoneView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    if !matches!(req.kind.as_str(), "window" | "closed") {
        return Err(ApiError::Validation(
            "kind must be 'window' or 'closed'".into(),
        ));
    }
    if req.label.trim().is_empty() {
        return Err(ApiError::Validation("label is required".into()));
    }
    if req.ends_on < req.starts_on {
        return Err(ApiError::Validation(
            "ends_on must not be before starts_on".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let note = req.note.as_deref().map(str::trim).filter(|n| !n.is_empty());

    let zone = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::projects::get_project(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let zone = core_db::projects::insert_calendar_zone(
            &mut *ctx.tx,
            core_db::projects::NewCalendarZone {
                org_id,
                project_id: id,
                kind: &req.kind,
                label: req.label.trim(),
                starts_on: req.starts_on,
                ends_on: req.ends_on,
                note,
            },
        )
        .await?;
        Ok((ctx, zone))
    })
    .await?;

    Ok(Json(zone_view(zone)))
}

/// DELETE /api/projects/{id}/zones/{zone_id} — removes the constraint.
#[utoipa::path(operation_id = "projects_delete_calendar_zone", delete, path = "/api/projects/{id}/zones/{zone_id}", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("zone_id" = Uuid, Path)),
    responses((status = 200, body = Value),
              (status = 404, description = "no such zone in scope")))]
pub async fn delete_zone(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, zone_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let deleted =
            core_db::projects::delete_calendar_zone(&mut *ctx.tx, org_id, id, zone_id).await?;
        if !deleted {
            return Err(ApiError::RlsSparse);
        }
        Ok((ctx, serde_json::json!({ "deleted": true })))
    })
    .await
    .map(Json)
}
