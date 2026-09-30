//! Time categories (Wave 39 lane 1, migration 0110): the org's semantic
//! time taxonomy and the capture/verification seams around it.
//!
//! Three surfaces, one module:
//!
//! 1. **Admin taxonomy CRUD** — create/update/retire/reorder category
//!    rows. Permissions: reads ride the Time matrix (View); writes gate
//!    on `special:manage_billing` (the billing semantics the taxonomy
//!    encodes live with billing, not boards). Mutations ride the
//!    IF1–IF3 instant-first envelope and emit their domain event +
//!    audit row in the same commit. Slug is identity — never editable.
//!    Delete is a soft retire (entries keep their slug reference).
//!
//! 2. **Capture-time help** — `GET /time-categories/capture-defaults`
//!    returns the ordered picker plus per-category billing defaults so
//!    the wrap-up sheet and quick-log can preselect from ticket
//!    context (board slug → category slug match, travel/expenses
//!    affinity); the entry create/stop bodies grow additive
//!    `work_type` ( désormais the category slug) and
//!    `unbillable_reason` fields validated against the org taxonomy.
//!
//! 3. **Billing-audit suggestion surfacing** — the
//!    `decision_billing_audit` shadow answers (decision.rs) read back
//!    per entry as classified suggestion chips. SUGGESTION-GRADE ONLY:
//!    the endpoint is a plain RLS'd read; chips never mutate
//!    anything. One entry or a bulk read for approvals rows.
//!
//! Non-purpose: no rate-card mutation here (contracts stay
//! contract-owned; the affinity flag is documentation), no entry
//! adjustment approvals flow (lane 2's verification workspace), no
//! auto-application of suggestions (never).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    routing::{get, patch, post},
    Json, Router,
};
use core_db::time_categories::{NewTimeCategory, WorkTypeMapRow};
use core_domain::entities::{Actor, AuditEvent, TimeCategory, TimeVerdict};
use core_domain::events::types;
use core_domain::policy::{Module, Permission};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

pub fn routes(gen: crate::routes::ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(
            &format!("{base}/time-categories"),
            get(list_categories).post(create_category),
        )
        .route(
            &format!("{base}/time-categories/reorder"),
            post(reorder_categories),
        )
        .route(
            &format!("{base}/time-categories/capture-defaults"),
            get(capture_defaults),
        )
        .route(
            &format!("{base}/time-categories/work-type-map"),
            get(list_work_type_map).post(upsert_work_type_map),
        )
        .route(
            &format!("{base}/time-categories/work-type-map/apply"),
            post(apply_work_type_map),
        )
        .route(
            &format!("{base}/time-categories/{{id}}"),
            patch(update_category).delete(delete_category),
        )
        .route(
            &format!("{base}/time-entries/{{id}}/billing-suggestions"),
            get(entry_suggestions),
        )
        .route(
            &format!("{base}/time-entries/billing-suggestions"),
            get(bulk_entry_suggestions),
        )
}

fn time_perm(action: core_domain::policy::Action) -> Permission {
    Permission::Matrix {
        module: Module::Time,
        action,
    }
}

fn manage_billing() -> Permission {
    Permission::ManageBilling
}

const SLUG_MAX: usize = 48;
const LABEL_MAX: usize = 80;
const DESCRIPTION_MAX: usize = 500;

fn validate_slug(slug: &str) -> ApiResult<String> {
    let slug = slug.trim().to_lowercase();
    if slug.is_empty() || slug.len() > SLUG_MAX {
        return Err(ApiError::Validation(format!(
            "slug must be 1..{SLUG_MAX} characters"
        )));
    }
    let bytes = slug.as_bytes();
    let ok = bytes[0].is_ascii_lowercase()
        && bytes[1..]
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_');
    if !ok {
        return Err(ApiError::Validation(
            "slug must be lowercase snake_case (letters, digits, underscores)".into(),
        ));
    }
    Ok(slug)
}

fn validate_label(label: &str) -> ApiResult<String> {
    let label = label.trim();
    if label.is_empty() || label.len() > LABEL_MAX {
        return Err(ApiError::Validation(format!(
            "label must be 1..{LABEL_MAX} characters"
        )));
    }
    Ok(label.to_string())
}

fn validate_description(description: Option<&str>) -> ApiResult<Option<String>> {
    match description {
        Some(d) if d.len() > DESCRIPTION_MAX => Err(ApiError::Validation(format!(
            "description must be at most {DESCRIPTION_MAX} characters"
        ))),
        Some(d) => Ok(Some(d.trim().to_string())),
        None => Ok(None),
    }
}

/// Suggestion payload built from one `decision_billing_audit` row.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EntrySuggestion {
    /// The suggested classification chip: how to interpret the audit.
    /// `billable_false` = "looks non-billable", `billable_true` =
    /// "looks billable". Derived from the `billable` answer's
    /// threshold state — never auto-applied.
    pub chip: String,
    /// Human-readable chip text the UI renders (typed, no model prose).
    pub label: String,
    /// The audit's `billable` probability, when recorded.
    pub billable_probability: Option<f64>,
    /// `act` | `pending` | `ignore` — threshold gate at read time.
    pub state: Option<String>,
    /// `shadow` rows drive nothing and are labeled as such.
    pub mode: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub recorded_at: chrono::DateTime<chrono::Utc>,
    /// The server's mechanical verdicts for this entry (Wave 41 lane 8), each
    /// with the one action that clears it. Empty means clean — and empty for
    /// an entry that is not visible in scope either, because RLS proves
    /// visibility by absence, never by an error.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verdicts: Vec<TimeVerdict>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EntrySuggestionsResponse {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub entry_id: Uuid,
    /// Empty when the surface is unconfigured or no answers exist —
    /// the honest visibly-absent state.
    pub suggestions: Vec<EntrySuggestion>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BulkEntrySuggestionsResponse {
    pub items: Vec<EntrySuggestionsResponse>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct BulkSuggestionsQuery {
    /// Comma-separated entry ids (≤ 100 — approvals page batches).
    pub ids: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TimeCategoryView {
    #[serde(flatten)]
    pub category: TimeCategory,
    /// Live count of entries carrying this slug (settings usage read).
    pub entry_count: i64,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TimeCategoryMutationResponse {
    pub category: TimeCategoryView,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TimeCategoryListResponse {
    pub categories: Vec<TimeCategoryView>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TimeCategoryCreateRequest {
    #[serde(flatten)]
    pub instant: instant::MutationParams,
    pub slug: String,
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_true")]
    pub default_billable: bool,
    #[serde(default)]
    pub requires_unbillable_reason: bool,
    #[serde(default)]
    pub rate_card_affinity: bool,
    #[serde(default)]
    pub position: Option<i32>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TimeCategoryUpdateRequest {
    #[serde(flatten)]
    pub instant: instant::MutationParams,
    pub label: Option<String>,
    /// Some → set (null clears); absent → no change.
    #[serde(default)]
    pub description: Option<Option<String>>,
    pub default_billable: Option<bool>,
    pub requires_unbillable_reason: Option<bool>,
    pub rate_card_affinity: Option<bool>,
    /// Un-retire a soft-retired category. `Some(false)` restores it to
    /// the pickers; retirement is reversible because entries and rate
    /// cards keep the slug.
    #[serde(default)]
    pub retired: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReorderCategoriesRequest {
    #[serde(flatten)]
    pub instant: instant::MutationParams,
    /// Category ids in their new display order.
    pub ordered_ids: Vec<Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkTypeMapRowWire {
    pub legacy_value: String,
    pub category_slug: String,
    /// `backfill` (the one-time migration mapping) or `manual` (an
    /// admin alias added later).
    pub source: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UpsertWorkTypeMapRequest {
    #[serde(flatten)]
    pub instant: instant::MutationParams,
    pub legacy_value: String,
    pub category_slug: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ApplyWorkTypeMapRequest {
    #[serde(flatten)]
    pub instant: instant::MutationParams,
    /// Free-text value to recategorize (case-insensitive, trimmed).
    pub legacy_value: String,
    pub category_slug: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ApplyWorkTypeMapResponse {
    pub entries_updated: i64,
    pub mutation: instant::MutationMeta,
}

/// The capture surface's read: ordered picker + theHELP defaults.
#[derive(Debug, Serialize, ToSchema)]
pub struct CaptureDefaultsResponse {
    /// Non-retired categories ordered by position (the picker).
    pub categories: Vec<TimeCategory>,
    /// The backfill/manual mapping (legacy value → slug) so capture
    /// surfaces can normalize pasted free text honestly.
    pub work_type_map: Vec<WorkTypeMapRowWire>,
}

/// GET /api/time-categories — the org's taxonomy (admin + pickers).
#[utoipa::path(operation_id = "time_categories_list", get, path = "/api/time-categories", tag = "time", security(("bearer" = [])),
    params(("include_retired" = Option<bool>, Query, description = "Include retired categories (settings admin reads)")),
    responses((status = 200, body = TimeCategoryListResponse)))]
pub async fn list_categories(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> ApiResult<Json<TimeCategoryListResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, time_perm(core_domain::policy::Action::View))?;
    let org_id = authed.principal.user.org_id;
    let include_retired = q
        .get("include_retired")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let categories = core_db::time_categories::list_categories(&mut *tx, org_id, include_retired)
        .await?;
    let counts = core_db::time_categories::entry_counts_by_slug(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(TimeCategoryListResponse {
        categories: categories
            .into_iter()
            .map(|c| TimeCategoryView {
                entry_count: counts.get(&c.slug).copied().unwrap_or(0),
                category: c,
            })
            .collect(),
    }))
}

/// POST /api/time-categories — create a category.
#[utoipa::path(operation_id = "time_categories_create", post, path = "/api/time-categories", tag = "time", security(("bearer" = [])),
    request_body = TimeCategoryCreateRequest,
    responses((status = 200, body = TimeCategoryMutationResponse),
              (status = 409, description = "duplicate slug or idempotency key reused")))]
pub async fn create_category(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<TimeCategoryCreateRequest>,
) -> ApiResult<Json<TimeCategoryMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_billing())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let slug = validate_slug(&req.slug)?;
    let label = validate_label(&req.label)?;
    let description = validate_description(req.description.as_deref())?;
    let position = req.position.unwrap_or_default();
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "time_category",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let id = Uuid::now_v7();
        core_db::time_categories::insert_category(
            &mut *ctx.tx,
            &NewTimeCategory {
                id,
                org_id,
                slug: &slug,
                label: &label,
                description: description.as_deref(),
                default_billable: req.default_billable,
                requires_unbillable_reason: req.requires_unbillable_reason,
                rate_card_affinity: req.rate_card_affinity,
                position,
                created_by: Some(user_id),
            },
        )
        .await
        .map_err(|e| match e {
            core_db::DbError::Sql(sqlx::Error::Database(db)) if db.constraint().is_some() => {
                ApiError::Conflict("a category with this slug already exists".into())
            }
            other => other.into(),
        })?;
        let category = core_db::time_categories::get_category(&mut *ctx.tx, org_id, id).await?;
        audit_category(&mut ctx, org_id, user_id, "time_category.created", &category, serde_json::json!({
            "slug": category.slug,
            "label": category.label,
        }))
        .await?;
        ctx.emit(
            types::TIME_CATEGORY_CREATED,
            "time_category",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TimeCategoryCreated {
                category: category.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        finish_category_mutation(ctx, key.as_deref(), None, category).await
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/time-categories/{id} — content update (slug immutable).
#[utoipa::path(operation_id = "time_categories_update", patch, path = "/api/time-categories/{id}", tag = "time", security(("bearer" = [])),
    request_body = TimeCategoryUpdateRequest,
    responses((status = 200, body = TimeCategoryMutationResponse),
              (status = 404, description = "category not visible in scope (rls_sparse)")))]
pub async fn update_category(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<TimeCategoryUpdateRequest>,
) -> ApiResult<Json<TimeCategoryMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_billing())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let label = req.label.as_deref().map(validate_label).transpose()?;
    let description = match &req.description {
        Some(inner) => Some(validate_description(inner.as_deref())?),
        None => None,
    };
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "time_category",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::time_categories::get_category(&mut *ctx.tx, org_id, id).await?;
        let label = label.unwrap_or_else(|| before.label.clone());
        let description = description.unwrap_or_else(|| before.description.clone());
        let default_billable = req.default_billable.unwrap_or(before.default_billable);
        let requires_unbillable_reason = req
            .requires_unbillable_reason
            .unwrap_or(before.requires_unbillable_reason);
        let rate_card_affinity = req.rate_card_affinity.unwrap_or(before.rate_card_affinity);
        core_db::time_categories::update_category(
            &mut *ctx.tx,
            org_id,
            id,
            &label,
            description.as_deref(),
            default_billable,
            requires_unbillable_reason,
            rate_card_affinity,
        )
        .await?;
        if let Some(retired) = req.retired {
            core_db::time_categories::set_category_retired(&mut *ctx.tx, org_id, id, retired).await?;
        }
        let category = core_db::time_categories::get_category(&mut *ctx.tx, org_id, id).await?;
        audit_category(&mut ctx, org_id, user_id, "time_category.updated", &category, serde_json::json!({
            "slug": category.slug,
        }))
        .await?;
        ctx.emit(
            types::TIME_CATEGORY_UPDATED,
            "time_category",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TimeCategoryUpdated {
                category: category.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        finish_category_mutation(ctx, key.as_deref(), req.instant.version, category).await
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/time-categories/{id} — soft retire (pickers drop it,
/// entries keep the slug reference; nothing is hard-deleted).
#[utoipa::path(operation_id = "time_categories_delete", delete, path = "/api/time-categories/{id}", tag = "time", security(("bearer" = [])),
    responses((status = 200, body = TimeCategoryMutationResponse),
              (status = 404, description = "category not visible in scope (rls_sparse)")))]
pub async fn delete_category(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<TimeCategoryMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_billing())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let retired = true;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let before = core_db::time_categories::get_category(&mut *ctx.tx, org_id, id).await?;
        core_db::time_categories::set_category_retired(&mut *ctx.tx, org_id, id, retired).await?;
        let category = core_db::time_categories::get_category(&mut *ctx.tx, org_id, id).await?;
        audit_category(&mut ctx, org_id, user_id, "time_category.retired", &category, serde_json::json!({
            "slug": category.slug,
            "was_retired": before.retired,
        }))
        .await?;
        ctx.emit(
            types::TIME_CATEGORY_UPDATED,
            "time_category",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TimeCategoryUpdated {
                category: category.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        finish_category_mutation(ctx, None, None, category).await
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// POST /api/time-categories/reorder — rewrite display order from an
/// ordered id list (settings drag-order).
#[utoipa::path(operation_id = "time_categories_reorder", post, path = "/api/time-categories/reorder", tag = "time", security(("bearer" = [])),
    request_body = ReorderCategoriesRequest,
    responses((status = 200, body = TimeCategoryListResponse)))]
pub async fn reorder_categories(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<ReorderCategoriesRequest>,
) -> ApiResult<Json<TimeCategoryListResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_billing())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(_stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "time_category",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            // Reorder replays as a fresh read of the (already reordered)
            // list — the idempotent result IS the new order.
        }
        core_db::time_categories::reorder_categories(&mut ctx.tx, org_id, &req.ordered_ids)
            .await?;
        audit_category(
            &mut ctx,
            org_id,
            user_id,
            "time_category.reordered",
            &TimeCategory {
                id: Uuid::nil(),
                org_id,
                slug: String::new(),
                label: String::new(),
                description: None,
                default_billable: true,
                requires_unbillable_reason: false,
                rate_card_affinity: false,
                position: 0,
                retired: false,
                created_by: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            serde_json::json!({ "ordered_ids": req.ordered_ids }),
        )
        .await?;
        ctx.emit(
            types::TIME_CATEGORY_UPDATED,
            "time_category",
            Some("reorder"),
            &serde_json::json!({ "reordered": req.ordered_ids }),
        )
        .await?;
        let categories = core_db::time_categories::list_categories(&mut *ctx.tx, org_id, true)
            .await?;
        let counts = core_db::time_categories::entry_counts_by_slug(&mut *ctx.tx, org_id).await?;
        let views: Vec<TimeCategoryView> = categories
            .into_iter()
            .map(|c| TimeCategoryView {
                entry_count: counts.get(&c.slug).copied().unwrap_or(0),
                category: c,
            })
            .collect();
        Ok((ctx, views))
    })
    .await?;
    Ok(Json(TimeCategoryListResponse { categories: outcome }))
}

/// GET /api/time-categories/capture-defaults — the picker read for the
/// wrap-up sheet + quick-log: ordered non-retired categories + the
/// kept backfill mapping.
#[utoipa::path(operation_id = "time_categories_capture_defaults", get, path = "/api/time-categories/capture-defaults", tag = "time", security(("bearer" = [])),
    responses((status = 200, body = CaptureDefaultsResponse)))]
pub async fn capture_defaults(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<CaptureDefaultsResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, time_perm(core_domain::policy::Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let categories = core_db::time_categories::list_categories(&mut *tx, org_id, false).await?;
    let map = core_db::time_categories::list_work_type_map(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(CaptureDefaultsResponse {
        categories,
        work_type_map: map.into_iter().map(wire_map_row).collect(),
    }))
}

/// GET /api/time-categories/work-type-map — the kept backfill + manual
/// mapping (settings read).
#[utoipa::path(operation_id = "time_categories_work_type_map_list", get, path = "/api/time-categories/work-type-map", tag = "time", security(("bearer" = [])),
    responses((status = 200, body = [WorkTypeMapRowWire])))]
pub async fn list_work_type_map(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<WorkTypeMapRowWire>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_billing())?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::time_categories::list_work_type_map(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows.into_iter().map(wire_map_row).collect()))
}

/// POST /api/time-categories/work-type-map — add a manual alias.
#[utoipa::path(operation_id = "time_categories_work_type_map_upsert", post, path = "/api/time-categories/work-type-map", tag = "time", security(("bearer" = [])),
    request_body = UpsertWorkTypeMapRequest,
    responses((status = 200, body = [WorkTypeMapRowWire]),
              (status = 404, description = "category slug not found in this org")))]
pub async fn upsert_work_type_map(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<UpsertWorkTypeMapRequest>,
) -> ApiResult<Json<Vec<WorkTypeMapRowWire>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_billing())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let legacy_value = req.legacy_value.trim().to_string();
    if legacy_value.is_empty() || legacy_value.len() > 120 {
        return Err(ApiError::Validation(
            "legacy_value must be 1..120 characters".into(),
        ));
    }
    let slug = validate_slug(&req.category_slug)?;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let _ = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "time_category",
                entity_id: None,
                key: req.instant.idempotency_key.as_deref(),
                fingerprint: &fingerprint_of(&req),
            },
        )
        .await?;
        core_db::time_categories::get_category_by_slug(&mut *ctx.tx, org_id, &slug)
            .await?
            .ok_or(ApiError::NotFound)?;
        core_db::time_categories::upsert_work_type_map(&mut *ctx.tx, org_id, &legacy_value, &slug)
            .await?;
        audit_category(
            &mut ctx,
            org_id,
            user_id,
            "time_category.work_type_alias",
            &TimeCategory {
                id: Uuid::nil(),
                org_id,
                slug: slug.clone(),
                label: String::new(),
                description: None,
                default_billable: true,
                requires_unbillable_reason: false,
                rate_card_affinity: false,
                position: 0,
                retired: false,
                created_by: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            serde_json::json!({ "legacy_value": legacy_value, "category_slug": slug }),
        )
        .await?;
        let rows = core_db::time_categories::list_work_type_map(&mut *ctx.tx, org_id).await?;
        Ok((
            ctx,
            rows.into_iter().map(wire_map_row).collect::<Vec<_>>(),
        ))
    })
    .await?;
    Ok(Json(outcome))
}

/// POST /api/time-categories/work-type-map/apply — recategorize
/// existing entries (explicit alias application).
#[utoipa::path(operation_id = "time_categories_work_type_map_apply", post, path = "/api/time-categories/work-type-map/apply", tag = "time", security(("bearer" = [])),
    request_body = ApplyWorkTypeMapRequest,
    responses((status = 200, body = ApplyWorkTypeMapResponse),
              (status = 404, description = "category slug not found in this org")))]
pub async fn apply_work_type_map(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<ApplyWorkTypeMapRequest>,
) -> ApiResult<Json<ApplyWorkTypeMapResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_billing())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let legacy_value = req.legacy_value.trim().to_string();
    if legacy_value.is_empty() || legacy_value.len() > 120 {
        return Err(ApiError::Validation(
            "legacy_value must be 1..120 characters".into(),
        ));
    }
    let slug = validate_slug(&req.category_slug)?;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::time_categories::get_category_by_slug(&mut *ctx.tx, org_id, &slug)
            .await?
            .ok_or(ApiError::NotFound)?;
        let updated =
            core_db::time_categories::apply_work_type_map(&mut *ctx.tx, org_id, &legacy_value, &slug)
                .await?;
        audit_category(
            &mut ctx,
            org_id,
            user_id,
            "time_category.work_type_applied",
            &TimeCategory {
                id: Uuid::nil(),
                org_id,
                slug: slug.clone(),
                label: String::new(),
                description: None,
                default_billable: true,
                requires_unbillable_reason: false,
                rate_card_affinity: false,
                position: 0,
                retired: false,
                created_by: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            serde_json::json!({
                "legacy_value": legacy_value,
                "category_slug": slug,
                "entries_updated": updated,
            }),
        )
        .await?;
        let mutation = instant::MutationMeta {
            seq: 0,
            version: 0,
            updated_at: chrono::Utc::now(),
            before: None,
            after: serde_json::Value::Null,
            idempotency_key: req.instant.idempotency_key.clone(),
        };
        Ok((
            ctx,
            ApplyWorkTypeMapResponse {
                entries_updated: updated as i64,
                mutation,
            },
        ))
    })
    .await?;
    Ok(Json(outcome))
}

/// GET /api/time-entries/{id}/billing-suggestions — the
/// `decision_billing_audit` shadow answers for one entry, classified
/// into suggestion chips. Read-only: chips never mutate anything.
#[utoipa::path(operation_id = "time_entry_billing_suggestions", get, path = "/api/time-entries/{id}/billing-suggestions", tag = "time", security(("bearer" = [])),
    responses((status = 200, body = EntrySuggestionsResponse),
              (status = 404, description = "entry not visible in scope (rls_sparse)")))]
pub async fn entry_suggestions(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<EntrySuggestionsResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, time_perm(core_domain::policy::Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    // RLS proves visibility: an invisible entry reads as no answers.
    let _ = core_db::timers::get_time_entry(&mut *tx, org_id, id).await?;
    let row =
        core_db::decision::latest_answers_for_entity(&mut *tx, org_id, "decision_billing_audit", "time_entry", id)
            .await?;
    let inputs = core_db::approvals::verdict_inputs(&mut *tx, org_id, &[id]).await?;
    let verdicts = inputs.first().map(build_verdicts).unwrap_or_default();
    tx.commit().await.map_err(ApiError::from)?;
    let mut suggestions = build_chips(row.as_ref());
    if let Some(first) = suggestions.first_mut() {
        first.verdicts = verdicts;
    } else if !verdicts.is_empty() {
        // A mechanical verdict stands on its own: an org with no audit
        // surface configured still gets the overlap and the description
        // check, because those are arithmetic, not opinions.
        suggestions.push(EntrySuggestion {
            chip: "verdict".to_string(),
            label: "Needs a look".to_string(),
            billable_probability: None,
            state: Some("pending".to_string()),
            mode: "local".to_string(),
            recorded_at: chrono::Utc::now(),
            verdicts,
        });
    }
    Ok(Json(EntrySuggestionsResponse {
        entry_id: id,
        suggestions,
    }))
}

/// GET /api/time-entries/billing-suggestions?ids=a,b,c — the bulk read
/// for approvals rows (≤ 100 ids per call).
#[utoipa::path(operation_id = "time_entries_billing_suggestions_bulk", get, path = "/api/time-entries/billing-suggestions", tag = "time", security(("bearer" = [])),
    params(BulkSuggestionsQuery),
    responses((status = 200, body = BulkEntrySuggestionsResponse)))]
pub async fn bulk_entry_suggestions(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<BulkSuggestionsQuery>,
) -> ApiResult<Json<BulkEntrySuggestionsResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, time_perm(core_domain::policy::Action::View))?;
    let org_id = authed.principal.user.org_id;
    let ids: Vec<Uuid> = q
        .ids
        .split(',')
        .filter_map(|s| Uuid::parse_str(s.trim()).ok())
        .take(100)
        .collect();
    if ids.is_empty() {
        return Ok(Json(BulkEntrySuggestionsResponse { items: vec![] }));
    }
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    // One read for every entry's verdicts, so a hundred queue rows cost one
    // extra round trip rather than a hundred.
    let inputs = core_db::approvals::verdict_inputs(&mut *tx, org_id, &ids).await?;
    let mut items = Vec::with_capacity(ids.len());
    for id in ids {
        let row = core_db::decision::latest_answers_for_entity(
            &mut *tx,
            org_id,
            "decision_billing_audit",
            "time_entry",
            id,
        )
        .await?;
        let verdicts = inputs
            .iter()
            .find(|input| input.entry.id == id)
            .map(build_verdicts)
            .unwrap_or_default();
        let mut suggestions = build_chips(row.as_ref());
        match suggestions.first_mut() {
            Some(first) => first.verdicts = verdicts,
            None if !verdicts.is_empty() => suggestions.push(EntrySuggestion {
                chip: "verdict".to_string(),
                label: "Needs a look".to_string(),
                billable_probability: None,
                state: Some("pending".to_string()),
                mode: "local".to_string(),
                recorded_at: chrono::Utc::now(),
                verdicts,
            }),
            None => {}
        }
        items.push(EntrySuggestionsResponse {
            entry_id: id,
            suggestions,
        });
    }
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(BulkEntrySuggestionsResponse { items }))
}

// ---- helpers ----

/// The server's verdicts for one entry, from the facts the read already has.
///
/// The overlap wins when an entry has more than one thing wrong: it is the one
/// that is certainly wrong, and trimming it is the one fix that needs no
/// judgement from the person pressing it.
pub fn build_verdicts(input: &core_db::approvals::VerdictEntry) -> Vec<TimeVerdict> {
    use core_domain::entities::{VerdictInput, verdicts_for};
    let overlap = input
        .overlaps
        .iter()
        .max_by_key(|(_, _, minutes)| *minutes);
    let (overlap_number, overlap_minutes) = match overlap {
        Some((_, number, minutes)) => (
            number.as_deref().map(|n| format!("#{n}")),
            *minutes,
        ),
        None => (None, 0),
    };
    let mut verdicts = verdicts_for(&VerdictInput {
        notes: input.entry.notes.as_deref(),
        minutes: input.entry.minutes,
        ticket_number: input.ticket_number.as_deref().map(|n| format!("#{n}")),
        work_type_label: input.entry.work_type.as_deref(),
        peer_minutes: &input.peer_minutes,
        overlap_with_ticket_number: overlap_number.as_deref(),
        overlap_minutes,
    });
    if verdicts.len() > 1 {
        // One mark per row is the pack's rule: the strongest, and the fix
        // beside it. The rest are for the detail pane.
        verdicts.truncate(1);
    }
    verdicts
}

fn build_chips(
    row: Option<&core_db::decision::DecisionAnswersRow>,
) -> Vec<EntrySuggestion> {
    let Some(row) = row else { return vec![] };
    let Some(answers) = row.answers.as_object() else {
        return vec![];
    };
    let mut chips = Vec::new();
    if let Some(billable) = answers.get("billable") {
        let probability = billable
            .get("probability")
            .and_then(|v| v.as_f64());
        // Threshold gate per the billing-audit starter (act 0.8, review
        // 0.6): ≥ 0.8 confident billable, 0.6..0.8 looks-billable
        // pending, < 0.6 looks non-billable.
        let (chip, label, state) = match probability {
            Some(p) if p >= 0.8 => (
                "billable_true",
                "Looks billable",
                Some("act"),
            ),
            Some(p) if p >= 0.6 => (
                "billable_true",
                "Probably billable",
                Some("pending"),
            ),
            Some(_) => (
                "billable_false",
                "Looks non-billable — add a reason?",
                Some("ignore"),
            ),
            None => ("billable_false", "Audit inconclusive", None),
        };
        chips.push(EntrySuggestion {
            chip: chip.to_string(),
            label: label.to_string(),
            billable_probability: probability,
            state: state.map(str::to_string),
            mode: row.mode.clone(),
            recorded_at: row.created_at,
            verdicts: Vec::new(),
        });
    }
    chips
}

fn wire_map_row(r: WorkTypeMapRow) -> WorkTypeMapRowWire {
    WorkTypeMapRowWire {
        legacy_value: r.legacy_value,
        category_slug: r.category_slug,
        source: r.source,
        created_at: r.created_at,
    }
}

fn fingerprint_of<T: serde::Serialize>(req: &T) -> serde_json::Value {
    serde_json::to_value(req).unwrap_or(serde_json::Value::Null)
}

async fn audit_category(
    ctx: &mut org_tx::OrgTx<'static>,
    org_id: Uuid,
    user_id: Uuid,
    action: &str,
    category: &TimeCategory,
    detail: serde_json::Value,
) -> ApiResult<()> {
    core_db::audit::insert(
        &mut *ctx.tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::Human(user_id),
            action: action.into(),
            entity_type: Some("time_category".into()),
            entity_id: Some(category.id.to_string()),
            detail,
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    Ok(())
}

async fn finish_category_mutation(
    mut ctx: org_tx::OrgTx<'static>,
    key: Option<&str>,
    version: Option<i64>,
    category: TimeCategory,
) -> ApiResult<(
    org_tx::OrgTx<'static>,
    MutationOutcome<TimeCategoryMutationResponse>,
)> {
    let count =
        core_db::time_categories::entry_counts_by_slug(&mut *ctx.tx, ctx.org_id)
            .await
            .unwrap_or_default();
    let view = TimeCategoryView {
        entry_count: count.get(&category.slug).copied().unwrap_or(0),
        category: category.clone(),
    };
    let write_seq = instant::entity_seq(
        &mut ctx,
        instant::SeqSpec {
            entity: "time_category",
            entity_id: category.id,
            // IF1 optimistic concurrency: a stale editor gets a typed 409
            // instead of silently overwriting a newer taxonomy row.
            version,
        },
    )
    .await?;
    let stored = TimeCategoryMutationResponse {
        category: view.clone(),
        mutation: instant::MutationMeta {
            seq: write_seq,
            version: write_seq,
            updated_at: chrono::Utc::now(),
            before: None,
            after: serde_json::Value::Null,
            idempotency_key: None,
        },
    };
    let mutation = instant::record(
        &mut ctx,
        instant::RecordSpec {
            entity: "time_category",
            entity_id: category.id,
            op: "mutation",
            seq: write_seq,
            before: None,
            after: serde_json::to_value(&category).unwrap_or(serde_json::Value::Null),
            updated_at: chrono::Utc::now(),
            key,
            payload: &stored,
            provenance: Default::default(),
        },
    )
    .await?;
    Ok((
        ctx,
        MutationOutcome::Applied(TimeCategoryMutationResponse {
            category: view,
            mutation,
        }),
    ))
}
