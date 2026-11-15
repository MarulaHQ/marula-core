use super::*;

// --- phase burn + billing releases (Wave 39 lane 4) -------------------------

/// GET /api/projects/{id}/phase-burn — the burn read model (report P3):
/// consumed approved billable minutes and card-priced value per phase
/// against the phase's budget, with the typed `none | warn | over`
/// alert state (`warn` at ≥80%, `over` at ≥100%).
#[utoipa::path(operation_id = "projects_phase_burn", get, path = "/api/projects/{id}/phase-burn", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [core_domain::entities::PhaseBurn]),
              (status = 404, description = "project not visible in scope (rls_sparse)")))]
pub async fn phase_burn(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<core_domain::entities::PhaseBurn>>> {
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
    let burns = core_db::projects::phase_burn(&mut *tx, authed.principal.user.org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(burns))
}

/// GET /api/projects/{id}/billing-releases — the release ledger: every
/// staged milestone/fee for this project with its status (`pending`
/// bills on the next run reaching its window; `billed` carries the
/// consuming invoice; `cancelled` terminal).
#[utoipa::path(operation_id = "projects_billing_releases", get, path = "/api/projects/{id}/billing-releases", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [core_domain::entities::ProjectBillingRelease])))]
pub async fn list_billing_releases(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<core_domain::entities::ProjectBillingRelease>>> {
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
        core_db::projects::list_releases_for_project(&mut *tx, authed.principal.user.org_id, id)
            .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// POST /api/projects/{id}/billing-releases/{release_id}/cancel — cancel
/// a PENDING release (admin correction; terminal). Billed releases are
/// immutable — they ride an invoice, and invoices correct through credit
/// notes.
#[utoipa::path(operation_id = "projects_billing_release_cancel", post, path = "/api/projects/{id}/billing-releases/{release_id}/cancel", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("release_id" = Uuid, Path)),
    responses((status = 200, body = core_domain::entities::ProjectBillingRelease),
              (status = 404, description = "release not visible/already settled (rls_sparse)")))]
pub async fn cancel_billing_release(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, release_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<core_domain::entities::ProjectBillingRelease>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    // RLS-scoped transaction; the project id scopes the read so a
    // release from another project 404s (rls_sparse).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    core_db::projects::get_project(&mut *tx, org_id, id).await?;
    let release = core_db::projects::cancel_release(&mut tx, org_id, release_id)
        .await
        .map_err(|_| ApiError::RlsSparse)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(release))
}
