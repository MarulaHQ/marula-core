//! Ticket endpoints: lifecycle, structured notes, pre-close checklists.
//! Every mutation writes the outbox event in the SAME transaction as the
//! data change (§4.2) and updates SLA clock state (§1).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    response::{IntoResponse, Response},
    Json,
};
use core_domain::entities::{Actor, AuditEvent, ChecklistItem, ChecklistProgress};
use core_domain::events::types;
use core_domain::policy::{Module, Permission};
use core_domain::status::{NoteKind, Priority, TicketStatusCategory};
use core_domain::workflow;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use super::board_access::authorize_board_op;
use super::custom_fields::{TicketFieldInput, TicketWithFields};
use core_domain::policy::BoardOp;

fn tickets(action: core_domain::policy::Action) -> Permission {
    Permission::Matrix {
        module: Module::Tickets,
        action,
    }
}

/// The board's close-enforcement posture: ACTIVE only when a checklist
/// template exists AND the board's require_checklist_close toggle is on
/// (defaults-first doctrine — see migration 0016's header).
async fn checklist_enforcement(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    board_id: Uuid,
) -> ApiResult<bool> {
    let board = core_db::boards::get_board(&mut *conn, org_id, board_id).await?;
    if !board.require_checklist_close {
        return Ok(false);
    }
    let has_template = core_db::checklists::get_template_for_board(&mut *conn, org_id, board_id)
        .await?
        .is_some();
    Ok(has_template)
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateTicketRequest {
    pub board_id: Uuid,
    pub title: String,
    pub body: Option<String>,
    pub priority: Priority,
    pub client_id: Option<Uuid>,
    pub site_id: Option<Uuid>,
    pub contact_id: Option<Uuid>,
    pub assignee_id: Option<Uuid>,
    pub due_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Initial custom-field values; each is validated against its
    /// definition. REQUIRED definitions must all be present (TASKLIST 2.17).
    #[serde(default)]
    pub custom_fields: Vec<TicketFieldInput>,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PatchTicketRequest {
    pub title: Option<String>,
    pub body: Option<String>,
    pub priority: Option<Priority>,
    /// `Some(Some(id))` assigns; `Some(None)` unassigns; `None` = no change.
    pub assignee_id: Option<Option<Uuid>>,
    pub client_id: Option<Option<Uuid>>,
    pub site_id: Option<Option<Uuid>>,
    pub contact_id: Option<Option<Uuid>>,
    pub due_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    /// Custom-field values to upsert. Partial semantics: only listed fields
    /// change, each validated against its definition.
    pub custom_fields: Option<Vec<TicketFieldInput>>,
    /// Classification (Wave 11 lane 3): `"type_id": null` clears,
    /// an id sets (the node must exist in this org and not be retired),
    /// an absent member leaves it untouched. Purely descriptive: behavior
    /// binds to the semantic categories, never these trees.
    #[serde(default, deserialize_with = "deserialize_optional_uuid")]
    pub type_id: Option<Option<Uuid>>,
    #[serde(default, deserialize_with = "deserialize_optional_uuid")]
    pub category_id: Option<Option<Uuid>>,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ChangeStatusRequest {
    pub status_id: Uuid,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct AddCommentRequest {
    pub body: String,
    pub kind: NoteKind,
    /// Attachments uploaded earlier against this ticket (composer flow:
    /// upload first, link when the comment is created). Absent/empty in
    /// older clients — purely additive.
    #[serde(default)]
    pub attachment_ids: Vec<Uuid>,
    /// IF2 delete-on-submit (Wave 17 lane 11, §5.2): the composer autosave
    /// draft's idempotency key. When a comment with this key commits, the
    /// author's `note_drafts` row for the ticket is deleted in the SAME
    /// transaction — the draft never resurrects after submit. Absent in
    /// legacy clients — purely additive.
    #[serde(default)]
    pub draft_key: Option<String>,
    /// Instant-first autosave parameters (IF1): idempotency key. Absent in
    /// legacy clients — purely additive. (Comments are fresh entities, so
    /// there is no version to match.)
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
    /// Per-message greeting/signature the composer wrote (Wave 42). Never a
    /// stored preference. A locked org refuses it: an override is a
    /// personal signature that was never stored, so a workspace that
    /// locked signatures must not be routable around one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email_envelope: Option<EmailEnvelopeOverride>,
}

/// The greeting and signature a composer wrote for THIS message (Wave 42).
///
/// Purely additive and purely per-message: it changes what goes out with
/// this one note and is never stored as a preference. The composer
/// prefills both fields from the sender's own default, so this carries the
/// EDIT, and the composer is the only thing that should set it — the
/// server never merges it into a preference behind the sender's back.
#[derive(Debug, Clone, Default, Serialize, ToSchema, PartialEq, Eq)]
pub struct EmailEnvelopeOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub greeting: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AddChecklistItemRequest {
    pub text: String,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SetChecklistItemRequest {
    pub checked: bool,
    /// Shared not-applicable state (Wave 12 lane 6): `true` marks the item
    /// N-A as ORG-WIDE server truth and clears the checkmark (the states
    /// are mutually exclusive; checked semantics are untouched for callers
    /// that omit it — legacy payloads default to `false` and behave
    /// exactly as before). N-A REQUIRED items are exempt from pre-close
    /// enforcement and derived progress.
    #[serde(default)]
    pub na: bool,
    /// Server-composed ledger reason (Wave 17 lane 11, §5.1): MANDATORY
    /// (non-empty, ≤2000 chars) when this call sets N-A (na: true) or
    /// reverses a terminal state (reopen); ignored for plain checks. The
    /// reason text lands in the ledger as the auto-appended internal note
    /// composed and inserted in THIS transaction — never a client-side
    /// second post.
    #[serde(default)]
    pub note_reason: Option<String>,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// One unassigned task on the close gate's typed 409 (§4.1): the row the
/// UI must resolve (Option A batch-close / Option B manual work) — decision
/// text, and whether it is a REQUIRED template item (enforcement pill).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TaskGateItem {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub item_id: Uuid,
    pub text: String,
    pub required: bool,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct TicketListQuery {
    pub board_id: Option<Uuid>,
    pub assignee_id: Option<Uuid>,
    pub client_id: Option<Uuid>,
    pub category: Option<TicketStatusCategory>,
    pub priority: Option<Priority>,
    /// Comma-separated tag ids (Wave 11 lane 9): AND semantics — tickets
    /// must carry every listed tag.
    #[serde(default)]
    pub tags: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
    /// Keyset cursor (Wave 15 lane 3): the opaque `x-next-cursor` value
    /// from the previous page's last row. Resumes the ranked list
    /// strictly after that row — deep pages stop paying O(offset) scan
    /// cost (docs/perf.md F2). When present, `offset` is ignored.
    #[serde(default)]
    pub cursor: Option<String>,
}

/// Instant-first mutation response (IF2): the enriched ticket fields at the
/// top level (unchanged shape) plus the additive `mutation` metadata — the
/// applied snapshot, monotonic per-entity `seq`/`version`, and the prior
/// and new values that make undo a core capability.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TicketMutationResponse {
    #[serde(flatten)]
    pub ticket: TicketWithFields,
    pub mutation: instant::MutationMeta,
}

/// Same additive envelope for status transitions (plain ticket body).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TicketStatusMutationResponse {
    #[serde(flatten)]
    pub ticket: core_domain::entities::Ticket,
    pub mutation: instant::MutationMeta,
}

/// Same additive envelope for comments (plain comment body).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TicketCommentMutationResponse {
    #[serde(flatten)]
    pub comment: core_domain::entities::TicketComment,
    pub mutation: instant::MutationMeta,
}

/// Same additive envelope for checklist writes: the full checklist view at
/// the top level (unchanged shape) plus the read-your-write metadata for
/// the affected item. `appended_note` carries the server-composed ledger
/// note when the transition wrote one (§3.3/§3.4) — absent otherwise.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TicketChecklistMutationResponse {
    #[serde(flatten)]
    pub view: TicketChecklistView,
    pub mutation: instant::MutationMeta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appended_note: Option<core_domain::entities::TicketComment>,
}

/// POST /api/tickets/{id}/checklist/complete-all request (Wave 17 lane 11
/// §5.3): the close gate's Option A batch — every listed item flips to
/// `checked` and each lands its auto-append note, ALL in one transaction
/// with per-item idempotency keys (duplicate-proof at the envelope, scored
/// as REPORTED auto notes via `workflow_meta.auto = true`).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CompleteAllRequest {
    /// Checklist items to complete. Items already checked/N-A are skipped
    /// (idempotent); unknown ids 404 the whole call (batch atomicity).
    pub task_ids: Vec<Uuid>,
    /// Instant-first autosave parameters (IF1): idempotency key + version.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// Batch response: full updated checklist view + the appended notes in
/// item order so the client reconciles in one round trip.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TicketChecklistBatchResponse {
    #[serde(flatten)]
    pub view: TicketChecklistView,
    pub appended_notes: Vec<core_domain::entities::TicketComment>,
    pub mutation: instant::MutationMeta,
}

/// Live SLA clock readout for one ticket (badge + countdown surfaces).
#[derive(Debug, Serialize, ToSchema)]
pub struct TicketSlaStatus {
    /// False when no SLA policy applies to the ticket — a typed empty
    /// state, never a 404 (the endpoint is polled by the ticket header).
    pub applied: bool,
    pub state: Option<core_domain::entities::TicketSlaState>,
    /// Business minutes left on the resolution budget (negative = over).
    pub minutes_remaining: Option<i64>,
    /// True while a Waiting-category pause is open.
    pub paused: bool,
    pub breached: bool,
}

fn default_limit() -> i64 {
    50
}

/// Keyset cursor codec (Wave 15 lane 3): opaque base64 of
/// `v1|{rank}|{due_at}|{created_at}|{id}`, same URL-safe base64 pattern the
/// audit/provenance cursors use. `due_at` empty string = NULL (NULLS LAST
/// in the list order). The version field lets the codec evolve without
/// silently mangling old cursors.
const CURSOR_VERSION: &str = "v1";

fn encode_cursor(c: core_db::tickets::TicketCursor) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
    use base64::Engine;
    let due = c.due_at.map(|d| d.to_rfc3339()).unwrap_or_default();
    B64.encode(
        format!(
            "{CURSOR_VERSION}|{}|{due}|{}|{}",
            c.priority_rank,
            c.created_at.to_rfc3339(),
            c.id
        )
        .as_bytes(),
    )
}

fn decode_cursor(raw: &str) -> Option<core_db::tickets::TicketCursor> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
    use base64::Engine;
    let text = String::from_utf8(B64.decode(raw.as_bytes()).ok()?).ok()?;
    let parts: Vec<&str> = text.split('|').collect();
    if parts.len() != 5 || parts[0] != CURSOR_VERSION {
        return None;
    }
    let priority_rank: i32 = parts[1].parse().ok()?;
    if !(0..=3).contains(&priority_rank) {
        return None;
    }
    let due_at = if parts[2].is_empty() {
        None
    } else {
        Some(parts[2].parse().ok()?)
    };
    Some(core_db::tickets::TicketCursor {
        priority_rank,
        due_at,
        created_at: parts[3].parse().ok()?,
        id: parts[4].parse().ok()?,
    })
}

/// The list order's primary rank, shared by the ORDER BY expression and
/// the cursor encoding.
fn priority_rank(p: Priority) -> i32 {
    match p {
        Priority::P1 => 0,
        Priority::P2 => 1,
        Priority::P3 => 2,
        Priority::P4 => 3,
    }
}

/// Distinguishes `"type_id": null` (clear) from an absent member — the
/// classification patch contract (Wave 11 lane 3).
fn deserialize_optional_uuid<'de, D>(deserializer: D) -> Result<Option<Option<Uuid>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::<Uuid>::deserialize(deserializer)?))
}

/// GET /api/tickets — filtered, priority-ordered list; each row carries its
/// custom-field values (TASKLIST 2.17) fetched in one batched query.
/// Pagination is envelope-compatible: the body stays the plain ticket
/// array; deep-page walks pass the opaque `cursor` query param (from the
/// previous page's `x-next-cursor` response header) and the server resumes
/// strictly after that row — keyset, not O(offset) scan (docs/perf.md F2).
#[utoipa::path(operation_id = "tickets_list", get, path = "/api/tickets", tag = "tickets", security(("bearer" = [])),
    params(TicketListQuery),
    responses(
        (status = 200, body = [TicketWithFields],
         headers(("x-next-cursor" = String,
                  description = "Opaque keyset cursor for the next page; present when this page is full and the walk may continue")))
    )
)]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<TicketListQuery>,
) -> ApiResult<Response> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let org_id = authed.principal.user.org_id;
    let limit = q.limit.clamp(1, 200);
    let anchor = q.cursor.as_deref().and_then(decode_cursor);
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::tickets::list_tickets(
        &mut *tx,
        org_id,
        core_db::tickets::TicketFilter {
            board_id: q.board_id,
            assignee_id: q.assignee_id,
            client_id: q.client_id,
            category: q.category,
            priority: q.priority,
            tag_ids: q.tags.as_deref().and_then(|s| {
                let ids: Result<Vec<Uuid>, _> = s
                    .split(',')
                    .filter(|t| !t.trim().is_empty())
                    .map(|t| Uuid::parse_str(t.trim()))
                    .collect();
                ids.ok()
            }),
            limit,
            // Keyset and offset are different callers' mechanisms: a
            // cursor walk resumes on its own anchor, so offset zeroes.
            offset: if anchor.is_some() { 0 } else { q.offset.max(0) },
            cursor: anchor,
        },
    )
    .await?;
    let enriched = TicketWithFields::load_many(&mut *tx, org_id, rows).await?;
    tx.commit().await.map_err(ApiError::from)?;
    // A full page hints another exists; anchor it to the LAST row (the
    // sort order is global + id-tiebroken, so this anchor is exact).
    let next_cursor = if enriched.len() as i64 >= limit {
        enriched.last().map(|t| core_db::tickets::TicketCursor {
            priority_rank: priority_rank(t.ticket.priority),
            due_at: t.ticket.due_at,
            created_at: t.ticket.created_at,
            id: t.ticket.id,
        })
    } else {
        None
    };
    let mut response = Json(enriched).into_response();
    if let Some(cursor) = next_cursor {
        // Header, not body: the response envelope stays the plain array.
        if let Ok(value) = axum::http::HeaderValue::from_str(&encode_cursor(cursor)) {
            response.headers_mut().insert("x-next-cursor", value);
        }
    }
    Ok(response)
}

/// GET /api/tickets/my-work — everything assigned to me, not closed,
/// across all boards, priority-then-SLA-due ordered (the unified
/// cross-board view from §1; TASKLIST 2.11 adds client/board annotations).
#[utoipa::path(operation_id = "tickets_my_work", get, path = "/api/tickets/my-work", tag = "tickets", security(("bearer" = [])),
    responses((status = 200, body = [MyWorkItem])))]
pub async fn my_work(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<MyWorkItem>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let rows = {
        // RLS-scoped read (pool carries no app.org_id GUC).
        let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
        core_db::set_org_and_user(
            &mut tx,
            authed.principal.user.org_id,
            authed.principal.user.id,
        )
        .await
        .map_err(ApiError::from)?;
        let rows = core_db::tickets::my_work_annotated(
            &mut *tx,
            authed.principal.user.org_id,
            authed.principal.user.id,
            100,
        )
        .await?;
        tx.commit().await.map_err(ApiError::from)?;
        rows
    };
    Ok(Json(rows.into_iter().map(MyWorkItem::from_row).collect()))
}

/// One cross-board my-work queue row (TASKLIST 2.11): the ticket plus the
/// annotations the Today screen renders per the mockup.
#[derive(Debug, Serialize, ToSchema)]
pub struct MyWorkItem {
    pub id: Uuid,
    pub number: String,
    pub title: String,
    pub priority: Priority,
    pub category: TicketStatusCategory,
    pub status_name: String,
    /// The source board of this cross-board row.
    pub board_id: Uuid,
    pub board_name: String,
    /// Client short label for the squared tag (`None` = internal ticket).
    pub client_name: Option<String>,
    /// Manual deadline set on the ticket itself.
    pub due_at: Option<chrono::DateTime<chrono::Utc>>,
    /// SLA resolution deadline when state exists; the UI prefers this over
    /// `due_at` for the right-aligned countdown column.
    pub sla_resolution_due: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl MyWorkItem {
    pub(crate) fn from_row(row: core_db::tickets::MyWorkRow) -> Self {
        let t = row.ticket;
        MyWorkItem {
            id: t.id,
            number: t.number.0,
            title: t.title,
            priority: t.priority,
            category: t.category,
            status_name: row.status_name,
            board_id: t.board_id,
            board_name: row.board_name,
            client_name: row.client_name,
            due_at: t.due_at,
            sla_resolution_due: row.sla_resolution_due,
            updated_at: t.updated_at,
        }
    }
}

/// POST /api/tickets — allocates the human ticket number, initializes SLA
/// clocks, validates + stores any initial custom-field values (REQUIRED
/// definitions enforced), and emits `ticket.created` — all in one transaction.
/// Instant-first (IF1/IF2): routed through the shared idempotency gate and
/// returns the read-your-write `mutation` metadata.
#[utoipa::path(operation_id = "tickets_create", post, path = "/api/tickets", tag = "tickets", security(("bearer" = [])),
    request_body = CreateTicketRequest,
    responses((status = 200, body = TicketMutationResponse),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "missing required custom fields or invalid value")))]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateTicketRequest>,
) -> ApiResult<Json<TicketMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Create),
    )?;
    if req.title.trim().is_empty() {
        return Err(ApiError::Validation("title is required".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "ticket",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        // Board-level RBAC first (deny wins): a viewer-role membership row
        // blocks creation even when the org matrix would allow it.
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            req.board_id,
            BoardOp::CreateTicket,
        )
        .await?;
        let status = core_db::boards::default_status_for_board(&mut *ctx.tx, req.board_id).await?;
        let (prefix, seq) = core_db::orgs::next_ticket_number(&mut *ctx.tx, org_id).await?;
        let number = format!("{prefix}-{seq}");
        let ticket = core_db::tickets::insert_ticket(
            &mut ctx.tx,
            core_db::tickets::NewTicket {
                id: Uuid::now_v7(),
                org_id,
                number: &number,
                board_id: req.board_id,
                status_id: status.id,
                title: req.title.trim(),
                body: req.body.as_deref(),
                priority: req.priority,
                client_id: req.client_id,
                site_id: req.site_id,
                contact_id: req.contact_id,
                assignee_id: req.assignee_id,
                requester_id: None,
                due_at: req.due_at,
            },
        )
        .await?;
        // Fresh entity: allocate the seq the response reports. A create
        // presenting a non-zero version is a client bug — a fresh entity
        // can only ever match version 0 — and is rejected before any row
        // is written.
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "ticket",
                entity_id: ticket.id,
                version: req.instant.version,
            },
        )
        .await?;

        // SLA clocks start now, under the org's default policy.
        if let Ok(policy) = core_db::sla::get_default_policy(&mut *ctx.tx, org_id).await {
            let sla = core_domain::entities::TicketSlaState::init(
                org_id,
                ticket.id,
                &policy,
                ticket.priority,
                ticket.created_at,
            );
            core_db::sla::upsert_ticket_state(&mut *ctx.tx, &sla).await?;
        }

        // Checklist templates instantiate eagerly on creation so staff see
        // required steps from minute one; template edits flow onto existing
        // tickets lazily via `instantiate_missing` (close path / reads).
        core_db::checklists::instantiate_missing(&mut ctx.tx, org_id, req.board_id, ticket.id)
            .await?;

        ctx.emit(
            types::TICKET_CREATED,
            "ticket",
            Some(ticket.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketCreated {
                ticket_id: ticket.id,
                number: ticket.number.0.clone(),
                board_id: ticket.board_id,
                title: ticket.title.clone(),
                priority: ticket.priority,
                client_id: ticket.client_id,
                assignee_id: ticket.assignee_id,
                created_at: ticket.created_at,
            })
            .unwrap_or_default(),
        )
        .await?;

        // Initial custom-field values: validated per definition, stored
        // canonically, then the REQUIRED set is completeness-checked.
        let applied = super::custom_fields::apply_ticket_field_inputs(
            &mut ctx.tx,
            org_id,
            req.board_id,
            ticket.id,
            &req.custom_fields,
        )
        .await?;
        for value in &applied {
            ctx.emit(
                types::TICKET_FIELD_SET,
                "ticket",
                Some(ticket.id.to_string().as_str()),
                &serde_json::json!({
                    "ticket_id": ticket.id,
                    "board_id": req.board_id,
                    "field_id": value.field_id,
                    "value": value.value,
                }),
            )
            .await?;
        }
        super::custom_fields::enforce_required_present(
            &mut ctx.tx,
            org_id,
            req.board_id,
            ticket.id,
        )
        .await?;

        let enriched = TicketWithFields::new(ticket, applied);
        let after = serde_json::to_value(&enriched.ticket).unwrap_or(Value::Null);
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "ticket",
                entity_id: enriched.ticket.id,
                op: "create",
                seq: write_seq,
                before: None,
                after,
                updated_at: enriched.ticket.updated_at,
                key: key.as_deref(),
                payload: &enriched,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TicketMutationResponse {
                ticket: enriched,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// GET /api/tickets/{id}
#[utoipa::path(operation_id = "tickets_get_one", get, path = "/api/tickets/{id}", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = TicketWithFields)))]
pub async fn get_one(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<TicketWithFields>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let org_id = authed.principal.user.org_id;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let ticket = core_db::tickets::get_ticket(&mut *tx, org_id, id).await?;
    let enriched = TicketWithFields::load(&mut *tx, org_id, ticket).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(enriched))
}

/// PATCH /api/tickets/{id} — partial update; validates + upserts any
/// provided custom-field values; emits `ticket.updated`,
/// `ticket.field_set` (per changed field), and `ticket.assigned` when the
/// assignee changes. Instant-first (IF1/IF2): idempotency-keyed, version-
/// gated, and returns prior + new values for undo.
#[utoipa::path(operation_id = "tickets_patch", patch, path = "/api/tickets/{id}", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = PatchTicketRequest,
    responses((status = 200, body = TicketMutationResponse),
              (status = 404, description = "ticket not visible in scope (rls_sparse)"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "invalid custom-field value")))]
pub async fn patch(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchTicketRequest>,
) -> ApiResult<Json<TicketMutationResponse>> {
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
        // Classification validation (Wave 11 lane 3): the node must exist
        // in this org and not be retired; clearing is always legal. A
        // retired node already ON the ticket stays (retirement never
        // rewrites tickets) — it just cannot be newly assigned.
        for (label, node_id) in [("type_id", req.type_id), ("category_id", req.category_id)] {
            if let Some(Some(node_id)) = node_id {
                let node = core_db::taxonomy::get_node(
                    &mut ctx.tx,
                    org_id,
                    if label == "type_id" {
                        core_db::taxonomy::TaxonomyTable::Type
                    } else {
                        core_db::taxonomy::TaxonomyTable::Category
                    },
                    node_id,
                )
                .await
                .map_err(|e| instant::mutation_scope_err(e.into()))?;
                if node.is_retired {
                    return Err(ApiError::Validation(format!(
                        "{label} refers to a retired taxonomy node"
                    )));
                }
            }
        }
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            before.board_id,
            BoardOp::WorkTicket,
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
        core_db::tickets::update_ticket(
            &mut *ctx.tx,
            org_id,
            id,
            core_db::tickets::TicketPatch {
                title: req.title.as_deref(),
                body: req.body.as_deref().map(Some),
                priority: req.priority,
                assignee_id: req.assignee_id,
                client_id: req.client_id,
                site_id: req.site_id,
                contact_id: req.contact_id,
                due_at: req.due_at,
                type_id: req.type_id,
                category_id: req.category_id,
            },
        )
        .await?;
        let after = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await?;

        // Partial custom-field update: only listed fields change; each is
        // validated against its definition before storage. Clearing a
        // REQUIRED value stays blocked at the dedicated clear endpoint.
        if let Some(inputs) = &req.custom_fields {
            let applied = super::custom_fields::apply_ticket_field_inputs(
                &mut ctx.tx,
                org_id,
                after.board_id,
                id,
                inputs,
            )
            .await?;
            for value in &applied {
                ctx.emit(
                    types::TICKET_FIELD_SET,
                    "ticket",
                    Some(after.id.to_string().as_str()),
                    &serde_json::json!({
                        "ticket_id": after.id,
                        "board_id": after.board_id,
                        "field_id": value.field_id,
                        "value": value.value,
                    }),
                )
                .await?;
            }
        }

        ctx.emit(
            types::TICKET_UPDATED,
            "ticket",
            Some(after.id.to_string().as_str()),
            &serde_json::json!({
                "ticket_id": after.id,
                "board_id": after.board_id,
                "assignee_id": after.assignee_id,
            }),
        )
        .await?;
        if before.assignee_id != after.assignee_id {
            ctx.emit(
                types::TICKET_ASSIGNED,
                "ticket",
                Some(after.id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::TicketAssigned {
                    ticket_id: after.id,
                    board_id: after.board_id,
                    assignee_id: after.assignee_id,
                    assigned_by: Some(user_id),
                })
                .unwrap_or_default(),
            )
            .await?;
        }
        let enriched = TicketWithFields::load(&mut *ctx.tx, org_id, after).await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "ticket",
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&enriched.ticket).unwrap_or(Value::Null),
                updated_at: enriched.ticket.updated_at,
                key: key.as_deref(),
                payload: &enriched,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TicketMutationResponse {
                ticket: enriched,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/tickets/{id}/status — the guarded lifecycle transition:
/// domain transition rules, pre-close checklist enforcement (typed 409
/// listing the incomplete required items when the board enforces), SLA
/// pause bookkeeping, and the `ticket.status_changed` event, all in one tx.
/// Instant-first (IF1/IF2): idempotency-keyed, version-gated, read-your-write.
#[utoipa::path(operation_id = "tickets_change_status", post, path = "/api/tickets/{id}/status", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = ChangeStatusRequest,
    responses((status = 200, body = TicketStatusMutationResponse),
              (status = 404, description = "ticket not visible in scope (rls_sparse)"),
              (status = 409, description = "pre-close checklist blocked, version conflict, or idempotency key reused"),
              (status = 422, description = "invalid transition")))]
pub async fn change_status(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<ChangeStatusRequest>,
) -> ApiResult<Json<TicketStatusMutationResponse>> {
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
        // The board's statuses + explicit transition matrix define what is
        // legal here; with no explicit matrix the semantic-category floor
        // governs (core-domain `workflow`).
        let statuses = core_db::boards::list_statuses(&mut *ctx.tx, before.board_id).await?;
        let edges = core_db::boards::list_transitions(&mut *ctx.tx, before.board_id).await?;
        if let Err(rejection) =
            workflow::validate_transition(&statuses, &edges, before.status_id, req.status_id)
        {
            return Err(ApiError::Validation(rejection.message()));
        }
        let target = statuses
            .iter()
            .find(|s| s.id == req.status_id)
            .ok_or(ApiError::NotFound)?
            .clone();

        // Per-column WIP cap (0089): the target column's occupancy
        // (excluding THIS ticket — a move within is self-neutral) is
        // enforced inside the same write transaction as the move, so a
        // concurrent move cannot squeeze past the cap either. Typed 422
        // naming the column + its cap; the board UI renders the count.
        if let Some(wip_limit) = target.wip_limit {
            let occupancy = core_db::boards::count_tickets_in_status(
                &mut *ctx.tx,
                req.status_id,
                id,
            )
            .await?;
            if occupancy >= i64::from(wip_limit) {
                return Err(ApiError::Validation(format!(
                    "status `{}` has reached its WIP limit ({}/{}) — move the ticket elsewhere or raise the cap",
                    target.name, occupancy, wip_limit
                )));
            }
        }

        // Board-level RBAC (deny wins): closing is a distinct board-scoped
        // operation from ordinary work, so the role matrix can forbid
        // closure while still allowing edits.
        let close_op = target.category == TicketStatusCategory::Closed;
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            before.board_id,
            if close_op {
                BoardOp::CloseTicket
            } else {
                BoardOp::WorkTicket
            },
        )
        .await?;

        // Pre-close checklist gate (defaults-first): lazy-instantiate any
        // template items this ticket has not picked up yet — both so older
        // tickets see template edits and so the verdict below judges the
        // complete required set — then apply the pure domain rule. Blocking
        // is a typed 409 that names every unchecked REQUIRED item.
        let instantiated =
            core_db::checklists::instantiate_missing(&mut ctx.tx, org_id, before.board_id, id)
                .await?;
        let checklist = core_db::tickets::list_checklist(&mut *ctx.tx, org_id, id).await?;
        // Only closures are gated; other categories never consult the list.
        if target.category.requires_checklist() {
            let active = checklist_enforcement(&mut ctx.tx, org_id, before.board_id).await?;
            if let Err(blocked) = core_domain::notes::enforce_pre_close(&checklist, active) {
                return Err(ApiError::ChecklistBlocked {
                    missing: blocked.missing,
                });
            }
            // Wave 17 workbar close gate (§4): every task needs a terminal
            // decision (checked / N-A) — independent of the board's
            // require_checklist_close posture; empty checklists pass
            // vacuously. Typed 409 with the exact unassigned rows so raw
            // API callers get the same gate as any closing surface.
            if let Err(gate) = core_domain::notes::enforce_tasks_assigned(&checklist) {
                return Err(ApiError::TasksUnassigned {
                    items: gate
                        .items
                        .iter()
                        .map(|(item_id, text, required)| TaskGateItem {
                            item_id: *item_id,
                            text: text.clone(),
                            required: *required,
                        })
                        .collect(),
                });
            }
        }
        if instantiated > 0 {
            ctx.emit(
                types::TICKET_CHECKLIST_UPDATED,
                "ticket",
                Some(id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::TicketChecklistUpdated {
                    ticket_id: id,
                    board_id: before.board_id,
                    items: checklist,
                    updated_by: None,
                })
                .unwrap_or_default(),
            )
            .await?;
        }

        // SLA pause bookkeeping binds to the semantic category, under the
        // policy the ticket is actually bound to (attached calendar wins
        // over the embedded default document).
        let now = chrono::Utc::now();
        if let Ok(Some(mut sla_state)) = core_db::sla::get_ticket_state(&mut *ctx.tx, id).await {
            let calendar =
                match core_db::sla::get_policy_record(&mut *ctx.tx, org_id, sla_state.policy_id)
                    .await
                {
                    Ok(record) => core_db::sla::resolve_policy_calendar(&mut *ctx.tx, &record)
                        .await
                        .unwrap_or_else(|_| record.calendar.clone()),
                    Err(_) => return Err(ApiError::NotFound),
                };
            sla_state.apply_category(target.category, now, &calendar);
            core_db::sla::upsert_ticket_state(&mut *ctx.tx, &sla_state).await?;
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
        core_db::tickets::update_status(&mut *ctx.tx, org_id, id, req.status_id).await?;
        let after = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await?;

        ctx.emit(
            types::TICKET_STATUS_CHANGED,
            "ticket",
            Some(after.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketStatusChanged {
                ticket_id: after.id,
                board_id: after.board_id,
                assignee_id: after.assignee_id,
                from_status: before.status_id.to_string(),
                to_status: target.name.clone(),
                from_category: before.category,
                to_category: target.category,
                changed_by: Some(user_id),
                occurred_at: now,
            })
            .unwrap_or_default(),
        )
        .await?;
        if target.category == TicketStatusCategory::Closed {
            ctx.emit(
                types::TICKET_CLOSED,
                "ticket",
                Some(after.id.to_string().as_str()),
                &serde_json::json!({
                    "ticket_id": after.id,
                    "board_id": after.board_id,
                    "resolved_at": after.resolved_at
                }),
            )
            .await?;
        }

        // Audit the lifecycle change with full from→to provenance.
        let from_name = statuses
            .iter()
            .find(|s| s.id == before.status_id)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "ticket.status_change".into(),
                entity_type: Some("ticket".into()),
                entity_id: Some(after.id.to_string()),
                detail: serde_json::json!({
                    "from_status": from_name,
                    "from_status_id": before.status_id,
                    "from_category": before.category.as_str(),
                    "to_status": target.name,
                    "to_status_id": target.id,
                    "to_category": target.category.as_str(),
                }),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;

        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "ticket",
                entity_id: id,
                op: "status_change",
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
            MutationOutcome::Applied(TicketStatusMutationResponse {
                ticket: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct CommentListQuery {
    /// Optional taxonomy filter (`internal`, `client_visible`,
    /// `in_house_update`). Omit for all kinds (staff view).
    pub kind: Option<NoteKind>,
}

/// GET /api/tickets/{id}/comments — optional kind filter; internal notes
/// require ticket view. The portal surface must consume the dedicated
/// client-visible read (`core-db::tickets::list_client_visible_comments`),
/// never this staff endpoint.
/// GET /api/tickets/{id}/sla — clock state plus a live business-time
/// remaining estimate for the resolution budget (drives the UI badge).
#[utoipa::path(operation_id = "tickets_sla_status", get, path = "/api/tickets/{id}/sla", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = TicketSlaStatus)))]
pub async fn sla_status(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<TicketSlaStatus>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let org_id = authed.principal.user.org_id;

    let status = org_tx::with_org_tx(
        &state,
        org_id,
        authed.principal.user.id,
        |mut ctx| async move {
            let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await?;
            // None-mode: a ticket created without an applicable SLA policy
            // has no state row — that is a typed empty state, not a 404.
            let Some(state_row) = core_db::sla::get_ticket_state(&mut *ctx.tx, id).await? else {
                return Ok((
                    ctx,
                    TicketSlaStatus {
                        applied: false,
                        state: None,
                        minutes_remaining: None,
                        paused: false,
                        breached: false,
                    },
                ));
            };
            // Attached row wins; embedded document keeps governing otherwise.
            let record = core_db::sla::get_policy_record(&mut *ctx.tx, org_id, state_row.policy_id)
                .await
                .map_err(|_| ApiError::NotFound)?;
            let calendar = core_db::sla::resolve_policy_calendar(&mut *ctx.tx, &record)
                .await
                .unwrap_or_else(|_| record.calendar.clone());
            let now = chrono::Utc::now();
            let position = core_domain::sla_engine::position(
                &calendar,
                ticket.created_at,
                now,
                state_row.resolution_due,
                state_row.paused_business_secs,
                state_row.paused_since,
            );
            let minutes_remaining =
                position.map(|p| ((p.budget_secs - p.consumed_secs) / 60).max(i64::MIN / 2));
            let paused = state_row.paused_since.is_some();
            let breached = state_row.breached;
            Ok((
                ctx,
                TicketSlaStatus {
                    applied: true,
                    state: Some(state_row),
                    minutes_remaining,
                    paused,
                    breached,
                },
            ))
        },
    )
    .await?;
    Ok(Json(status))
}

/// GET /api/tickets/{id}/comments — internal notes require ticket view;
/// the portal surface filters kinds separately (portal v1).
#[utoipa::path(operation_id = "tickets_list_comments", get, path = "/api/tickets/{id}/comments", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path), CommentListQuery),
    responses((status = 200, body = [core_domain::entities::TicketComment])))]
pub async fn list_comments(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Query(q): Query<CommentListQuery>,
) -> ApiResult<Json<Vec<core_domain::entities::TicketComment>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let kinds: Option<Vec<NoteKind>> = q.kind.map(|k| vec![k]);
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let comments = core_db::tickets::list_comments(
        &mut *tx,
        authed.principal.user.org_id,
        id,
        kinds.as_deref(),
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(comments))
}

/// POST /api/tickets/{id}/comments — adds a structured note; @mentions
/// fan out into the notification center. Instant-first (IF1/IF2):
/// idempotency-keyed and returns the read-your-write mutation metadata.
#[utoipa::path(operation_id = "tickets_add_comment", post, path = "/api/tickets/{id}/comments", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = AddCommentRequest,
    responses((status = 200, body = TicketCommentMutationResponse),
              (status = 404, description = "ticket not visible in scope (rls_sparse)"),
              (status = 409, description = "idempotency key reused")))]
pub async fn add_comment(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<AddCommentRequest>,
) -> ApiResult<Json<TicketCommentMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
    if req.body.trim().is_empty() {
        return Err(ApiError::Validation("comment body is required".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let author_name = authed.principal.user.display_name.clone();
    let requester_email = authed.principal.user.email.clone();
    let mail = std::sync::Arc::clone(&state.mail);
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "ticket_comment",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            ticket.board_id,
            BoardOp::WorkTicket,
        )
        .await?;
        let comment = core_db::tickets::insert_comment(
            &mut *ctx.tx,
            core_db::tickets::NewTicketComment {
                id: Uuid::now_v7(),
                org_id,
                ticket_id: id,
                author_id: Some(user_id),
                kind: req.kind,
                body: req.body.trim(),
                workflow_meta: None,
            },
        )
        .await?;

        // IF2 delete-on-submit (§5.2): a committed comment that carries the
        // composer draft's idempotency key clears that draft in THIS
        // transaction — an autosaved draft never survives its own submit.
        if let Some(draft_key) = &req.draft_key {
            let _ = core_db::note_drafts::delete_submitted(
                &mut *ctx.tx,
                org_id,
                id,
                user_id,
                draft_key,
            )
            .await?;
        }

        // Bind composer-uploaded files to this comment (one-shot links;
        // foreign/duplicate ids are skipped rather than failing the note).
        for attachment_id in &req.attachment_ids {
            if let Some(linked) = core_db::attachments::link_to_comment(
                &mut ctx.tx,
                org_id,
                *attachment_id,
                id,
                comment.id,
            )
            .await?
            {
                ctx.emit(
                    types::ATTACHMENT_LINKED,
                    "attachment",
                    Some(linked.id.to_string().as_str()),
                    &serde_json::to_value(core_domain::events::AttachmentLinked {
                        attachment_id: linked.id,
                        ticket_comment_id: comment.id,
                        ticket_id: id,
                    })
                    .unwrap_or_default(),
                )
                .await?;
            }
        }

        // Resolve @mentions against active org members (visibility is the
        // member directory itself until board RBAC lands), ignoring the
        // author's own handle and unknown handles. Notifications are
        // created in THIS transaction and each emits its own
        // `notification.created` so the Redis→hub chain fans out per-user.
        let tokens = core_domain::mentions::parse(req.body.trim());
        let mentioned = if tokens.is_empty() {
            Vec::new()
        } else {
            let directory: Vec<core_domain::mentions::MemberHandles> =
                core_db::users::list_users(&mut *ctx.tx, org_id)
                    .await?
                    .into_iter()
                    .filter(|u| u.is_active)
                    .map(|u| {
                        core_domain::mentions::MemberHandles::from_identity(
                            u.id,
                            &u.email,
                            &u.display_name,
                        )
                    })
                    .collect();
            core_domain::mentions::resolve(&tokens, &directory, user_id)
        };

        for mentioned_user in &mentioned {
            // The in-app channel toggle gates whether the inbox row exists
            // at all (defaults-first chain); email fan-out happens later in
            // the worker against the same preference chain.
            let pref = core_db::notifications::effective_preference(
                &mut *ctx.tx,
                org_id,
                *mentioned_user,
                core_domain::entities::NotificationKind::Mention.as_str(),
            )
            .await?;
            if !pref.in_app {
                continue;
            }
            let notification = core_db::notifications::insert(
                &mut *ctx.tx,
                Uuid::now_v7(),
                org_id,
                *mentioned_user,
                "mention",
                Some(id),
                &format!("{} mentioned you on {}", author_name, ticket.number.0),
                Some(comment_excerpt(req.body.trim())),
            )
            .await?;
            ctx.emit(
                types::NOTIFICATION_CREATED,
                "notification",
                Some(notification.id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::NotificationCreated {
                    user_id: *mentioned_user,
                    kind: notification.kind.as_str().to_string(),
                    ticket_id: Some(id),
                    title: notification.title.clone(),
                })
                .unwrap_or_default(),
            )
            .await?;
        }

        // Email-out v1: client-visible replies notify the requester
        // (skipped when the author IS the requester or the requester has
        // no account email). Enqueue-only here; the worker transports.
        //
        // The composer may have written a greeting and a signature for THIS
        // message. A locked org refuses the signature rather than dropping
        // it: a per-message override is a personal signature that was never
        // stored, so silently ignoring it would send a message the sender
        // did not write. The refusal names the policy, so the composer can
        // say why rather than "rejected".
        let email_overrides = match req.email_envelope.clone() {
            Some(override_fields) => {
                let overrides = crate::email::EnvelopeOverrides {
                    greeting: override_fields.greeting,
                    signature: override_fields.signature,
                };
                if overrides
                    .signature
                    .as_deref()
                    .is_some_and(|text| !text.trim().is_empty())
                {
                    let policy = crate::email::signature_policy(
                        &mut ctx.tx,
                        org_id,
                    )
                    .await?;
                    if !policy.allows_personal() {
                        return Err(ApiError::Validation(
                            "This workspace locks email signatures to its own, so a \
                             per-message signature cannot be sent. The organization's \
                             signature is on its way."
                                .into(),
                        ));
                    }
                }
                overrides
            }
            None => crate::email::EnvelopeOverrides::default(),
        };
        if req.kind == NoteKind::ClientVisible {
            if let Ok(ticket) = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await {
                if let Some(requester_id) = ticket.requester_id {
                    if let Ok(requester) =
                        core_db::users::get_user(&mut *ctx.tx, org_id, requester_id).await
                    {
                        if requester.is_active
                            && !requester.email.eq_ignore_ascii_case(&requester_email)
                        {
                            crate::email::enqueue_outbound(
                                &mut ctx.tx,
                                crate::email::OutboundEnqueue {
                                    mail: &mail,
                                    org_id,
                                    to_addresses: vec![requester.email],
                                    kind: &core_mail::outbound::OutboundKind::CommentNotification {
                                        ticket_number: ticket.number.0.clone(),
                                        ticket_title: ticket.title.clone(),
                                        excerpt: core_mail::outbound::excerpt(req.body.trim(), 280),
                                    },
                                    in_reply_to: None,
                                    references: Vec::new(),
                                    ticket_id: Some(id),
                                    // The AUTHOR is the person the requester is
                                    // corresponding with, so their default (or
                                    // the override they wrote for this note) is
                                    // the signature that goes out — resolved
                                    // here, at send time, from their row.
                                    signer: Some(user_id),
                                    overrides: email_overrides.clone(),
                                },
                            )
                            .await?;
                        }
                    }
                }
            }
        }

        ctx.emit(
            types::TICKET_COMMENT_ADDED,
            "ticket_comment",
            Some(comment.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketCommentAdded {
                comment: comment.clone(),
                mentioned_user_ids: mentioned,
            })
            .unwrap_or_default(),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "ticket_comment",
                entity_id: comment.id,
                version: None,
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "ticket_comment",
                entity_id: comment.id,
                op: "comment_add",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&comment).unwrap_or(Value::Null),
                updated_at: comment.created_at,
                key: key.as_deref(),
                payload: &comment,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TicketCommentMutationResponse { comment, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// First ~140 characters of a note body for notification previews.
fn comment_excerpt(body: &str) -> &str {
    match body.char_indices().nth(140) {
        Some((idx, _)) => body[..idx].trim_end(),
        None => body,
    }
}

/// Unchecked REQUIRED item texts under the given posture (empty when
/// inactive) — the UI-facing projection of the close gate.
fn blocking_of(items: &[ChecklistItem], enforcement_active: bool) -> Vec<String> {
    if !enforcement_active {
        return Vec::new();
    }
    core_domain::notes::enforce_pre_close(items, true)
        .err()
        .map(|b| b.missing)
        .unwrap_or_default()
}

/// Classifies one checklist state transition into the server-composed
/// ledger append kind (docs/notes-tasks-workbar.md §3.3/§3.4): a flip to
/// `checked` composes the Completed note; a flip to `na` composes N-A
/// (reason MANDATORY); reverting any terminal state composes Reopened
/// (reason MANDATORY). `None` = no ledger append.
fn task_transition_kind(
    before: (bool, bool),
    after: (bool, bool),
) -> Option<core_domain::notes::TaskNoteKind> {
    use core_domain::notes::TaskNoteKind;
    match (before, after) {
        ((false, _), (true, _)) => Some(TaskNoteKind::Completed),
        ((false, false), (false, true)) => Some(TaskNoteKind::Na),
        ((true, false) | (false, true), (false, false)) => Some(TaskNoteKind::Reopen),
        _ => None,
    }
}

/// Audited, file-mirrored validation for the mandatory reason (§3.4):
/// non-empty after trim, ≤ 2000 chars, else typed 422.
fn validate_reason(reason: &Option<String>) -> ApiResult<String> {
    match reason.as_ref().map(|r| r.trim()).filter(|r| !r.is_empty()) {
        Some(r) if r.chars().count() <= 2000 => Ok(r.to_string()),
        Some(_) => Err(ApiError::Validation(
            "note_reason must be 2000 characters or fewer".into(),
        )),
        None => Err(ApiError::Validation(
            "note_reason is required to mark an item N-A or reopen it".into(),
        )),
    }
}

/// The server-composed, in-transaction ledger append (§5.1): composes the
/// standardized body through the SHARED `notes::format_note_body` rules,
/// inserts the comment, and emits `ticket.comment_added` in the SAME
/// transaction as the checklist flip — a crash can never leave a flipped
/// task with no ledger line, and the envelope key makes retries
/// duplicate-proof. `workflow_meta` marks machine origin at the row level
/// (never a body-text sentinel). Returns the inserted comment.
#[allow(clippy::too_many_arguments)]
/// One server-composed task-ledger append (Wave 18 lane 2b: the
/// `too_many_arguments` clippy fix — callers name their fields).
struct TaskAppendParams<'a> {
    key: Option<&'a str>,
    item_id: Uuid,
    ticket_id: Uuid,
    org_id: Uuid,
    user_id: Uuid,
    kind: core_domain::notes::TaskNoteKind,
    detail: &'a str,
}

async fn compose_task_append(
    ctx: &mut crate::org_tx::OrgTx<'_>,
    p: TaskAppendParams<'_>,
) -> ApiResult<core_domain::entities::TicketComment> {
    use core_domain::events::TicketCommentAdded;
    use core_domain::status::NoteKind;
    let TaskAppendParams {
        key,
        item_id,
        ticket_id,
        org_id,
        user_id,
        kind,
        detail,
    } = p;
    let body = core_domain::notes::compose_task_note(kind, detail);
    let meta = serde_json::json!({
        "kind": kind.as_str(),
        "auto": true,
        "item_ids": [item_id],
    });
    let note_key = key.map(|k| format!("{k}:{}", kind.key_tag()));
    let comment = core_db::tickets::insert_comment(
        &mut *ctx.tx,
        core_db::tickets::NewTicketComment {
            id: Uuid::now_v7(),
            org_id,
            ticket_id,
            author_id: Some(user_id),
            kind: NoteKind::Internal,
            body: &body,
            workflow_meta: Some(&meta),
        },
    )
    .await?;
    ctx.emit(
        types::TICKET_COMMENT_ADDED,
        "ticket_comment",
        Some(comment.id.to_string().as_str()),
        &serde_json::to_value(TicketCommentAdded {
            comment: comment.clone(),
            mentioned_user_ids: Vec::new(),
        })
        .unwrap_or_default(),
    )
    .await?;
    let note_seq = instant::entity_seq(
        ctx,
        instant::SeqSpec {
            entity: "ticket_comment",
            entity_id: comment.id,
            version: None,
        },
    )
    .await?;
    instant::record(
        ctx,
        instant::RecordSpec {
            entity: "ticket_comment",
            entity_id: comment.id,
            op: "comment_add",
            seq: note_seq,
            before: None,
            after: serde_json::to_value(&comment).unwrap_or(Value::Null),
            updated_at: chrono::Utc::now(),
            key: note_key.as_deref(),
            payload: &comment,
            provenance: Default::default(),
        },
    )
    .await?;
    Ok(comment)
}

/// GET response for a ticket's checklist: instances, derived progress,
/// and the board's live enforcement posture. `blocking` lists the unchecked
/// REQUIRED items when enforcement is active (empty otherwise).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TicketChecklistView {
    pub items: Vec<ChecklistItem>,
    pub progress: ChecklistProgress,
    pub enforcement_active: bool,
    pub blocking: Vec<String>,
}

async fn checklist_view(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    ticket: &core_domain::entities::Ticket,
) -> ApiResult<TicketChecklistView> {
    let (items, progress) = core_db::checklists::view(&mut *conn, org_id, ticket.id).await?;
    let enforcement_active = checklist_enforcement(conn, org_id, ticket.board_id).await?;
    let blocking = blocking_of(&items, enforcement_active);
    Ok(TicketChecklistView {
        items,
        progress,
        enforcement_active,
        blocking,
    })
}

/// GET /api/tickets/{id}/checklist — lazily instantiates any template items
/// the ticket has not picked up yet, then returns the full view.
#[utoipa::path(operation_id = "tickets_get_checklist", get, path = "/api/tickets/{id}/checklist", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = TicketChecklistView)))]
pub async fn get_checklist(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<TicketChecklistView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    core_db::set_org(&mut tx, org_id)
        .await
        .map_err(ApiError::from)?;
    // Instantiation is idempotent; a plain read must never fail because the
    // template changed, so it runs inside its own committed transaction.
    let ticket = core_db::tickets::get_ticket(&mut *tx, org_id, id).await?;
    core_db::checklists::instantiate_missing(&mut tx, org_id, ticket.board_id, id).await?;
    let view = checklist_view(&mut tx, org_id, &ticket).await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(view))
}

/// POST /api/tickets/{id}/checklist — appends an ad-hoc checklist item.
/// Ad-hoc items never block closure; emits `ticket.checklist_updated`.
/// Instant-first (IF1/IF2): idempotency-keyed and returns the
/// read-your-write mutation metadata for the created item.
#[utoipa::path(operation_id = "tickets_add_checklist_item", post, path = "/api/tickets/{id}/checklist", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = AddChecklistItemRequest,
    responses((status = 200, body = TicketChecklistMutationResponse),
              (status = 404, description = "ticket not visible in scope (rls_sparse)"),
              (status = 409, description = "idempotency key reused")))]
pub async fn add_checklist_item(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<AddChecklistItemRequest>,
) -> ApiResult<Json<TicketChecklistMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
    if req.text.trim().is_empty() {
        return Err(ApiError::Validation("checklist text is required".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    // Hoisted so a retried create replays the SAME stored item result.
    let item_id = Uuid::now_v7();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "checklist_item",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            ticket.board_id,
            BoardOp::WorkTicket,
        )
        .await?;
        let existing = core_db::tickets::list_checklist(&mut *ctx.tx, org_id, id).await?;
        core_db::tickets::add_checklist_item(
            &mut *ctx.tx,
            item_id,
            org_id,
            id,
            req.text.trim(),
            existing.len() as i32,
        )
        .await?;
        let (items, progress) = core_db::checklists::view(&mut ctx.tx, org_id, id).await?;
        let created = items
            .iter()
            .find(|item| item.id == item_id)
            .cloned()
            .ok_or_else(|| {
                ApiError::Internal(anyhow::anyhow!("checklist item vanished in-transaction"))
            })?;
        ctx.emit(
            types::TICKET_CHECKLIST_UPDATED,
            "ticket",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketChecklistUpdated {
                ticket_id: id,
                board_id: ticket.board_id,
                items: items.clone(),
                updated_by: Some(user_id),
            })
            .unwrap_or_default(),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "checklist_item",
                entity_id: item_id,
                version: req.instant.version,
            },
        )
        .await?;
        let enforcement_active =
            checklist_enforcement(&mut ctx.tx, org_id, ticket.board_id).await?;
        let blocking = blocking_of(&items, enforcement_active);
        let view = TicketChecklistView {
            items,
            progress,
            enforcement_active,
            blocking,
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "checklist_item",
                entity_id: item_id,
                op: "checklist_item_add",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&created).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TicketChecklistMutationResponse {
                view,
                mutation,
                appended_note: None,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// POST /api/tickets/{id}/checklist/{item_id} — toggles one checklist item;
/// emits `ticket.checklist_updated` in the same transaction. Instant-first
/// (IF1/IF2): idempotency-keyed and returns the read-your-write mutation
/// metadata for the toggled item.
#[utoipa::path(operation_id = "tickets_set_checklist_item", post, path = "/api/tickets/{id}/checklist/{item_id}", tag = "tickets",
    security(("bearer" = [])),
    params(("id" = Uuid, Path), ("item_id" = Uuid, Path)),
    request_body = SetChecklistItemRequest,
    responses((status = 200, body = TicketChecklistMutationResponse),
              (status = 404, description = "ticket or item not visible in scope (rls_sparse)"),
              (status = 409, description = "idempotency key reused")))]
pub async fn set_checklist_item(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, item_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<SetChecklistItemRequest>,
) -> ApiResult<Json<TicketChecklistMutationResponse>> {
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
                entity: "checklist_item",
                entity_id: Some(item_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            ticket.board_id,
            BoardOp::WorkTicket,
        )
        .await?;
        let before = core_db::tickets::list_checklist(&mut *ctx.tx, org_id, id)
            .await?
            .into_iter()
            .find(|item| item.id == item_id)
            .ok_or(ApiError::NotFound)?;
        // Mutual exclusion (Wave 12 lane 6): checked and N-A are distinct
        // server states; asking for both is a client bug, not a preference.
        if req.checked && req.na {
            return Err(ApiError::Validation(
                "a checklist item cannot be both checked and N-A".into(),
            ));
        }
        core_db::tickets::set_checklist_item(
            &mut *ctx.tx,
            org_id,
            item_id,
            req.checked,
            req.na,
            Some(user_id),
        )
        .await?;
        // Server-composed ledger append (§3.3/§3.4): classify the flip and,
        // when it warrants one, compose + insert the note and emit BOTH
        // events in THIS transaction. Reasons for N-A/reopen are validated
        // first (typed 422 — nothing flips without its ledger line).
        let appended =
            match task_transition_kind((before.checked, before.na), (req.checked, req.na)) {
                Some(kind @ core_domain::notes::TaskNoteKind::Completed) => Some(
                    compose_task_append(
                        &mut ctx,
                        TaskAppendParams {
                            key: key.as_deref(),
                            item_id,
                            ticket_id: id,
                            org_id,
                            user_id,
                            kind,
                            detail: &before.text,
                        },
                    )
                    .await?,
                ),
                Some(
                    kind @ (core_domain::notes::TaskNoteKind::Na
                    | core_domain::notes::TaskNoteKind::Reopen),
                ) => {
                    let reason = validate_reason(&req.note_reason)?;
                    Some(
                        compose_task_append(
                            &mut ctx,
                            TaskAppendParams {
                                key: key.as_deref(),
                                item_id,
                                ticket_id: id,
                                org_id,
                                user_id,
                                kind,
                                detail: &reason,
                            },
                        )
                        .await?,
                    )
                }
                None => None,
                Some(core_domain::notes::TaskNoteKind::CloseAll) => None,
            };

        let (items, progress) = core_db::checklists::view(&mut ctx.tx, org_id, id).await?;
        let updated = items
            .iter()
            .find(|item| item.id == item_id)
            .cloned()
            .ok_or_else(|| {
                ApiError::Internal(anyhow::anyhow!("checklist item vanished in-transaction"))
            })?;
        ctx.emit(
            types::TICKET_CHECKLIST_UPDATED,
            "ticket",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketChecklistUpdated {
                ticket_id: id,
                board_id: ticket.board_id,
                items: items.clone(),
                updated_by: Some(user_id),
            })
            .unwrap_or_default(),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "checklist_item",
                entity_id: item_id,
                version: req.instant.version,
            },
        )
        .await?;
        let enforcement_active =
            checklist_enforcement(&mut ctx.tx, org_id, ticket.board_id).await?;
        let blocking = blocking_of(&items, enforcement_active);
        let view = TicketChecklistView {
            items,
            progress,
            enforcement_active,
            blocking,
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "checklist_item",
                entity_id: item_id,
                op: "checklist_item_toggle",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&updated).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TicketChecklistMutationResponse {
                view,
                mutation,
                appended_note: appended,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// GET /api/tickets/{id}/time-entries
#[utoipa::path(operation_id = "tickets_ticket_time_entries", get, path = "/api/tickets/{id}/time-entries", tag = "tickets",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 200, body = [core_domain::entities::TimeEntry])))]
pub async fn ticket_time_entries(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<core_domain::entities::TimeEntry>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let entries = core_db::timers::list_time_entries(
        &mut *tx,
        authed.principal.user.org_id,
        Some(id),
        None,
        None,
        200,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(entries))
}

/// POST /api/tickets/{id}/checklist/complete-all — the close gate's Option
/// A batch (docs/notes-tasks-workbar.md §4.2/§5.3): every listed task flips
/// to `checked` AND lands its server-composed auto-append note, ALL in one
/// transaction with per-item idempotency keys (`complete-all:{ticket}:{item}`-
/// derived under the caller's envelope key). Items already terminal
/// (checked/N-A) are skipped idempotently; an unknown id 404s the whole
/// batch (atomicity). Every appended note carries
/// `workflow_meta { kind: "task_closeall", auto: true, item_ids: [..] }` —
/// the REPORTED auto-close provenance the close-gate dialog displays.
/// Also usable pre-close, not only at the gate.
#[utoipa::path(operation_id = "tickets_complete_all_checklist", post, path = "/api/tickets/{id}/checklist/complete-all", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = CompleteAllRequest,
    responses((status = 200, body = TicketChecklistBatchResponse),
              (status = 404, description = "ticket or a listed item not visible in scope (rls_sparse; whole batch atomic)"),
              (status = 409, description = "idempotency key reused")))]
pub async fn complete_all_checklist(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CompleteAllRequest>,
) -> ApiResult<Json<TicketChecklistBatchResponse>> {
    use core_domain::notes::TaskNoteKind;
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
    if req.task_ids.is_empty() {
        return Err(ApiError::Validation(
            "task_ids must list at least one checklist item".into(),
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
                entity: "checklist_batch",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            ticket.board_id,
            BoardOp::WorkTicket,
        )
        .await?;
        let items = core_db::tickets::list_checklist(&mut *ctx.tx, org_id, id).await?;
        let mut appended_notes = Vec::new();
        let mut per_item_keys = Vec::new();
        for task_id in &req.task_ids {
            let item = items
                .iter()
                .find(|i| &i.id == task_id)
                .ok_or(ApiError::NotFound)?;
            if item.checked || item.na {
                continue; // already has a terminal decision — skip
            }
            core_db::tickets::set_checklist_item(
                &mut *ctx.tx,
                org_id,
                *task_id,
                true,
                false,
                Some(user_id),
            )
            .await?;
            let note = compose_task_append(
                &mut ctx,
                TaskAppendParams {
                    key: key
                        .as_deref()
                        .map(|k| format!("{k}:complete-all:{task_id}"))
                        .as_deref(),
                    item_id: *task_id,
                    ticket_id: id,
                    org_id,
                    user_id,
                    kind: TaskNoteKind::CloseAll,
                    detail: &item.text,
                },
            )
            .await?;
            appended_notes.push(note);
            per_item_keys.push(*task_id);
        }
        let (items_after, progress) = core_db::checklists::view(&mut ctx.tx, org_id, id).await?;
        ctx.emit(
            types::TICKET_CHECKLIST_UPDATED,
            "ticket",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketChecklistUpdated {
                ticket_id: id,
                board_id: ticket.board_id,
                items: items_after.clone(),
                updated_by: Some(user_id),
            })
            .unwrap_or_default(),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "checklist_batch",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let enforcement_active =
            checklist_enforcement(&mut ctx.tx, org_id, ticket.board_id).await?;
        let blocking = blocking_of(&items_after, enforcement_active);
        let view = TicketChecklistView {
            items: items_after,
            progress,
            enforcement_active,
            blocking,
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "checklist_batch",
                entity_id: id,
                op: "checklist_complete_all",
                seq: write_seq,
                before: Some(serde_json::to_value(&items).unwrap_or(Value::Null)),
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
            MutationOutcome::Applied(TicketChecklistBatchResponse {
                view,
                appended_notes,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// GET /api/tickets/{id}/note-draft — restores the caller's autosaved
/// composer draft (IF2, §5.2). Typed absence (200 with `draft: null`),
/// never a 404: an absent draft is a normal first-open state, not a miss.
#[utoipa::path(operation_id = "tickets_get_note_draft", get, path = "/api/tickets/{id}/note-draft", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = NoteDraftResponse)))]
pub async fn get_note_draft(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<NoteDraftResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id)
        .await
        .map_err(ApiError::from)?;
    core_db::note_drafts::purge_stale(&mut *tx).await.ok();
    let row = core_db::note_drafts::get_draft(&mut *tx, org_id, id, user_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(NoteDraftResponse {
        draft: row.map(|r| NoteDraftBody {
            body: r.body,
            idempotency_key: r.idempotency_key,
            updated_at: r.updated_at,
        }),
    }))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct NoteDraftBody {
    pub body: String,
    /// The autosave claim key — the composer must echo it back on submit
    /// so the committed comment clears this exact draft row.
    pub idempotency_key: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct NoteDraftResponse {
    /// The caller's draft (per user × ticket); null when none is saved.
    pub draft: Option<NoteDraftBody>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PutNoteDraftRequest {
    pub body: String,
    /// Autosave claim key (IF1): same key on a debounced retry is a no-op
    /// overwrite (last-writer-wins per §5.2 cadence).
    #[serde(default)]
    pub instant: instant::MutationParams,
}

/// PUT /api/tickets/{id}/note-draft — debounced autosave of the workbar
/// composer draft (NOT a comment; never renders on any ledger). Fresh
/// drafts ride the IF idempotency-keyed write path and opportunistically
/// purge 30-day-stale rows.
#[utoipa::path(operation_id = "tickets_put_note_draft", put, path = "/api/tickets/{id}/note-draft", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = PutNoteDraftRequest,
    responses((status = 200, body = NoteDraftResponse),
              (status = 404, description = "ticket not visible in scope (rls_sparse)")))]
pub async fn put_note_draft(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<PutNoteDraftRequest>,
) -> ApiResult<Json<NoteDraftResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let key = req
        .instant
        .idempotency_key
        .clone()
        .unwrap_or_else(|| format!("draft:{id}:{user_id}:{}", Uuid::now_v7()));
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id)
        .await
        .map_err(ApiError::from)?;
    // RLS makes visibility the guard: an out-of-scope ticket id fails here.
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM tickets WHERE org_id = $1 AND id = $2)",
    )
    .bind(org_id)
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;
    if !exists {
        return Err(ApiError::RlsSparse);
    }
    if req.body.trim().is_empty() {
        core_db::note_drafts::delete(&mut *tx, org_id, id, user_id).await?;
        tx.commit().await.map_err(ApiError::from)?;
        return Ok(Json(NoteDraftResponse { draft: None }));
    }
    let row = core_db::note_drafts::upsert_draft(
        &mut *tx,
        Uuid::now_v7(),
        org_id,
        id,
        user_id,
        req.body.trim(),
        &key,
    )
    .await?;
    // Retention housekeeping rides the autosave path (deterministic,
    // write-side, no cron dependency).
    core_db::note_drafts::purge_stale(&mut *tx).await.ok();
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(NoteDraftResponse {
        draft: Some(NoteDraftBody {
            body: row.body,
            idempotency_key: row.idempotency_key,
            updated_at: row.updated_at,
        }),
    }))
}

/// DELETE /api/tickets/{id}/note-draft — explicit draft discard.
#[utoipa::path(operation_id = "tickets_delete_note_draft", delete, path = "/api/tickets/{id}/note-draft", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "draft cleared (idempotent; 200 even with none)")))]
pub async fn delete_note_draft(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id)
        .await
        .map_err(ApiError::from)?;
    core_db::note_drafts::delete(&mut *tx, org_id, id, user_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({ "cleared": true })))
}

/// POST /api/tickets/{id}/notes/refine — Tier-2 grammar pass (docs/notes-
/// tasks-workbar.md §2.5): sends the draft to the org's configured
/// CompletionProvider (BYOK seam) and returns a PROPOSAL. NEVER a direct
/// write: the ledger is untouched; the composer shows a diff with
/// Apply/Discard and Apply rides the normal submit path. Provider
/// unconfigured/unavailable → typed `status: "unavailable"` + the reason —
/// the Tier-1 Tidy always works and the UI owns the "not available yet"
/// state (Template-button pattern).
#[utoipa::path(operation_id = "tickets_refine_note", post, path = "/api/tickets/{id}/notes/refine", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = RefineNoteRequest,
    responses((status = 200, body = RefineNoteResponse),
              (status = 404, description = "ticket not visible in scope (rls_sparse)")))]
pub async fn refine_note(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<RefineNoteRequest>,
) -> ApiResult<Json<RefineNoteResponse>> {
    use core_domain::completion::{
        CompletionError, CompletionMessage, CompletionRequest, MessageRole,
    };
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
    if req.body.trim().is_empty() {
        return Err(ApiError::Validation("refine needs draft text".into()));
    }
    let org_id = authed.principal.user.org_id;
    // RLS-scoped existence guard so a foreign ticket id reads as rls_sparse.
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id)
        .await
        .map_err(ApiError::from)?;
    let visible = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM tickets WHERE org_id = $1 AND id = $2)",
    )
    .bind(org_id)
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;
    tx.commit().await.map_err(ApiError::from)?;
    if !visible {
        return Err(ApiError::RlsSparse);
    }
    let provider = crate::ai::resolve_org_completion_provider(&state, org_id).await;
    use core_domain::completion::CompletionProvider;
    let prompt = [
        "You are a proofreading assistant for MSP technician notes.",
        "Rewrite the note with corrected grammar, spelling and punctuation.",
        "Preserve every bullet line, the ordered markup (`- `, `1.`), markdown links,",
        "technical terms and acronyms exactly. Never add facts. Reply with ONLY the rewritten note.",
    ]
    .join(" ");
    let creq = CompletionRequest {
        messages: vec![
            CompletionMessage {
                role: MessageRole::System,
                content: prompt,
            },
            CompletionMessage {
                role: MessageRole::User,
                content: req.body.trim().to_string(),
            },
        ],
        model: None,
        temperature: Some(0.1),
        max_tokens: Some(2000),
    };
    match provider.complete(&creq).await {
        Ok(resp) => Ok(Json(RefineNoteResponse {
            status: "proposal".into(),
            proposal: Some(resp.content),
            provider: Some(provider.name().to_string()),
            model: Some(resp.model),
            reason: None,
        })),
        Err(CompletionError::Unavailable { reason }) => Ok(Json(RefineNoteResponse {
            status: "unavailable".into(),
            proposal: None,
            provider: Some(provider.name().to_string()),
            model: None,
            reason: Some(reason),
        })),
        Err(CompletionError::RateLimited { .. })
        | Err(CompletionError::InvalidRequest { .. })
        | Err(CompletionError::Backend { .. }) => {
            tracing::warn!(ticket = %id, provider = provider.name(), "notes/refine degraded; proposal suppressed");
            Ok(Json(RefineNoteResponse {
                status: "unavailable".into(),
                proposal: None,
                provider: Some(provider.name().to_string()),
                model: None,
                reason: Some(
                    "refinement is not available right now — the draft was not changed".into(),
                ),
            }))
        }
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RefineNoteRequest {
    pub body: String,
}

/// Refine outcome. `proposal` is a SUGGESTION — applying it is an explicit
/// user action that posts through the normal comment-create envelope with
/// `workflow_meta.kind = "refined"`; AI provenance rides the approval
/// pipeline (docs/10-agent-platform.md), never a direct ledger write.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RefineNoteResponse {
    /// `"proposal"` = a rewrite is offered for the user's Apply/Discard;
    /// `"unavailable"` = the provider seam is not configured — Tier-1
    /// Tidy remains fully functional.
    pub status: String,
    pub proposal: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub reason: Option<String>,
}
