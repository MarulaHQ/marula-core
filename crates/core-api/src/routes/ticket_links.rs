//! Ticket links + merge (Wave 11 lane 9, migration 0056).
//!
//! Purpose: typed ticket-to-ticket edges — `parent_child` (child →
//! parent) and `major_incident` (member → incident) — the read-side
//! ROLLUP over children/members + money, and the MERGE fold: children +
//! comments + time + costs + expenses fold into a survivor in one
//! audited transaction, with typed conflicts refused BEFORE any write
//! and field conflicts resolved explicitly in the request.
//!
//! Conflict model: structural conflicts (self-merge, already-merged,
//! closed survivor) are hard typed 409s; `board_mismatch` is refused
//! unless `resolve.board == "source"` (the survivor then moves onto the
//! source's board); field-value conflicts (title/priority/client/
//! assignee differences) resolve per field via `resolve.<field>` —
//! `"survivor"` (the default when unresolved) or `"source"`. The merge
//! response lists every conflict and what was applied.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::routes::board_access::authorize_board_op;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    routing::{delete, get, post},
    Json, Router,
};
use core_domain::entities::{Actor, AuditEvent, TicketLink, TicketLinkType};
use core_domain::events::types;
use core_domain::policy::{BoardOp, Module, Permission};
use core_domain::status::TicketStatusCategory;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

pub fn routes(gen: crate::routes::ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(
            &format!("{base}/tickets/{{id}}/links"),
            get(list_links).post(create_link),
        )
        .route(
            &format!("{base}/tickets/{{id}}/links/{{link_id}}"),
            delete(remove_link),
        )
        .route(&format!("{base}/tickets/{{id}}/rollup"), get(get_rollup))
        .route(&format!("{base}/tickets/{{id}}/merge"), post(merge))
}

fn tickets(action: core_domain::policy::Action) -> Permission {
    Permission::Matrix {
        module: Module::Tickets,
        action,
    }
}

async fn ticket_for_board_check(
    tx: &mut sqlx::PgConnection,
    principal: &core_domain::entities::Principal,
    org_id: Uuid,
    ticket_id: Uuid,
    op: BoardOp,
) -> ApiResult<core_domain::entities::Ticket> {
    let ticket = core_db::tickets::get_ticket(&mut *tx, org_id, ticket_id).await?;
    authorize_board_op(&mut *tx, principal, ticket.board_id, op).await?;
    Ok(ticket)
}

// --- link reads ------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct LinksView {
    #[schema(value_type = Vec<Object>)]
    pub from: Vec<TicketLink>,
    #[schema(value_type = Vec<Object>)]
    pub to: Vec<TicketLink>,
    pub rollup: core_domain::entities::TicketRollup,
}

/// GET /api/tickets/{id}/links — outgoing + incoming edges + the rollup.
#[utoipa::path(operation_id = "tickets_list_links", get, path = "/api/tickets/{id}/links", tag = "ticket-links", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = LinksView)))]
pub async fn list_links(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<LinksView>> {
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
    ticket_for_board_check(&mut tx, &authed.principal, org_id, id, BoardOp::View).await?;
    let from = core_db::ticket_links::list_links_from(&mut *tx, org_id, id).await?;
    let to = core_db::ticket_links::list_links_to(&mut *tx, org_id, id).await?;
    let rollup = core_db::ticket_links::rollup_for_ticket(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(LinksView { from, to, rollup }))
}

/// GET /api/tickets/{id}/rollup — children/members + folded time + billable money.
#[utoipa::path(operation_id = "tickets_rollup", get, path = "/api/tickets/{id}/rollup", tag = "ticket-links", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = core_domain::entities::TicketRollup)))]
pub async fn get_rollup(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<core_domain::entities::TicketRollup>> {
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
    ticket_for_board_check(&mut tx, &authed.principal, org_id, id, BoardOp::View).await?;
    let rollup = core_db::ticket_links::rollup_for_ticket(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rollup))
}

// --- link mutations ----------------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct CreateTicketLinkRequest {
    #[serde(flatten)]
    pub instant: instant::MutationParams,
    pub to_ticket_id: Uuid,
    pub link_type: TicketLinkType,
}

/// POST /api/tickets/{id}/links — link this ticket (from) to another (to).
#[utoipa::path(operation_id = "tickets_create_link", post, path = "/api/tickets/{id}/links", tag = "ticket-links", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = CreateTicketLinkRequest,
    responses((status = 200, body = Value),
              (status = 404, description = "ticket not found in scope"),
              (status = 409, description = "duplicate link or idempotency key reused")))]
pub async fn create_link(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateTicketLinkRequest>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    if req.to_ticket_id == id {
        return Err(ApiError::Validation(
            "a ticket cannot link to itself".into(),
        ));
    }
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "ticket_link",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let from_ticket = ticket_for_board_check(
            &mut ctx.tx,
            &authed.principal,
            org_id,
            id,
            BoardOp::WorkTicket,
        )
        .await?;
        ticket_for_board_check(
            &mut ctx.tx,
            &authed.principal,
            org_id,
            req.to_ticket_id,
            BoardOp::View,
        )
        .await?;
        let link = TicketLink {
            id: Uuid::now_v7(),
            org_id,
            from_ticket_id: id,
            to_ticket_id: req.to_ticket_id,
            link_type: req.link_type,
            created_by: Some(user_id),
            created_at: chrono::Utc::now(),
        };
        core_db::ticket_links::insert_link(&mut *ctx.tx, &link).await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "ticket.link_created".into(),
                entity_type: Some("ticket".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "to_ticket_id": req.to_ticket_id,
                    "link_type": req.link_type.as_str(),
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        ctx.emit(
            types::TICKET_LINKED,
            "ticket",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketLinkChanged {
                from_ticket_id: id,
                to_ticket_id: req.to_ticket_id,
                link_type: req.link_type.as_str().to_string(),
                linked: true,
                changed_by: Some(user_id),
            })
            .unwrap_or_default(),
        )
        .await?;
        let _ = from_ticket;
        Ok((
            ctx,
            MutationOutcome::Applied(serde_json::json!({ "linked": true, "link_id": link.id })),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/tickets/{id}/links/{link_id} — remove one edge.
#[utoipa::path(operation_id = "tickets_remove_link", delete, path = "/api/tickets/{id}/links/{link_id}", tag = "ticket-links", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("link_id" = Uuid, Path)),
    responses((status = 200, body = Value),
              (status = 404, description = "link not found in scope")))]
pub async fn remove_link(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, link_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        ticket_for_board_check(
            &mut ctx.tx,
            &authed.principal,
            org_id,
            id,
            BoardOp::WorkTicket,
        )
        .await?;
        // Pull the edge first so the event carries both endpoints.
        let from = core_db::ticket_links::list_links_from(&mut *ctx.tx, org_id, id).await?;
        let to = core_db::ticket_links::list_links_to(&mut *ctx.tx, org_id, id).await?;
        let removed_link = from
            .iter()
            .chain(to.iter())
            .find(|l| l.id == link_id)
            .ok_or(ApiError::NotFound)?
            .clone();
        let removed = core_db::ticket_links::remove_link(&mut *ctx.tx, org_id, link_id).await?;
        if !removed {
            return Err(ApiError::NotFound);
        }
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "ticket.link_removed".into(),
                entity_type: Some("ticket".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "link_id": link_id }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        ctx.emit(
            types::TICKET_UNLINKED,
            "ticket",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketLinkChanged {
                from_ticket_id: removed_link.from_ticket_id,
                to_ticket_id: removed_link.to_ticket_id,
                link_type: removed_link.link_type.as_str().to_string(),
                linked: false,
                changed_by: Some(user_id),
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, serde_json::json!({ "removed": true })))
    })
    .await
    .map(Json)
}

// --- merge ------------------------------------------------------------------------

/// Per-field resolution: which side's value wins when the tickets
/// disagree. `"survivor"` (the default) keeps the survivor's value.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FieldResolution {
    Survivor,
    Source,
}

#[derive(Debug, Default, Deserialize, ToSchema, Serialize)]
pub struct MergeResolutions {
    #[serde(default)]
    pub board: Option<FieldResolution>,
    #[serde(default)]
    pub title: Option<FieldResolution>,
    #[serde(default)]
    pub priority: Option<FieldResolution>,
    #[serde(default)]
    pub client: Option<FieldResolution>,
    #[serde(default)]
    pub assignee: Option<FieldResolution>,
}

impl MergeResolutions {
    fn of(&self, field: &str) -> FieldResolution {
        match field {
            "board" => self.board.unwrap_or(FieldResolution::Survivor),
            "title" => self.title.unwrap_or(FieldResolution::Survivor),
            "priority" => self.priority.unwrap_or(FieldResolution::Survivor),
            "client" => self.client.unwrap_or(FieldResolution::Survivor),
            "assignee" => self.assignee.unwrap_or(FieldResolution::Survivor),
            _ => FieldResolution::Survivor,
        }
    }
}

#[derive(Debug, Serialize, ToSchema, serde::Deserialize)]
pub struct MergeConflictEntry {
    /// "board" | "title" | "priority" | "client" | "assignee"
    pub field: String,
    pub resolution: FieldResolution,
    #[schema(value_type = Object, nullable = true)]
    pub survivor_value: Value,
    #[schema(value_type = Object, nullable = true)]
    pub source_value: Value,
}

#[derive(Debug, Serialize, ToSchema, serde::Deserialize)]
pub struct MergeFoldSummary {
    pub comments_folded: i64,
    pub time_entries_folded: i64,
    pub time_minutes_folded: i64,
    pub cost_lines_folded: i64,
    pub expenses_folded: i64,
    pub children_relinked: i64,
}

#[derive(Debug, Serialize, ToSchema, serde::Deserialize)]
pub struct MergeResponse {
    #[schema(value_type = uuid::Uuid)]
    pub survivor_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub source_id: Uuid,
    pub conflicts: Vec<MergeConflictEntry>,
    pub fold: MergeFoldSummary,
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct MergeRequest {
    #[serde(flatten)]
    pub instant: instant::MutationParams,
    pub survivor_id: Uuid,
    #[serde(default)]
    pub resolve: MergeResolutions,
}

/// POST /api/tickets/{id}/merge — merge THIS ticket into `survivor_id`.
/// Children, comments, time entries, cost lines, and expenses fold into
/// the survivor; the source is closed on its own board and marked
/// `merged_into_ticket_id` (its row stays queryable — doctrine #9).
#[utoipa::path(operation_id = "tickets_merge", post, path = "/api/tickets/{id}/merge", tag = "ticket-links", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = MergeRequest,
    responses((status = 200, body = MergeResponse),
              (status = 404, description = "ticket not found in scope"),
              (status = 409, description = "typed merge conflict (self_merge, already_merged, closed_survivor, board_mismatch)"),
              (status = 422, description = "idempotency key reused")))]
pub async fn merge(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<MergeRequest>,
) -> ApiResult<Json<MergeResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "ticket_merge",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }

        // --- typed conflicts, refused BEFORE any write ---------------
        if req.survivor_id == id {
            return Err(ApiError::Validation(
                "self_merge: a ticket cannot merge into itself".into(),
            ));
        }
        let source = ticket_for_board_check(
            &mut ctx.tx,
            &authed.principal,
            org_id,
            id,
            BoardOp::WorkTicket,
        )
        .await?;
        let survivor = ticket_for_board_check(
            &mut ctx.tx,
            &authed.principal,
            org_id,
            req.survivor_id,
            BoardOp::WorkTicket,
        )
        .await?;
        if source.merged_into_ticket_id.is_some() {
            return Err(ApiError::Conflict(
                "already_merged: the source ticket has already been merged".into(),
            ));
        }
        if survivor.merged_into_ticket_id.is_some() {
            return Err(ApiError::Conflict(
                "already_merged: the survivor ticket has already been merged".into(),
            ));
        }
        if matches!(survivor.category, TicketStatusCategory::Closed) {
            return Err(ApiError::Conflict(
                "closed_survivor: the survivor ticket is closed".into(),
            ));
        }
        let mut conflicts: Vec<MergeConflictEntry> = Vec::new();
        if source.board_id != survivor.board_id {
            match req.resolve.of("board") {
                FieldResolution::Source => {
                    conflicts.push(MergeConflictEntry {
                        field: "board".into(),
                        resolution: FieldResolution::Source,
                        survivor_value: serde_json::to_value(survivor.board_id).unwrap_or(Value::Null),
                        source_value: serde_json::to_value(source.board_id).unwrap_or(Value::Null),
                    });
                }
                FieldResolution::Survivor => {
                    return Err(ApiError::Conflict(
                        "board_mismatch: tickets live on different boards; resolve with resolve.board=\"source\" to move the survivor onto the source's board".into(),
                    ));
                }
            }
        }
        for (field, s_val, v_val) in [
            ("title", source.title.clone(), survivor.title.clone()),
            ("priority", source.priority.as_str().to_string(), survivor.priority.as_str().to_string()),
        ] {
            if s_val != v_val {
                let resolution = req.resolve.of(field);
                conflicts.push(MergeConflictEntry {
                    field: field.into(),
                    resolution,
                    survivor_value: serde_json::to_value(v_val).unwrap_or(Value::Null),
                    source_value: serde_json::to_value(s_val).unwrap_or(Value::Null),
                });
            }
        }
        if source.client_id != survivor.client_id {
            conflicts.push(MergeConflictEntry {
                field: "client".into(),
                resolution: req.resolve.of("client"),
                survivor_value: serde_json::to_value(survivor.client_id).unwrap_or(Value::Null),
                source_value: serde_json::to_value(source.client_id).unwrap_or(Value::Null),
            });
        }
        if source.assignee_id != survivor.assignee_id {
            conflicts.push(MergeConflictEntry {
                field: "assignee".into(),
                resolution: req.resolve.of("assignee"),
                survivor_value: serde_json::to_value(survivor.assignee_id).unwrap_or(Value::Null),
                source_value: serde_json::to_value(source.assignee_id).unwrap_or(Value::Null),
            });
        }

        // --- field application per resolution ------------------------
        let apply_source_title = conflicts
            .iter()
            .any(|c| c.field == "title" && c.resolution == FieldResolution::Source);
        let apply_source_priority = conflicts
            .iter()
            .any(|c| c.field == "priority" && c.resolution == FieldResolution::Source);
        let apply_source_client = conflicts
            .iter()
            .any(|c| c.field == "client" && c.resolution == FieldResolution::Source);
        let apply_source_assignee = conflicts
            .iter()
            .any(|c| c.field == "assignee" && c.resolution == FieldResolution::Source);
        core_db::tickets::update_ticket(
            &mut *ctx.tx,
            org_id,
            req.survivor_id,
            core_db::tickets::TicketPatch {
                title: if apply_source_title { Some(source.title.as_str()) } else { None },
                body: None,
                priority: if apply_source_priority {
                    Some(source.priority)
                } else {
                    None
                },
                assignee_id: if apply_source_assignee {
                    Some(source.assignee_id)
                } else {
                    None
                },
                client_id: if apply_source_client {
                    Some(source.client_id)
                } else {
                    None
                },
                site_id: None,
                contact_id: None,
                due_at: None,
                type_id: None,
                category_id: None,
            },
        )
        .await?;
        if conflicts
            .iter()
            .any(|c| c.field == "board" && c.resolution == FieldResolution::Source)
        {
            // Status remap through the semantic category (the move
            // doctrine): the FIRST source-board status sharing the
            // survivor's current category in deterministic order wins.
            let target_statuses =
                core_db::boards::list_statuses(&mut *ctx.tx, source.board_id).await?;
            let mut candidates: Vec<_> = target_statuses
                .iter()
                .filter(|s| s.category == survivor.category)
                .collect();
            candidates.sort_by(|a, b| {
                (a.sort_order, &a.name, a.id).cmp(&(b.sort_order, &b.name, b.id))
            });
            let target = candidates.first().ok_or_else(|| {
                ApiError::Conflict(
                    "board_mismatch: the source board has no status sharing the survivor's category".into(),
                )
            })?;
            core_db::tickets::move_to_board(
                &mut *ctx.tx,
                org_id,
                req.survivor_id,
                source.board_id,
                target.id,
            )
            .await?;
        }

        // --- the fold (one transaction, audited) ---------------------
        let fold = core_db::ticket_links::merge_fold(&mut ctx.tx, org_id, id, req.survivor_id)
            .await?;

        // Close the source on ITS board: the first closed-category
        // status in deterministic order (move_to_board stamps
        // resolved_at on the category crossing).
        let source_statuses =
            core_db::boards::list_statuses(&mut *ctx.tx, source.board_id).await?;
        let mut closed: Vec<_> = source_statuses
            .iter()
            .filter(|s| s.category == TicketStatusCategory::Closed)
            .collect();
        closed.sort_by(|a, b| (a.sort_order, &a.name, a.id).cmp(&(b.sort_order, &b.name, b.id)));
        let closed_status = closed.first().ok_or_else(|| {
            ApiError::Conflict(
                "the source's board has no closed-category status to fold it into".into(),
            )
        })?;
        core_db::tickets::move_to_board(
            &mut *ctx.tx,
            org_id,
            id,
            source.board_id,
            closed_status.id,
        )
        .await?;

        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "ticket.merged".into(),
                entity_type: Some("ticket".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "survivor_id": req.survivor_id,
                    "comments_folded": fold.comments_folded,
                    "time_entries_folded": fold.time_entries_folded,
                    "cost_lines_folded": fold.cost_lines_folded,
                    "expenses_folded": fold.expenses_folded,
                    "children_relinked": fold.children_relinked,
                    "conflicts": conflicts.iter().map(|c| c.field.clone()).collect::<Vec<_>>(),
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        ctx.emit(
            types::TICKET_MERGED,
            "ticket",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketMerged {
                source_id: id,
                source_number: source.number.0.clone(),
                survivor_id: req.survivor_id,
                survivor_number: survivor.number.0.clone(),
                comments_folded: fold.comments_folded,
                time_entries_folded: fold.time_entries_folded,
                time_minutes_folded: fold.time_minutes_folded,
                cost_lines_folded: fold.cost_lines_folded,
                expenses_folded: fold.expenses_folded,
                children_relinked: fold.children_relinked,
                conflicts: conflicts.iter().map(|c| c.field.clone()).collect(),
                merged_by: Some(user_id),
                occurred_at: chrono::Utc::now(),
            })
            .unwrap_or_default(),
        )
        .await?;

        Ok((
            ctx,
            MutationOutcome::Applied(MergeResponse {
                survivor_id: req.survivor_id,
                source_id: id,
                conflicts,
                fold: MergeFoldSummary {
                    comments_folded: fold.comments_folded,
                    time_entries_folded: fold.time_entries_folded,
                    time_minutes_folded: fold.time_minutes_folded,
                    cost_lines_folded: fold.cost_lines_folded,
                    expenses_folded: fold.expenses_folded,
                    children_relinked: fold.children_relinked,
                },
            }),
        ))
    })
    .await?;

    let response: MergeResponse = outcome.into_response()?;
    Ok(Json(response))
}
