//! Ticket type/category/subcategory taxonomy (Wave 11 lane 3) + the
//! ticket board-move endpoint.
//!
//! The taxonomy trees are CLASSIFICATION-ONLY (§5): full org CRUD here —
//! create, rename/re-sort, reparent (subcategories between roots), and
//! soft retire — is behavior-neutral by construction. Nothing reads these
//! trees to drive SLA, workflow, automations, billing, or reports; those
//! bind to the SEMANTIC status categories. Retirement never deletes: the
//! row is flagged, existing tickets keep their FK reference, and the U5
//! manifest still exports retired nodes.
//!
//! Board moves remap the ticket's status through the semantic categories:
//! the target status is the first target-board status sharing the current
//! category (deterministic order), or the caller pins one explicitly. When
//! the category has no counterpart, the mutation is rejected with a typed
//! 409 (`board_move_status_conflict`) listing every target-board status.
//! SLA state is PRESERVED by default; `reset_sla: true` re-initializes the
//! clocks under the org's default policy anchored at the move moment.
//!
//! Mutations ride the IF1–IF3 instant-first envelope and write their
//! domain event + audit row in the same commit.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::routes::board_access::authorize_board_op;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use core_db::taxonomy::{TaxonomyNode, TaxonomyTable};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::events::types;
use core_domain::policy::{BoardOp, Module, Permission};
use core_domain::status::TicketStatusCategory;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

fn manage_boards() -> Permission {
    Permission::ManageBoards
}

fn tickets(action: core_domain::policy::Action) -> Permission {
    Permission::Matrix {
        module: Module::Tickets,
        action,
    }
}

const NAME_MAX: usize = 120;

fn validate_name(name: &str) -> ApiResult<String> {
    let trimmed = name.trim().to_string();
    if trimmed.is_empty() {
        return Err(ApiError::Validation("name is required".into()));
    }
    if trimmed.len() > NAME_MAX {
        return Err(ApiError::Validation(format!(
            "name must be at most {NAME_MAX} characters"
        )));
    }
    Ok(trimmed)
}

/// One taxonomy node (type or category). `parent_id` is null for types and
/// root categories; subcategories carry their parent.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TaxonomyNodeView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub org_id: Uuid,
    #[schema(value_type = Option<uuid::Uuid>, nullable = true)]
    pub parent_id: Option<Uuid>,
    pub name: String,
    #[schema(nullable = true)]
    pub description: Option<String>,
    pub is_retired: bool,
    pub sort_order: i32,
    #[schema(value_type = String)]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[schema(value_type = String)]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<TaxonomyNode> for TaxonomyNodeView {
    fn from(n: TaxonomyNode) -> Self {
        Self {
            id: n.id,
            org_id: n.org_id,
            parent_id: n.parent_id,
            name: n.name,
            description: n.description,
            is_retired: n.is_retired,
            sort_order: n.sort_order,
            created_at: n.created_at,
            updated_at: n.updated_at,
        }
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct ListTaxonomyQuery {
    /// Include retired nodes (admin editors + the manifest want them;
    /// pickers default to false).
    #[serde(default)]
    pub include_retired: bool,
}

/// One target-board status option inside a `board_move_status_conflict`
/// 409 — everything the client needs to render the pick list.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TargetStatusOption {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    pub name: String,
    pub category: TicketStatusCategory,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateTaxonomyNodeRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub sort_order: i32,
    /// Subcategory parent (categories only): must be a ROOT category of
    /// this org — the tree is exactly two levels deep.
    #[serde(default)]
    pub parent_id: Option<Uuid>,
    /// Instant-first autosave parameters (IF1): idempotency key. Fresh
    /// entities have no version to match.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UpdateTaxonomyNodeRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub sort_order: i32,
    /// Instant-first autosave parameters (IF1): idempotency key + version.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReparentCategoryRequest {
    /// New root category (must be a root of this org).
    pub parent_id: Uuid,
    /// Instant-first autosave parameters (IF1): idempotency key + version.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RetireTaxonomyNodeRequest {
    /// Instant-first autosave parameters (IF1): idempotency key + version.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// Instant-first mutation response (IF2): the node plus read-your-write
/// metadata.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TaxonomyMutationResponse {
    #[serde(flatten)]
    pub node: TaxonomyNodeView,
    /// How many tickets still reference the node (informational —
    /// retirement does not require zero references).
    pub ticket_references: i64,
    pub mutation: instant::MutationMeta,
}

fn parse_table(kind: &str) -> TaxonomyTable {
    match kind {
        "types" => TaxonomyTable::Type,
        _ => TaxonomyTable::Category,
    }
}

async fn count_refs(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    table: TaxonomyTable,
    id: Uuid,
) -> ApiResult<i64> {
    Ok(core_db::taxonomy::count_ticket_references(conn, org_id, table, id).await?)
}

// --- reads ---------------------------------------------------------------

/// GET /api/taxonomy/types — the org's ticket types; active-only unless
/// `include_retired`.
#[utoipa::path(operation_id = "taxonomy_list_types", get, path = "/api/taxonomy/types", tag = "taxonomy", security(("bearer" = [])),
    params(ListTaxonomyQuery),
    responses((status = 200, body = [TaxonomyNodeView])))]
pub async fn list_types(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ListTaxonomyQuery>,
) -> ApiResult<Json<Vec<TaxonomyNodeView>>> {
    list_nodes(state, extensions, TaxonomyTable::Type, q.include_retired).await
}

/// GET /api/taxonomy/categories — the two-level category tree, flat and
/// ordered (roots and subcategories interleaved by sort order; `parent_id`
/// reconstructs the tree).
#[utoipa::path(operation_id = "taxonomy_list_categories", get, path = "/api/taxonomy/categories", tag = "taxonomy", security(("bearer" = [])),
    params(ListTaxonomyQuery),
    responses((status = 200, body = [TaxonomyNodeView])))]
pub async fn list_categories(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ListTaxonomyQuery>,
) -> ApiResult<Json<Vec<TaxonomyNodeView>>> {
    list_nodes(
        state,
        extensions,
        TaxonomyTable::Category,
        q.include_retired,
    )
    .await
}

async fn list_nodes(
    state: AppState,
    extensions: Extensions,
    table: TaxonomyTable,
    include_retired: bool,
) -> ApiResult<Json<Vec<TaxonomyNodeView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let nodes = core_db::taxonomy::list_nodes(&mut tx, org_id, table, include_retired).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(nodes.into_iter().map(Into::into).collect()))
}

// --- mutations -----------------------------------------------------------

/// POST /api/taxonomy/types — create a ticket type.
#[utoipa::path(operation_id = "taxonomy_create_type", post, path = "/api/taxonomy/types", tag = "taxonomy", security(("bearer" = [])),
    request_body = CreateTaxonomyNodeRequest,
    responses((status = 200, body = TaxonomyMutationResponse),
              (status = 409, description = "duplicate name or idempotency key reused")))]
pub async fn create_type(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateTaxonomyNodeRequest>,
) -> ApiResult<Json<TaxonomyMutationResponse>> {
    create_node(state, extensions, "types", req).await
}

/// POST /api/taxonomy/categories — create a root category, or a
/// subcategory when `parent_id` names a root of this org.
#[utoipa::path(operation_id = "taxonomy_create_category", post, path = "/api/taxonomy/categories", tag = "taxonomy", security(("bearer" = [])),
    request_body = CreateTaxonomyNodeRequest,
    responses((status = 200, body = TaxonomyMutationResponse),
              (status = 404, description = "parent category not found"),
              (status = 409, description = "duplicate sibling name or idempotency key reused")))]
pub async fn create_category(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateTaxonomyNodeRequest>,
) -> ApiResult<Json<TaxonomyMutationResponse>> {
    create_node(state, extensions, "categories", req).await
}

async fn create_node(
    state: AppState,
    extensions: Extensions,
    kind: &str,
    req: CreateTaxonomyNodeRequest,
) -> ApiResult<Json<TaxonomyMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_boards())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let name = validate_name(&req.name)?;
    let table = parse_table(kind);
    if table == TaxonomyTable::Type && req.parent_id.is_some() {
        return Err(ApiError::Validation(
            "types are flat: parent_id is only valid for categories".into(),
        ));
    }
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "taxonomy",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        if let Some(parent_id) = req.parent_id {
            let parent = core_db::taxonomy::get_node(&mut ctx.tx, org_id, table, parent_id).await?;
            if parent.parent_id.is_some() {
                return Err(ApiError::Validation(
                    "subcategories cannot have children: the tree is exactly two levels deep"
                        .into(),
                ));
            }
        }
        let id = Uuid::now_v7();
        let node = core_db::taxonomy::insert_node(
            &mut ctx.tx,
            org_id,
            id,
            table,
            req.parent_id,
            &name,
            req.description.as_deref(),
            req.sort_order,
        )
        .await
        .map_err(|e| match e {
            // Sibling/flat name uniqueness (0048 partial unique indexes).
            core_db::DbError::Sql(sqlx::Error::Database(db)) if db.constraint().is_some() => {
                ApiError::Conflict("a node with this name already exists here".into())
            }
            other => other.into(),
        })?;

        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: format!("taxonomy.{}_created", table.as_str()),
                entity_type: Some(table.as_str().into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "name": node.name,
                    "parent_id": node.parent_id,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "taxonomy",
                entity_id: id,
                version: None,
            },
        )
        .await?;
        let view: TaxonomyNodeView = node.into();
        let refs = count_refs(&mut ctx.tx, org_id, table, id).await?;
        // The stored replay payload is the FULL response: a replayed key
        // must return exactly what the first call returned.
        let stored = TaxonomyMutationResponse {
            node: view.clone(),
            ticket_references: refs,
            mutation: instant::MutationMeta {
                seq: write_seq,
                version: write_seq,
                updated_at: chrono::Utc::now(),
                before: None,
                after: Value::Null,
                idempotency_key: None,
            },
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "taxonomy",
                entity_id: id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &stored,
                provenance: Default::default(),
            },
        )
        .await?;
        let response = TaxonomyMutationResponse {
            node: view,
            ticket_references: stored.ticket_references,
            mutation,
        };
        Ok((ctx, MutationOutcome::Applied(response)))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/taxonomy/types/{id} — rename / edit description / re-sort.
/// Behavior-neutral: nothing keys off these trees.
#[utoipa::path(operation_id = "taxonomy_update_type", patch, path = "/api/taxonomy/types/{id}", tag = "taxonomy", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = UpdateTaxonomyNodeRequest,
    responses((status = 200, body = TaxonomyMutationResponse),
              (status = 404, description = "node not visible in scope (rls_sparse)"),
              (status = 409, description = "duplicate name, version conflict, or idempotency key reused")))]
pub async fn update_type(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateTaxonomyNodeRequest>,
) -> ApiResult<Json<TaxonomyMutationResponse>> {
    update_node(state, extensions, "types", id, req).await
}

/// PATCH /api/taxonomy/categories/{id} — rename / edit description /
/// re-sort (root or subcategory).
#[utoipa::path(operation_id = "taxonomy_update_category", patch, path = "/api/taxonomy/categories/{id}", tag = "taxonomy", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = UpdateTaxonomyNodeRequest,
    responses((status = 200, body = TaxonomyMutationResponse),
              (status = 404, description = "node not visible in scope (rls_sparse)"),
              (status = 409, description = "duplicate sibling name, version conflict, or idempotency key reused")))]
pub async fn update_category(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateTaxonomyNodeRequest>,
) -> ApiResult<Json<TaxonomyMutationResponse>> {
    update_node(state, extensions, "categories", id, req).await
}

async fn update_node(
    state: AppState,
    extensions: Extensions,
    kind: &str,
    id: Uuid,
    req: UpdateTaxonomyNodeRequest,
) -> ApiResult<Json<TaxonomyMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_boards())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let name = validate_name(&req.name)?;
    let table = parse_table(kind);
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "taxonomy",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::taxonomy::get_node(&mut ctx.tx, org_id, table, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        core_db::taxonomy::update_node(
            &mut ctx.tx,
            org_id,
            table,
            id,
            &name,
            req.description.as_deref(),
            req.sort_order,
        )
        .await
        .map_err(|e| match e {
            core_db::DbError::Sql(sqlx::Error::Database(db)) if db.constraint().is_some() => {
                ApiError::Conflict("a node with this name already exists here".into())
            }
            other => other.into(),
        })?;
        let after = core_db::taxonomy::get_node(&mut ctx.tx, org_id, table, id).await?;

        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: format!("taxonomy.{}_updated", table.as_str()),
                entity_type: Some(table.as_str().into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "from_name": before.name,
                    "to_name": after.name,
                    "sort_order": after.sort_order,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "taxonomy",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let view: TaxonomyNodeView = after.into();
        let refs = count_refs(&mut ctx.tx, org_id, table, id).await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "taxonomy",
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: serde_json::to_value(TaxonomyNodeView::from(before)).ok(),
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TaxonomyMutationResponse {
                node: view,
                ticket_references: refs,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/taxonomy/categories/{id}/reparent — move a SUBCATEGORY under
/// a different root. Roots cannot be reparented (that would grow a third
/// level or orphan children).
#[utoipa::path(operation_id = "taxonomy_reparent_category", post, path = "/api/taxonomy/categories/{id}/reparent", tag = "taxonomy", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = ReparentCategoryRequest,
    responses((status = 200, body = TaxonomyMutationResponse),
              (status = 404, description = "node or new parent not visible in scope (rls_sparse)"),
              (status = 409, description = "version conflict or idempotency key reused")))]
pub async fn reparent_category(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<ReparentCategoryRequest>,
) -> ApiResult<Json<TaxonomyMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_boards())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let table = TaxonomyTable::Category;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "taxonomy",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::taxonomy::get_node(&mut ctx.tx, org_id, table, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if before.parent_id.is_none() {
            return Err(ApiError::Validation(
                "only subcategories can be reparented; roots are the top level".into(),
            ));
        }
        let new_parent =
            core_db::taxonomy::get_node(&mut ctx.tx, org_id, table, req.parent_id).await?;
        if new_parent.parent_id.is_some() {
            return Err(ApiError::Validation(
                "the new parent must be a root category (two-level tree)".into(),
            ));
        }
        let after = core_db::taxonomy::reparent_category(&mut ctx.tx, org_id, id, req.parent_id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;

        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "taxonomy.ticket_categories_reparented".into(),
                entity_type: Some("ticket_categories".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "name": after.name,
                    "from_parent_id": before.parent_id,
                    "to_parent_id": after.parent_id,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "taxonomy",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let view: TaxonomyNodeView = after.into();
        let refs = count_refs(&mut ctx.tx, org_id, table, id).await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "taxonomy",
                entity_id: id,
                op: "reparent",
                seq: write_seq,
                before: serde_json::to_value(TaxonomyNodeView::from(before)).ok(),
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TaxonomyMutationResponse {
                node: view,
                ticket_references: refs,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/taxonomy/types/{id}/retire — soft-retire: pickers hide the
/// node, existing tickets keep their reference. Idempotent: retiring an
/// already-retired node returns its current state.
#[utoipa::path(operation_id = "taxonomy_retire_type", post, path = "/api/taxonomy/types/{id}/retire", tag = "taxonomy", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = RetireTaxonomyNodeRequest,
    responses((status = 200, body = TaxonomyMutationResponse),
              (status = 404, description = "node not visible in scope (rls_sparse)"),
              (status = 409, description = "version conflict or idempotency key reused")))]
pub async fn retire_type(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<RetireTaxonomyNodeRequest>,
) -> ApiResult<Json<TaxonomyMutationResponse>> {
    retire_node(state, extensions, "types", id, req).await
}

/// POST /api/taxonomy/categories/{id}/retire — soft-retire a root or
/// subcategory. Idempotent like the type surface.
#[utoipa::path(operation_id = "taxonomy_retire_category", post, path = "/api/taxonomy/categories/{id}/retire", tag = "taxonomy", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = RetireTaxonomyNodeRequest,
    responses((status = 200, body = TaxonomyMutationResponse),
              (status = 404, description = "node not visible in scope (rls_sparse)"),
              (status = 409, description = "version conflict or idempotency key reused")))]
pub async fn retire_category(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<RetireTaxonomyNodeRequest>,
) -> ApiResult<Json<TaxonomyMutationResponse>> {
    retire_node(state, extensions, "categories", id, req).await
}

async fn retire_node(
    state: AppState,
    extensions: Extensions,
    kind: &str,
    id: Uuid,
    req: RetireTaxonomyNodeRequest,
) -> ApiResult<Json<TaxonomyMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_boards())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let table = parse_table(kind);
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "taxonomy",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::taxonomy::get_node(&mut ctx.tx, org_id, table, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        // Idempotent: a second retire returns the current state instead of
        // a 404 (the row is already where the caller wants it).
        if !before.is_retired {
            core_db::taxonomy::retire_node(&mut ctx.tx, org_id, table, id).await?;
        }
        let after = core_db::taxonomy::get_node(&mut ctx.tx, org_id, table, id).await?;

        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: format!("taxonomy.{}_retired", table.as_str()),
                entity_type: Some(table.as_str().into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "name": after.name }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "taxonomy",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let view: TaxonomyNodeView = after.into();
        let refs = count_refs(&mut ctx.tx, org_id, table, id).await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "taxonomy",
                entity_id: id,
                op: "retire",
                seq: write_seq,
                before: serde_json::to_value(TaxonomyNodeView::from(before)).ok(),
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TaxonomyMutationResponse {
                node: view,
                ticket_references: refs,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- board moves ---------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MoveBoardRequest {
    #[schema(value_type = uuid::Uuid)]
    pub target_board_id: Uuid,
    /// Pin the target status explicitly (the retry path after a
    /// `board_move_status_conflict`). Must be a status of the target
    /// board; omitted = remap through the semantic categories.
    #[serde(default)]
    pub status_id: Option<Uuid>,
    /// Default false: the existing SLA clock state is PRESERVED verbatim
    /// (board membership is classification, not a lifecycle event). true =
    /// re-initialize the clocks under the org's default policy anchored at
    /// the move moment.
    #[serde(default)]
    pub reset_sla: bool,
    /// Instant-first autosave parameters (IF1): idempotency key + version.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// Same additive envelope as the status endpoint (plain ticket body).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TicketBoardMoveResponse {
    #[serde(flatten)]
    pub ticket: core_domain::entities::Ticket,
    pub mutation: instant::MutationMeta,
}

/// POST /api/tickets/{id}/move-board — move a ticket to another board with
/// the status remapped through the SEMANTIC categories (typed 409 listing
/// the target statuses when the current category has no counterpart). SLA
/// state is preserved by default; `reset_sla` re-initializes the clocks.
/// Audited, emits `ticket.board_moved`, all in one transaction. Instant-
/// first (IF1/IF2): idempotency-keyed, version-gated, read-your-write.
#[utoipa::path(operation_id = "tickets_move_board", post, path = "/api/tickets/{id}/move-board", tag = "taxonomy", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = MoveBoardRequest,
    responses((status = 200, body = TicketBoardMoveResponse),
              (status = 404, description = "ticket or target board not visible in scope (rls_sparse)"),
              (status = 409, description = "no matching semantic status on the target board, version conflict, or idempotency key reused"),
              (status = 422, description = "validation error")))]
pub async fn move_board(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<MoveBoardRequest>,
) -> ApiResult<Json<TicketBoardMoveResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let version = req.instant.version;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "ticket",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if req.target_board_id == before.board_id {
            return Err(ApiError::Validation(
                "the ticket is already on that board".into(),
            ));
        }
        // Load the target board first: a missing/foreign board is the
        // typed not-visible error before anything else runs.
        core_db::boards::get_board(&mut *ctx.tx, org_id, req.target_board_id).await?;

        // Board-level RBAC on BOTH ends: working the ticket on the source
        // board and opening tickets on the target board are distinct
        // grants (deny wins on either).
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            before.board_id,
            BoardOp::WorkTicket,
        )
        .await?;
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            req.target_board_id,
            BoardOp::CreateTicket,
        )
        .await?;

        // Target status: explicit pin first (must belong to the target
        // board), otherwise remap through the semantic category — the
        // FIRST target status sharing the current category in the board's
        // deterministic order wins. No counterpart is a typed 409 listing
        // every target-board status.
        let target_statuses =
            core_db::boards::list_statuses(&mut *ctx.tx, req.target_board_id).await?;
        let target = if let Some(pinned) = req.status_id {
            target_statuses
                .iter()
                .find(|s| s.id == pinned)
                .ok_or_else(|| {
                    ApiError::Validation("status_id does not belong to the target board".into())
                })?
                .clone()
        } else {
            let mut candidates: Vec<_> = target_statuses
                .iter()
                .filter(|s| s.category == before.category)
                .collect();
            candidates
                .sort_by(|a, b| (a.sort_order, &a.name, a.id).cmp(&(b.sort_order, &b.name, b.id)));
            match candidates.first() {
                Some(s) => (*s).clone(),
                None => {
                    let options = target_statuses
                        .iter()
                        .map(|s| TargetStatusOption {
                            id: s.id,
                            name: s.name.clone(),
                            category: s.category,
                        })
                        .collect();
                    return Err(ApiError::BoardMoveStatusConflict {
                        target_board_id: req.target_board_id,
                        from_status_id: before.status_id,
                        from_category: before.category.as_str().to_string(),
                        target_statuses: options,
                    });
                }
            }
        };

        // Crossing into a closed-category status is the board-scoped close
        // operation, same as the status endpoint.
        if target.category == TicketStatusCategory::Closed {
            authorize_board_op(
                &mut ctx.tx,
                &authed.principal,
                req.target_board_id,
                BoardOp::CloseTicket,
            )
            .await?;
        }

        let now = chrono::Utc::now();
        let source_statuses = core_db::boards::list_statuses(&mut *ctx.tx, before.board_id).await?;
        let from_status = source_statuses
            .iter()
            .find(|s| s.id == before.status_id)
            .map(|s| s.name.clone())
            .unwrap_or_default();

        core_db::tickets::move_to_board(&mut *ctx.tx, org_id, id, req.target_board_id, target.id)
            .await?;

        // SLA (explicit re-init rules): the default PRESERVES the existing
        // ticket_sla_state row untouched — a board move is classification,
        // not a lifecycle event, and the pause/breach bookkeeping keyed on
        // the semantic category carries over (the target status shares the
        // category by construction). reset_sla=true re-initializes from
        // the org's default policy anchored NOW, exactly like creation —
        // an explicit operator decision, never a side effect.
        if req.reset_sla {
            if let Ok(policy) = core_db::sla::get_default_policy(&mut *ctx.tx, org_id).await {
                let fresh = core_domain::entities::TicketSlaState::init(
                    org_id,
                    id,
                    &policy,
                    before.priority,
                    now,
                );
                core_db::sla::upsert_ticket_state(&mut *ctx.tx, &fresh).await?;
            }
        }

        // The ticket picks up the target board's checklist template
        // (lazy-instantiate, same as the close path reads).
        let instantiated =
            core_db::checklists::instantiate_missing(&mut ctx.tx, org_id, req.target_board_id, id)
                .await?;
        if instantiated > 0 {
            let checklist = core_db::tickets::list_checklist(&mut *ctx.tx, org_id, id).await?;
            ctx.emit(
                types::TICKET_CHECKLIST_UPDATED,
                "ticket",
                Some(id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::TicketChecklistUpdated {
                    ticket_id: id,
                    board_id: req.target_board_id,
                    items: checklist,
                    updated_by: Some(user_id),
                })
                .unwrap_or_default(),
            )
            .await?;
        }

        let after = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await?;

        ctx.emit(
            types::TICKET_BOARD_MOVED,
            "ticket",
            Some(after.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketBoardMoved {
                ticket_id: after.id,
                number: after.number.0.clone(),
                from_board_id: before.board_id,
                to_board_id: after.board_id,
                from_status,
                to_status: target.name.clone(),
                from_category: before.category,
                to_category: target.category,
                sla_reset: req.reset_sla,
                moved_by: Some(user_id),
                occurred_at: now,
            })
            .unwrap_or_default(),
        )
        .await?;

        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "ticket.board_move".into(),
                entity_type: Some("ticket".into()),
                entity_id: Some(after.id.to_string()),
                detail: serde_json::json!({
                    "from_board_id": before.board_id,
                    "to_board_id": after.board_id,
                    "from_status_id": before.status_id,
                    "to_status_id": target.id,
                    "from_category": before.category.as_str(),
                    "to_category": target.category.as_str(),
                    "reset_sla": req.reset_sla,
                }),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "ticket",
                entity_id: id,
                version,
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "ticket",
                entity_id: id,
                op: "move_board",
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
            MutationOutcome::Applied(TicketBoardMoveResponse {
                ticket: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}
