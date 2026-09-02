//! Undo as a core capability (TASKLIST IF7, Wave 9 lane 5; binding
//! contract in `docs/instant-first.md` "Undo is the escape hatch").
//!
//! Purpose: the read model + write path that make autosave feel safe.
//! Removing explicit Save transfers the safety burden to Undo, so undo is
//! first-class in the core, not a client-side stack:
//!
//! * `GET /api/tickets/{id}/undo-stack` — the per-field / per-record
//!   applied-state history over the 0028 write trail (via the 0041
//!   `ops.v1_record_undo_stack` / `ops.v1_field_undo_stack` views),
//!   newest first, plus the record's undo `mode`.
//! * `POST /api/tickets/{id}/revert` — steps back through server-accepted
//!   edits as a NORMAL authorized idempotent write through the IF1 path
//!   (`claim` → version gate → the same domain write as any patch →
//!   `record`). The revert lands on the trail AGAIN (seq increments, op
//!   `revert`), emits the registered `ticket.reverted` event, and appends
//!   an audited `ticket.reverted` row — the append-only trail is never
//!   rewritten; undo is a new action in it.
//!
//! Authorization = the same as the original write: `tickets:edit` plus the
//! board's `WorkTicket` op, identical to the autosave PATCH path.
//!
//! Modes (the near-total / actioned line):
//! * Record open (status not in the closed category) → `open`: undo is
//!   near-total — a whole-record step (restore the full prior snapshot)
//!   or a per-field revert of any restorable write.
//! * Record closed (status in the closed category) → `reverts_only`:
//!   closing is a checkpoint; whole-record reverts are refused with the
//!   typed `undo_reverts_only` 409 and undo is limited to explicit,
//!   actioned per-field reverts that name exactly what they restore.
//!   Reopening the record returns it to `open`.
//!
//! Non-purpose: no client keybinding here (the Ctrl/Cmd+Z binding is a
//! web stretch goal riding the SDK); no revert of creates (a create has
//! no prior values to restore — withdrawal is a delete decision, not a
//! field restore); lifecycle values are never restored by this path
//! (status moves keep going through the guarded /status endpoint).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome, MutationParams};
use crate::org_tx;
use crate::routes::board_access::authorize_board_op;
use crate::routes::custom_fields::TicketWithFields;
use crate::routes::ApiGeneration;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::events::{types, TicketReverted};
use core_domain::policy::{Action, BoardOp, Module, Permission};
use core_domain::status::{Priority, TicketStatusCategory};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

/// The ticket fields the value-restore revert path can restore — exactly
/// the autosave-editable set the PATCH endpoint accepts. Lifecycle state
/// (`status_id`) and bookkeeping columns are deliberately absent.
const RESTORABLE_FIELDS: &[&str] = &[
    "title",
    "body",
    "priority",
    "assignee_id",
    "client_id",
    "site_id",
    "contact_id",
    "due_at",
];

fn tickets(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Tickets,
        action,
    }
}

pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(
            &format!("{base}/tickets/{{id}}/undo-stack"),
            get(undo_stack),
        )
        .route(&format!("{base}/tickets/{{id}}/revert"), post(revert))
}

// --- read model: GET /tickets/{id}/undo-stack --------------------------------

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct UndoStackQuery {
    /// Maximum steps / field rows returned (1–200, default 50). Newest
    /// first; the first step's `seq` is the record's current version.
    #[serde(default)]
    pub limit: Option<i64>,
}

/// The record's undo posture: `open` = near-total undo; `reverts_only` =
/// explicit actioned per-field reverts only (record is closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UndoMode {
    Open,
    RevertsOnly,
}

/// One applied write on the record — one step of the undo stack.
#[derive(Debug, Serialize, ToSchema)]
pub struct UndoStep {
    /// Trail row id (stable reference for actioned reverts).
    pub write_id: Uuid,
    /// Per-entity monotonic write number (the version this write produced).
    pub seq: i64,
    /// Machine op name (`create`, `update`, `status_change`, `revert`, ...).
    pub op: String,
    pub actor_id: Uuid,
    /// `human` | `agent` | ... (the audit actor vocabulary).
    pub actor_kind: String,
    #[schema(nullable = true)]
    pub idempotency_key: Option<String>,
    /// Prior entity snapshot; `null` on creates.
    #[schema(value_type = Object, nullable = true)]
    pub before: Option<Value>,
    /// Applied entity snapshot.
    #[schema(value_type = Object)]
    pub after: Value,
    /// The revert path can restore this write's prior values (`create`
    /// steps are visible history but not value-restorable).
    pub restorable: bool,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
}

/// One field of one applied write — per-field undo granularity.
#[derive(Debug, Serialize, ToSchema)]
pub struct UndoFieldStep {
    pub write_id: Uuid,
    pub seq: i64,
    pub op: String,
    pub field: String,
    /// Value before the write (absent keys read as `null`).
    #[schema(value_type = Object, nullable = true)]
    pub prior: Option<Value>,
    /// Value after the write (absent keys read as `null`).
    #[schema(value_type = Object, nullable = true)]
    pub new_value: Option<Value>,
    pub changed: bool,
    pub actor_id: Uuid,
    pub actor_kind: String,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UndoStackResponse {
    pub entity: String,
    pub entity_id: Uuid,
    /// Near-total (`open`) vs actioned-reverts-only (`reverts_only`).
    pub mode: UndoMode,
    /// The record's current version — echo it as `version` on a revert.
    pub current_version: i64,
    /// Applied writes, newest first.
    pub steps: Vec<UndoStep>,
    /// Per-field history, newest write first.
    pub fields: Vec<UndoFieldStep>,
}

/// GET /api/tickets/{id}/undo-stack — the per-field / per-record
/// applied-state history read model for one record.
#[utoipa::path(
    operation_id = "tickets_undo_stack",
    get,
    path = "/api/tickets/{id}/undo-stack",
    tag = "tickets",
    security(("bearer" = [])),
    params(("id" = Uuid, Path), UndoStackQuery),
    responses(
        (status = 200, body = UndoStackResponse),
        (status = 404, description = "ticket not visible in scope")
    )
)]
pub async fn undo_stack(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Query(query): Query<UndoStackQuery>,
) -> ApiResult<Json<UndoStackResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let limit = query.limit.unwrap_or(50);

    // The record gates the stack: an invisible ticket exposes nothing.
    let ticket = core_db::tickets::get_ticket(&state.pool, org_id, id).await?;
    let mode = if ticket.category == TicketStatusCategory::Closed {
        UndoMode::RevertsOnly
    } else {
        UndoMode::Open
    };

    let steps = core_db::undo::record_stack(&state.pool, org_id, "ticket", id, limit).await?;
    let current_version = steps.first().map(|s| s.seq).unwrap_or(0);
    let fields = core_db::undo::field_stack(&state.pool, org_id, "ticket", id, limit).await?;

    Ok(Json(UndoStackResponse {
        entity: "ticket".into(),
        entity_id: id,
        mode,
        current_version,
        steps: steps
            .into_iter()
            .map(|s| UndoStep {
                write_id: s.id,
                seq: s.seq,
                op: s.op,
                actor_id: s.actor_id,
                actor_kind: s.actor_kind,
                idempotency_key: s.idempotency_key,
                before: s.before,
                after: s.after,
                restorable: s.restorable,
                created_at: s.created_at,
            })
            .collect(),
        fields: fields
            .into_iter()
            .map(|f| UndoFieldStep {
                write_id: f.id,
                seq: f.seq,
                op: f.op,
                field: f.field,
                prior: f.prior,
                new_value: f.new_value,
                changed: f.changed,
                actor_id: f.actor_id,
                actor_kind: f.actor_kind,
                created_at: f.created_at,
            })
            .collect(),
    }))
}

// --- write path: POST /tickets/{id}/revert -----------------------------------

/// POST /api/tickets/{id}/revert body.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RevertTicketRequest {
    /// The write-trail `seq` to step back over (from the undo stack).
    pub seq: i64,
    /// Explicit per-field revert: restore ONLY these fields to their values
    /// at the target write. Omit for a whole-record step (rejected with
    /// `undo_reverts_only` while the record is closed).
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version, exactly as any autosave write.
    #[serde(flatten, default)]
    pub instant: MutationParams,
}

/// What the revert restored.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RevertDetail {
    /// The write-trail seq the revert stepped back over.
    pub reverted_seq: i64,
    /// Record fields whose values were restored.
    pub fields: Vec<String>,
    /// Whether this was a whole-record step (no explicit field list).
    pub whole_record: bool,
}

/// The reverted ticket with the mutation envelope, plus what was reverted.
/// Deserializable so an IF1 idempotent replay can reconstruct the exact
/// stored response.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TicketRevertResponse {
    #[serde(flatten)]
    pub ticket: TicketWithFields,
    pub mutation: instant::MutationMeta,
    pub revert: RevertDetail,
}

/// The prior values a revert restores, parsed from the trail snapshot.
#[derive(Debug, Deserialize)]
struct SnapshotValues {
    title: String,
    body: Option<String>,
    priority: Priority,
    #[serde(default)]
    assignee_id: Option<Uuid>,
    #[serde(default)]
    client_id: Option<Uuid>,
    #[serde(default)]
    site_id: Option<Uuid>,
    #[serde(default)]
    contact_id: Option<Uuid>,
    #[serde(default)]
    due_at: Option<DateTime<Utc>>,
}

/// POST /api/tickets/{id}/revert — undo one server-accepted edit by
/// restoring its prior values as a NORMAL authorized idempotent write
/// (IF1 envelope end to end). Appends a new trail row (op `revert`),
/// emits `ticket.reverted` on the event bus, and writes the audited
/// `ticket.reverted` trail row — the append-only history is never
/// rewritten.
#[utoipa::path(
    operation_id = "tickets_revert",
    post,
    path = "/api/tickets/{id}/revert",
    tag = "tickets",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = RevertTicketRequest,
    responses(
        (status = 200, body = TicketRevertResponse),
        (status = 409, description = "version conflict, idempotency key reuse, or undo_reverts_only (closed record, whole-record revert)"),
        (status = 422, description = "unknown target seq, non-restorable step/field, or nothing to restore")
    )
)]
pub async fn revert(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<RevertTicketRequest>,
) -> ApiResult<Json<TicketRevertResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    // Authorization = the same as the original write (autosave PATCH).
    require_permission(&authed.principal, tickets(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let version = req.instant.version;
    let whole_record = req.fields.is_none();
    let target_seq = req.seq;

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
        let before_ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            before_ticket.board_id,
            BoardOp::WorkTicket,
        )
        .await?;

        // The revert target must exist on THIS record's trail. A miss is a
        // validation error naming the seq (the record itself is visible, so
        // this is a bad reference, not an RLS-sparse scope miss).
        let step = core_db::undo::trail_step(&mut *ctx.tx, org_id, "ticket", id, target_seq)
            .await?
            .ok_or_else(|| {
                ApiError::Validation(format!(
                    "no write-trail entry with seq {target_seq} for this ticket"
                ))
            })?;
        if !step.restorable {
            return Err(ApiError::Validation(
                "cannot revert a create: it has no prior values to restore".into(),
            ));
        }
        let before = step.before.unwrap_or_default();
        let snapshot: SnapshotValues = serde_json::from_value(before.clone())
            .map_err(|e| ApiError::Validation(format!("prior snapshot unreadable: {e}")))?;

        // Mode gate: near-total while the record is open; after close only
        // explicit, actioned per-field reverts (never a silent whole-record
        // rewrite past the close checkpoint).
        let closed = before_ticket.category == TicketStatusCategory::Closed;
        let fields: Vec<String> = match &req.fields {
            None => {
                if closed {
                    return Err(ApiError::UndoRevertsOnly);
                }
                RESTORABLE_FIELDS.iter().map(|s| s.to_string()).collect()
            }
            Some(list) => {
                if list.is_empty() {
                    return Err(ApiError::Validation(
                        "fields must not be empty; omit it for a whole-record revert".into(),
                    ));
                }
                let mut seen = Vec::with_capacity(list.len());
                for field in list {
                    if !RESTORABLE_FIELDS.contains(&field.as_str()) {
                        return Err(ApiError::Validation(format!(
                            "unknown or non-restorable field: {field}"
                        )));
                    }
                    if !seen.contains(field) {
                        seen.push(field.clone());
                    }
                }
                seen
            }
        };

        // Nothing to restore ⇒ refuse rather than append a meaningless
        // no-op write to the trail.
        if !revert_changes_anything(&snapshot, &before_ticket, &fields) {
            return Err(ApiError::Validation(
                "revert would change nothing: the record already matches the prior state".into(),
            ));
        }

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "ticket",
                entity_id: id,
                version,
            },
        )
        .await?;
        core_db::tickets::update_ticket(
            &mut *ctx.tx,
            org_id,
            id,
            core_db::tickets::TicketPatch {
                title: fields
                    .iter()
                    .any(|f| f == "title")
                    .then_some(snapshot.title.as_str()),
                body: fields
                    .iter()
                    .any(|f| f == "body")
                    .then_some(snapshot.body.as_deref()),
                priority: fields
                    .iter()
                    .any(|f| f == "priority")
                    .then_some(snapshot.priority),
                assignee_id: fields
                    .iter()
                    .any(|f| f == "assignee_id")
                    .then_some(snapshot.assignee_id),
                client_id: fields
                    .iter()
                    .any(|f| f == "client_id")
                    .then_some(snapshot.client_id),
                site_id: fields
                    .iter()
                    .any(|f| f == "site_id")
                    .then_some(snapshot.site_id),
                contact_id: fields
                    .iter()
                    .any(|f| f == "contact_id")
                    .then_some(snapshot.contact_id),
                due_at: fields
                    .iter()
                    .any(|f| f == "due_at")
                    .then_some(snapshot.due_at),
            },
        )
        .await?;
        let after = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await?;

        // The audited revert: event bus (contract-registered payload) AND
        // the append-only audit trail. Undo never rewrites either.
        let now = Utc::now();
        ctx.emit(
            types::TICKET_REVERTED,
            "ticket",
            Some(after.id.to_string().as_str()),
            &serde_json::to_value(TicketReverted {
                ticket_id: after.id,
                board_id: after.board_id,
                assignee_id: after.assignee_id,
                reverted_seq: target_seq,
                fields: fields.clone(),
                whole_record,
                reverted_by: Some(user_id),
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
                action: "ticket.reverted".into(),
                entity_type: Some("ticket".into()),
                entity_id: Some(after.id.to_string()),
                detail: serde_json::json!({
                    "reverted_seq": target_seq,
                    "target_op": step.op,
                    "target_at": step.created_at,
                    "fields": fields.clone(),
                    "whole_record": whole_record,
                    "from_title": before_ticket.title.clone(),
                    "to_title": after.title.clone(),
                }),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;

        let enriched = TicketWithFields::load(&mut *ctx.tx, org_id, after).await?;
        let detail = RevertDetail {
            reverted_seq: target_seq,
            fields,
            whole_record,
        };
        // The stored idempotency payload is the FULL response shape (entity
        // fields + revert detail; `record` merges the mutation metadata) so
        // the fresh response and a replay are byte-identical by
        // construction: both are deserialized from the same stored value.
        let mut stored = serde_json::to_value(StoredRevertResponse {
            ticket: &enriched,
            revert: &detail,
        })
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("revert response serialize: {e}")))?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "ticket",
                entity_id: id,
                op: "revert",
                seq: write_seq,
                before: Some(serde_json::to_value(&before_ticket).unwrap_or(Value::Null)),
                after: serde_json::to_value(&enriched.ticket).unwrap_or(Value::Null),
                updated_at: enriched.ticket.updated_at,
                key: key.as_deref(),
                // IF7 revert riding the normal path: flagged, never a
                // rewritten row (IF9 keeps the trail append-only).
                provenance: instant::ProvenanceContext {
                    is_undo: true,
                    ..Default::default()
                },
                payload: &stored,
            },
        )
        .await?;
        if let Some(map) = stored.as_object_mut() {
            map.insert(
                "mutation".to_string(),
                serde_json::to_value(&mutation)
                    .map_err(|e| ApiError::Internal(anyhow::anyhow!("meta serialize: {e}")))?,
            );
        }
        let response: TicketRevertResponse = serde_json::from_value(stored)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("revert response decode: {e}")))?;
        Ok((ctx, MutationOutcome::Applied(response)))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// The response body minus the mutation metadata (which `record` merges
/// into the stored idempotency payload).
#[derive(Serialize)]
struct StoredRevertResponse<'a> {
    #[serde(flatten)]
    ticket: &'a TicketWithFields,
    revert: &'a RevertDetail,
}

/// Does restoring the selected snapshot fields actually move any value?
fn revert_changes_anything(
    snapshot: &SnapshotValues,
    current: &core_domain::entities::Ticket,
    fields: &[String],
) -> bool {
    for field in fields {
        let differs = match field.as_str() {
            "title" => snapshot.title != current.title,
            "body" => snapshot.body != current.body,
            "priority" => snapshot.priority != current.priority,
            "assignee_id" => snapshot.assignee_id != current.assignee_id,
            "client_id" => snapshot.client_id != current.client_id,
            "site_id" => snapshot.site_id != current.site_id,
            "contact_id" => snapshot.contact_id != current.contact_id,
            "due_at" => snapshot.due_at != current.due_at,
            _ => false,
        };
        if differs {
            return true;
        }
    }
    false
}
