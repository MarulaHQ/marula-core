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

/// The release ledger row as the CALLER may see it. The amount and its currency
/// are financial data: both serialize `null` without `financial:read` (RB2,
/// ADR 0004), while the release itself stays visible so the plan reads.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BillingReleaseView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub org_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub project_id: Uuid,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub phase_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub task_id: Option<Uuid>,
    pub label: String,
    /// `null` without `financial:read` (RB2).
    #[schema(nullable = true)]
    pub amount_minor: Option<i64>,
    #[schema(nullable = true)]
    pub currency: Option<String>,
    pub released_on: chrono::NaiveDate,
    pub status: core_domain::entities::ReleaseStatus,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub invoice_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub invoice_line_id: Option<Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl BillingReleaseView {
    fn visible(release: core_domain::entities::ProjectBillingRelease, financial: bool) -> Self {
        Self {
            id: release.id,
            org_id: release.org_id,
            project_id: release.project_id,
            phase_id: release.phase_id,
            task_id: release.task_id,
            label: release.label,
            amount_minor: financial.then_some(release.amount_minor),
            currency: financial.then_some(release.currency),
            released_on: release.released_on,
            status: release.status,
            invoice_id: release.invoice_id,
            invoice_line_id: release.invoice_line_id,
            created_at: release.created_at,
            updated_at: release.updated_at,
        }
    }
}

/// GET /api/projects/{id}/billing-releases — the release ledger: every
/// staged milestone/fee for this project with its status (`pending`
/// bills on the next run reaching its window; `billed` carries the
/// consuming invoice; `cancelled` terminal).
#[utoipa::path(operation_id = "projects_billing_releases", get, path = "/api/projects/{id}/billing-releases", tag = "projects", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [BillingReleaseView])))]
pub async fn list_billing_releases(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<BillingReleaseView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, projects(Action::View))?;
    let financial =
        core_domain::policy::check(&authed.principal.permissions, Permission::ViewFinancials)
            .is_allowed();
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
    Ok(Json(
        rows.into_iter()
            .map(|row| BillingReleaseView::visible(row, financial))
            .collect(),
    ))
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
    // Cancelling a release changes money that would be billed: the same
    // financial gate the ledger read carries (RB2).
    require_permission(&authed.principal, Permission::ViewFinancials)?;
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
