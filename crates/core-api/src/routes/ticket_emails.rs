//! Per-ticket email read endpoint (Wave 12 lane 6 follow-up).
//!
//! Purpose: list the email rows a ticket's timeline needs — inbound mail
//! routed to the ticket and outbound mail sent about it — through the 2.4
//! email path (`email_messages` ledger + `email_outbound` retry ledger).
//! Each row carries its RFC threading keys plus a resolved
//! `reply_to_message_id` so clients can render reply CHAINS without
//! reimplementing Message-ID matching. This replaces the timeline's typed
//! "not available yet" state: the messages were recorded and transported;
//! nothing listed them per ticket.
//!
//! Non-purpose: composition/replying (staff mail is commented, not
//! emailed, from the API today), attachment payloads (the attachments
//! surface lists those via `email_message_id`), and any write — the read
//! rides the same RLS org-binding as every other staff surface.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use core_domain::entities::EmailMessage;
use core_domain::policy::Permission;
use serde::Serialize;
use std::collections::HashMap;
use utoipa::ToSchema;
use uuid::Uuid;

fn tickets(action: core_domain::policy::Action) -> Permission {
    Permission::Matrix {
        module: core_domain::policy::Module::Tickets,
        action,
    }
}

/// Delivery projection from the `email_outbound` ledger. Present for
/// outbound rows only — inbound mail has no send lifecycle.
#[derive(Debug, Clone, Serialize, PartialEq, ToSchema)]
pub struct EmailDeliveryView {
    /// queued | sending | sent | dead
    pub status: String,
    pub attempts: i32,
    pub max_attempts: i32,
    pub last_error: Option<String>,
}

/// One ledger message as the timeline renders it: the full `EmailMessage`
/// shape (unchanged, additive only) plus the delivery projection and the
/// resolved in-ticket reply parent.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TicketEmailRow {
    #[serde(flatten)]
    pub message: EmailMessage,
    /// Outbound send lifecycle; `null` for inbound rows.
    pub delivery: Option<EmailDeliveryView>,
    /// The id of the message INSIDE THIS TICKET this row replies to,
    /// resolved case-insensitively against the ledger's Message-IDs.
    /// `null` when the parent is unknown or lives on another ticket.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub reply_to_message_id: Option<Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TicketEmailsView {
    /// Newest-first, mirroring the ledger's thread view.
    pub messages: Vec<TicketEmailRow>,
}

/// GET /api/tickets/{id}/emails — every ledger row routed to this ticket
/// (inbound + outbound), newest first, with reply-chain parents resolved
/// and outbound delivery state projected from the retry ledger.
#[utoipa::path(operation_id = "tickets_list_emails", get,
    path = "/api/tickets/{id}/emails", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = TicketEmailsView),
              (status = 404, description = "ticket not visible in scope")))]
pub async fn list_for_ticket(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<TicketEmailsView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let org_id = authed.principal.user.org_id;
    // RLS-scoped read (pool carries no app.org_id GUC) — a ticket outside
    // the caller's org (or board scope) is a clean 404.
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    // The read is ticket-gated first: an unknown/foreign ticket is NotFound
    // even when orphaned ledger rows exist (no rows can exist for a foreign
    // ticket under RLS, but the explicit get keeps the 404 contract typed).
    core_db::tickets::get_ticket(&mut *tx, org_id, id).await?;
    let messages = core_db::emails::list_for_ticket(&mut *tx, org_id, id).await?;
    let delivery = core_db::emails::outbound_delivery_for_messages(
        &mut *tx,
        org_id,
        &messages.iter().map(|m| m.id).collect::<Vec<_>>(),
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;

    // Reply-chain resolution: index THIS ticket's messages by their bare
    // (lowercased) Message-ID; a row's In-Reply-To pointing at one of them
    // becomes a client-resolvable parent link. Keys stay normalized like
    // the inbound pipeline stores them.
    let by_rfc: HashMap<String, Uuid> = messages
        .iter()
        .map(|m| (m.rfc_message_id.to_ascii_lowercase(), m.id))
        .collect();
    let delivery_by_message: HashMap<Uuid, core_db::emails::OutboundDelivery> = delivery
        .into_iter()
        .map(|d| (d.email_message_id, d))
        .collect();
    let rows = messages
        .into_iter()
        .map(|message| {
            let reply_to_message_id = message
                .in_reply_to
                .as_deref()
                .map(|r| {
                    r.trim()
                        .trim_start_matches('<')
                        .trim_end_matches('>')
                        .to_ascii_lowercase()
                })
                .and_then(|r| by_rfc.get(&r))
                .filter(|parent| **parent != message.id)
                .copied();
            let delivery = delivery_by_message
                .get(&message.id)
                .map(|d| EmailDeliveryView {
                    status: d.status.clone(),
                    attempts: d.attempts,
                    max_attempts: d.max_attempts,
                    last_error: d.last_error.clone(),
                });
            TicketEmailRow {
                message,
                delivery,
                reply_to_message_id,
            }
        })
        .collect();
    Ok(Json(TicketEmailsView { messages: rows }))
}
