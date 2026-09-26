//! Email orchestration inside core-api (TASKLIST 2.4).
//!
//! Purpose: build the transport/scanner stack from config, run the inbound
//! webhook pipeline — HMAC-verified payload → malware scan → routing →
//! idempotent ledger gate → thread-or-create → domain events — and shared
//! enqueue helpers for outbound mail. All database writes happen here,
//! inside core-api, preserving single-writer (§4.1); the worker only
//! transports and reports back through internal endpoints.
//!
//! Non-purpose: no retry/backoff math (core-mail owns it), no raw SQL
//! beyond calls into core-db queries, no notification policy decisions
//! beyond the two v1 outbound kinds wired by callers.

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use chrono::Utc;
use core_db::emails::{self, InboundRoute, NewMessage};
use core_domain::entities::{EmailDirection, ScanStatus};
use core_domain::events::{self, types, EmailMessageRecorded, TicketCommentAdded};
use core_domain::settings as registry;
use core_domain::status::{NoteKind, Priority};
use core_mail::inbound::{normalize_message_key, plan_inbound, InboundOutcome};
use serde_json::json;
use sqlx::{PgConnection, Postgres, Transaction};
use std::sync::Arc;
use uuid::Uuid;

pub use core_mail::scan::NoopScan;
use core_mail::{MailTransport, NoopTransport, ScanProvider};

type Tx = Transaction<'static, Postgres>;

/// The configured email edge: one transport + one scanner per process.
pub struct MailStack {
    pub transport: Arc<dyn MailTransport>,
    pub scanner: Arc<dyn ScanProvider>,
    pub from_address: String,
    pub from_name: Option<String>,
}

impl MailStack {
    /// Builds from environment-driven config. Unknown/absent selections
    /// degrade to the documented none modes rather than erroring at boot.
    #[must_use]
    pub fn from_config(config: &crate::config::Config) -> Self {
        let transport: Arc<dyn MailTransport> = match &config.email_transport {
            TransportKind::None => {
                tracing::warn!("email transport=none: outbound mail will stay queued");
                Arc::new(NoopTransport)
            }
            other => core_mail::build_transport(other),
        };
        let scanner: Arc<dyn ScanProvider> = match config.scan_choice() {
            crate::config::ScanChoice::None => Arc::new(NoopScan::new()),
            crate::config::ScanChoice::Clamd(addr) => Arc::new(core_mail::ClamdScan::new(addr)),
        };
        Self {
            transport,
            scanner,
            from_address: config.email_from_address.clone(),
            from_name: config.email_from_name.clone(),
        }
    }
}

use core_mail::TransportKind;

/// The resolved From/Reply-To/signature identity for one outbound message:
/// org settings overlaid by the ticket's board identity when there is one,
/// with the process env defaults as the last fallback.
pub struct MailIdentity {
    pub from_address: String,
    pub from_name: Option<String>,
    pub reply_to: Option<String>,
    pub signature: Option<String>,
    /// Transport label recorded on the ledger row (informational; the
    /// worker resolves the actual adapter per org at send time).
    pub transport_label: String,
}

/// Brand values for the notification/digest wrapper header.
pub struct BrandValues {
    pub brand_name: String,
    pub logo_url: Option<String>,
}

/// One stored org setting as trimmed text; `None` when unset or blank.
async fn stored_text(
    conn: &mut PgConnection,
    org_id: Uuid,
    key: &str,
) -> ApiResult<Option<String>> {
    let value = core_db::settings::get_org_value(&mut *conn, org_id, key).await?;
    Ok(value
        .and_then(|v| v.as_str().map(str::to_string))
        .filter(|s| !s.trim().is_empty()))
}

/// Org-level mail identity (settings over the process defaults).
pub async fn resolve_org_identity(
    conn: &mut PgConnection,
    org_id: Uuid,
    fallback_from: &str,
    fallback_name: Option<&str>,
) -> ApiResult<MailIdentity> {
    let transport = stored_text(conn, org_id, registry::EMAIL_TRANSPORT)
        .await?
        .unwrap_or_else(|| "none".to_string());
    Ok(MailIdentity {
        from_address: stored_text(conn, org_id, registry::EMAIL_FROM_ADDRESS)
            .await?
            .unwrap_or_else(|| fallback_from.to_string()),
        from_name: stored_text(conn, org_id, registry::EMAIL_FROM_NAME)
            .await?
            .or_else(|| fallback_name.map(str::to_string)),
        reply_to: stored_text(conn, org_id, registry::EMAIL_REPLY_TO).await?,
        signature: stored_text(conn, org_id, registry::EMAIL_SIGNATURE).await?,
        transport_label: transport,
    })
}

/// Full identity for one outbound message. Ticket-linked mail takes the
/// ticket's board identity field-by-field over the org identity; a missing
/// ticket silently keeps the org identity (a send must not fail because its
/// ticket vanished mid-flight).
pub async fn resolve_mail_identity(
    conn: &mut PgConnection,
    org_id: Uuid,
    ticket_id: Option<Uuid>,
    mail: &MailStack,
) -> ApiResult<MailIdentity> {
    let mut identity =
        resolve_org_identity(conn, org_id, &mail.from_address, mail.from_name.as_deref()).await?;
    if identity.transport_label == "none" {
        // No org transport selected: record what the process would use.
        identity.transport_label = mail.transport.name().to_string();
    }
    if let Some(ticket_id) = ticket_id {
        if let Ok(ticket) = core_db::tickets::get_ticket(&mut *conn, org_id, ticket_id).await {
            let board = core_db::emails::get_board_identity(&mut *conn, org_id, ticket.board_id)
                .await?;
            if let Some(value) = board.from_address {
                identity.from_address = value;
            }
            if let Some(value) = board.from_name {
                identity.from_name = Some(value);
            }
            if let Some(value) = board.reply_to {
                identity.reply_to = Some(value);
            }
            if let Some(value) = board.signature {
                identity.signature = Some(value);
            }
        }
    }
    Ok(identity)
}

/// Brand header values: the configured wordmark/logo, falling back to the
/// organization name. Logo URLs must be https (mail clients refuse the
/// rest, and a plain-http logo would be a content leak vector).
pub async fn resolve_brand(conn: &mut PgConnection, org_id: Uuid) -> ApiResult<BrandValues> {
    let brand_name = match stored_text(conn, org_id, registry::EMAIL_BRAND_NAME).await? {
        Some(name) => name,
        None => core_db::orgs::get_org(&mut *conn, org_id)
            .await
            .map(|org| org.name)
            .unwrap_or_default(),
    };
    let logo_url = stored_text(conn, org_id, registry::EMAIL_LOGO_URL)
        .await?
        .filter(|url| url.starts_with("https://"));
    Ok(BrandValues {
        brand_name,
        logo_url,
    })
}

/// Outcome of processing one webhook POST — terminal statuses surfaced to
/// the provider so it can stop retrying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundStatus {
    Commented,
    TicketCreated,
    Quarantined,
    Duplicate,
    Unroutable,
}

impl InboundStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            InboundStatus::Commented => "comment",
            InboundStatus::TicketCreated => "ticket_created",
            InboundStatus::Quarantined => "quarantined",
            InboundStatus::Duplicate => "duplicate",
            InboundStatus::Unroutable => "unrouted",
        }
    }
}

/// Runs the full inbound pipeline for an HMAC-verified payload.
///
/// Order matters: the malware scan happens BEFORE persistence (§2 email
/// row) and its failures fail CLOSED — an unreachable scanner quarantines
/// rather than admits. The ledger row itself is inserted FIRST as the
/// replay gate (unique on org + Message-ID); only when THIS call wins the
/// insert do comments/tickets get created, then the row is linked. A race
/// loser sees `duplicate` and touches nothing else.
///
/// The transaction runs as the table owner: recipient→org routing spans
/// orgs by definition and there is no user session. Same narrow exception
/// as the outbound claim endpoint.
pub async fn process_inbound(
    state: &AppState,
    msg: core_mail::InboundEmail,
) -> ApiResult<InboundStatus> {
    msg.validate()
        .map_err(|e| ApiError::Validation(e.to_string()))?;

    // --- attachment preparation (TASKLIST 2.14): decode + size-cap +
    // hash BEFORE the scan gate so nothing undecoded/oversized ever
    // reaches storage, and so the scanner sees attachment bytes too.
    // Email attachments skip the API's magic-byte allowlist ON PURPOSE:
    // after the malware gate this is a trusted channel and rejecting,
    // say, .eml forwards would silently drop ticket content.
    let max_bytes = state.config.max_upload_bytes;
    let mut prepared_attachments: Vec<PreparedAttachment> =
        Vec::with_capacity(msg.attachments.len());
    for part in &msg.attachments {
        let bytes = part.decoded().map_err(|e| {
            ApiError::Validation(format!("attachment {:?}: bad base64: {e}", part.filename))
        })?;
        if bytes.is_empty() {
            return Err(ApiError::Validation(format!(
                "attachment {:?} is empty",
                part.filename
            )));
        }
        if bytes.len() as i64 > max_bytes {
            return Err(ApiError::Validation(format!(
                "attachment {:?} exceeds size cap",
                part.filename
            )));
        }
        use sha2::{Digest, Sha256};
        let sha256_hex = hex::encode(Sha256::digest(&bytes));
        let filename = core_domain::attachments::sanitize_filename(&part.filename);
        prepared_attachments.push(PreparedAttachment {
            filename,
            content_type: if part.content_type.trim().is_empty() {
                "application/octet-stream".to_string()
            } else {
                part.content_type
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase()
            },
            sha256_hex,
            bytes,
        });
    }

    // --- scan (network IO deliberately outside the DB transaction); the
    // scanner receives body text PLUS attachment contents so a payload
    // hidden in an attachment still trips the verdict ---
    let body_text = msg.effective_text();
    let mut scan_text = body_text.clone().unwrap_or_default();
    for part in &prepared_attachments {
        scan_text.push('\n');
        scan_text.push_str(&String::from_utf8_lossy(&part.bytes));
    }
    let (scan_status, proceed, scan_detail) = match state.mail.scanner.scan(&scan_text).await {
        Ok(v) if v.infected => (ScanStatus::Infected, false, v.detail),
        Ok(_) => (
            if state.mail.scanner.name() == "none" {
                ScanStatus::Skipped
            } else {
                ScanStatus::Clean
            },
            true,
            None,
        ),
        Err(e) => {
            tracing::warn!(error = %e, "scanner unavailable; quarantining message");
            (
                ScanStatus::Infected,
                false,
                Some(format!("scanner unavailable: {e}")),
            )
        }
    };

    let rfc_id = normalize_message_key(&msg.rfc_message_id);
    let refs: Vec<String> = msg.thread_keys();
    let recipients = msg.recipients_in_priority_order();

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;

    // --- routing (board address beats org catch-all); SECURITY DEFINER
    // seam because no org scope exists yet ---
    let Some(InboundRoute { org_id, board_id }) =
        emails::resolve_inbound_route_for_app(&mut tx, &recipients).await?
    else {
        return Ok(InboundStatus::Unroutable);
    };
    core_db::set_org(&mut tx, org_id).await?;

    // --- REPLAY GATE: claim the Message-ID before any side effect ---
    let message_id = Uuid::now_v7();
    let inserted = emails::insert_message(
        &mut tx,
        &NewMessage {
            id: message_id,
            org_id,
            direction: EmailDirection::Inbound,
            rfc_message_id: rfc_id.clone(),
            in_reply_to: msg.in_reply_to.as_deref().map(normalize_message_key),
            email_refs: refs.clone(),
            subject: msg.subject.clone(),
            from_address: msg.from_address.trim().to_ascii_lowercase(),
            from_name: msg.from_name.clone(),
            to_addresses: recipients,
            body_text: body_text.clone(),
            ticket_id: None,
            comment_id: None,
            scan_status,
            transport: format!("webhook/{}", state.mail.scanner.name()),
        },
    )
    .await?;
    if inserted.is_none() {
        return Ok(InboundStatus::Duplicate);
    }

    // --- quarantine notice (Wave 36 lane 3): a quarantined message notifies
    // org owners/admins over the EXISTING notification chain (durable in-app
    // row + `notification.created` → worker → email leg). Inside the
    // claim-winning transaction and after the replay gate, so a replayed
    // message never re-notifies and a failed commit notifies nobody.
    if !proceed {
        notify_quarantine(&mut tx, org_id, &msg, scan_detail.as_deref()).await?;
    }

    // --- attachments: store objects + rows INSIDE the claim-winning
    // transaction. Claiming first prevents replay-race orphans; the tx is
    // deliberately held across object PUTs (bounded by the per-part cap)
    // and every stored key is tracked so the error path below can reclaim
    // exactly what it wrote. Rows point at the LEDGER message; ticket/
    // comment linkage is copied onto them once routing decides below.
    let mut stored_keys: Vec<String> = Vec::new();
    let attachment_result: ApiResult<()> = async {
        for part in &prepared_attachments {
            if !proceed {
                break;
            }
            let key = core_store::attachment_key(org_id, &part.sha256_hex);
            let existing =
                core_db::attachments::find_existing_object_key(&mut tx, org_id, &part.sha256_hex)
                    .await?;
            if existing.is_none() {
                state
                    .store
                    .put(
                        &key,
                        &part.content_type,
                        bytes::Bytes::from(part.bytes.clone()),
                    )
                    .await
                    .map_err(|e| {
                        tracing::error!(error = %e, "attachment store failed");
                        ApiError::Internal(anyhow::anyhow!(e.to_string()))
                    })?;
                stored_keys.push(key.clone());
            }
            let kind = core_domain::entities::AttachmentKind::EmailMessage;
            match core_db::attachments::insert(
                &mut tx,
                core_db::attachments::NewAttachment {
                    id: Uuid::now_v7(),
                    org_id,
                    kind,
                    ticket_id: None,
                    ticket_comment_id: None,
                    email_message_id: Some(message_id),
                    filename: &part.filename,
                    content_type: &part.content_type,
                    size_bytes: part.bytes.len() as i64,
                    sha256: &part.sha256_hex,
                    storage_key: &key,
                    uploaded_by_user_id: None,
                    uploaded_by_portal_user_id: None,
                },
            )
            .await
            {
                Ok(attachment) => {
                    emit_event(
                        &mut tx,
                        org_id,
                        types::ATTACHMENT_ADDED,
                        "attachment",
                        Some(attachment.id),
                        &serde_json::to_value(core_domain::events::AttachmentAdded {
                            attachment_id: attachment.id,
                            kind: kind.as_str().to_string(),
                            ticket_id: None,
                            ticket_comment_id: None,
                            email_message_id: Some(message_id),
                            filename: attachment.filename.clone(),
                            content_type: attachment.content_type.clone(),
                            size_bytes: attachment.size_bytes,
                            sha256: attachment.sha256.clone(),
                            uploaded_by_user_id: None,
                        })
                        .unwrap_or_default(),
                    )
                    .await?;
                }
                Err(e) => {
                    // Fatal to THIS stage: a swallowed error would leave
                    // the transaction aborted for every later statement.
                    return Err(ApiError::from(e));
                }
            }
        }
        Ok(())
    }
    .await;
    if let Err(e) = attachment_result {
        reclaim(&state.store, &stored_keys).await;
        return Err(e);
    }

    // --- decide + act ---
    let (ticket_id, comment_id, status) = if !proceed {
        (None, None, InboundStatus::Quarantined)
    } else {
        let thread_hit = emails::find_thread_target(&mut *tx, org_id, &refs).await?;
        let token_hit = if thread_hit.is_none() {
            let mut hit = None;
            for token in msg
                .subject
                .as_deref()
                .map(core_mail::inbound::extract_subject_tokens)
                .unwrap_or_default()
            {
                hit = emails::find_ticket_id_by_number_ci(&mut *tx, org_id, &token).await?;
                if hit.is_some() {
                    break;
                }
            }
            hit
        } else {
            None
        };
        match plan_inbound(thread_hit, token_hit) {
            InboundOutcome::CommentOn { ticket_id }
            | InboundOutcome::AttachByToken { ticket_id } => {
                let comment = core_db::tickets::insert_comment(
                    &mut *tx,
                    core_db::tickets::NewTicketComment {
                        id: Uuid::now_v7(),
                        org_id,
                        ticket_id,
                        author_id: None,
                        kind: NoteKind::ClientVisible,
                        body: body_text.as_deref().unwrap_or("(empty body)"),
                        workflow_meta: None,
                    },
                )
                .await?;
                emit_event(
                    &mut tx,
                    org_id,
                    types::TICKET_COMMENT_ADDED,
                    "ticket_comment",
                    Some(comment.id),
                    &serde_json::to_value(TicketCommentAdded {
                        comment: comment.clone(),
                        mentioned_user_ids: Vec::new(),
                    })
                    .unwrap_or_default(),
                )
                .await?;
                (Some(ticket_id), Some(comment.id), InboundStatus::Commented)
            }
            InboundOutcome::NewTicket => {
                create_ticket_from_email(
                    &mut tx,
                    state.mail.as_ref(),
                    org_id,
                    board_id,
                    &msg,
                    body_text.as_deref(),
                )
                .await?
            }
        }
    };

    // --- link the routing result onto the claimed ledger row; copy the
    // same targets onto email-message attachment rows so authz and
    // portal visibility flow through the ticket like any other file ---
    emails::link_message_targets(&mut *tx, org_id, message_id, ticket_id, comment_id).await?;
    // The ledger row is the single source of truth for routing targets:
    // email attachments stay linked via email_message_id and authz plus
    // portal visibility JOIN through it (routes/attachments +
    // core_db::attachments list_portal_visible/find_portal_visible).
    emit_event(
        &mut tx,
        org_id,
        types::EMAIL_MESSAGE_RECORDED,
        "email_message",
        Some(message_id),
        &serde_json::to_value(EmailMessageRecorded {
            message_id,
            rfc_message_id: rfc_id,
            direction: "inbound".into(),
            outcome: status.as_str().into(),
            ticket_id,
            comment_id,
        })
        .unwrap_or_default(),
    )
    .await?;

    if let Err(e) = tx.commit().await {
        reclaim(&state.store, &stored_keys).await;
        return Err(ApiError::from(e));
    }
    Ok(status)
}

/// One decoded, size-capped, hashed inbound attachment ready to persist.
struct PreparedAttachment {
    filename: String,
    content_type: String,
    sha256_hex: String,
    bytes: Vec<u8>,
}

/// Best-effort object reclamation after a failed pipeline stage. Delete
/// is idempotent per trait contract; failures are logged, never fatal.
async fn reclaim(store: &Arc<dyn core_store::ObjectStore>, keys: &[String]) {
    for key in keys {
        if let Err(e) = store.delete(key).await {
            tracing::warn!(key = %key, error = %e, "orphaned attachment bytes");
        }
    }
}

/// Char-safe clip for notification copy — never splits a codepoint.
fn clip(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Fans one quarantined inbound message out to the org's owners/admins:
/// one durable in-app row plus `notification.created` per recipient, so the
/// worker's existing notifier chain carries the email leg. Recipients with
/// the channel toggled off are skipped before any row exists (defaults-first
/// preference chain, same rule as @mention notifications).
async fn notify_quarantine(
    tx: &mut Tx,
    org_id: Uuid,
    msg: &core_mail::InboundEmail,
    detail: Option<&str>,
) -> ApiResult<()> {
    let recipients = core_db::notifications::scan_detection_recipients(&mut **tx, org_id).await?;
    if recipients.is_empty() {
        return Ok(());
    }
    let subject = msg.subject.as_deref().unwrap_or("(no subject)");
    let title = "Malware scanner quarantined an inbound message";
    let body = format!(
        "From {} — {} ({})",
        clip(&msg.from_address, 120),
        clip(subject, 160),
        clip(detail.unwrap_or("no scanner detail recorded"), 160),
    );
    let kind = core_domain::entities::NotificationKind::ScanDetection;
    for user_id in recipients {
        let pref = core_db::notifications::effective_preference(
            &mut **tx,
            org_id,
            user_id,
            kind.as_str(),
        )
        .await?;
        if !pref.in_app {
            continue;
        }
        let notification = core_db::notifications::insert(
            &mut **tx,
            Uuid::now_v7(),
            org_id,
            user_id,
            kind.as_str(),
            None,
            title,
            Some(&body),
        )
        .await?;
        emit_event(
            tx,
            org_id,
            types::NOTIFICATION_CREATED,
            "notification",
            Some(notification.id),
            &serde_json::to_value(core_domain::events::NotificationCreated {
                user_id,
                kind: notification.kind.as_str().to_string(),
                ticket_id: None,
                title: notification.title.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
    }
    Ok(())
}

/// Creates the autoresponder ticket for brand-new senders: requester
/// resolution (org user → client contact), entry status, SLA init,
/// `ticket.created`, and the v1 confirmation enqueue.
async fn create_ticket_from_email(
    tx: &mut Tx,
    mail: &MailStack,
    org_id: Uuid,
    board_id: Uuid,
    msg: &core_mail::InboundEmail,
    body_text: Option<&str>,
) -> ApiResult<(Option<Uuid>, Option<Uuid>, InboundStatus)> {
    let mut requester_id = None;
    let mut client_id = None;
    let mut contact_id = None;
    if let Some(user) =
        core_db::users::find_by_email_in_org(&mut **tx, org_id, &msg.from_address).await?
    {
        requester_id = Some(user.id);
    } else if let Some(contact) =
        core_db::clients::find_contact_by_email(&mut **tx, org_id, &msg.from_address).await?
    {
        client_id = Some(contact.client_id);
        contact_id = Some(contact.id);
    }

    let status = core_db::boards::default_status_for_board(&mut **tx, board_id).await?;
    let (prefix, seq) = core_db::orgs::next_ticket_number(&mut **tx, org_id).await?;
    let number = format!("{prefix}-{seq}");
    let title = msg
        .subject
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("Email from {}", msg.from_address));
    let ticket = core_db::tickets::insert_ticket(
        tx,
        core_db::tickets::NewTicket {
            id: Uuid::now_v7(),
            org_id,
            number: &number,
            board_id,
            status_id: status.id,
            title: title.trim(),
            body: body_text,
            priority: Priority::P3,
            client_id,
            site_id: None,
            contact_id,
            assignee_id: None,
            requester_id,
            due_at: None,
        },
    )
    .await?;

    if let Ok(policy) = core_db::sla::get_default_policy(&mut **tx, org_id).await {
        let sla = core_domain::entities::TicketSlaState::init(
            org_id,
            ticket.id,
            &policy,
            ticket.priority,
            ticket.created_at,
        );
        core_db::sla::upsert_ticket_state(&mut **tx, &sla).await?;
    }

    emit_event(
        tx,
        org_id,
        types::TICKET_CREATED,
        "ticket",
        Some(ticket.id),
        &serde_json::to_value(events::TicketCreated {
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

    // v1 autoresponder: confirm receipt, threaded to the original message.
    enqueue_outbound(
        tx,
        OutboundEnqueue {
            mail,
            org_id,
            to_addresses: std::slice::from_ref(&msg.from_address).to_vec(),
            kind: &core_mail::outbound::OutboundKind::TicketCreatedConfirmation {
                ticket_number: ticket.number.0.clone(),
                ticket_title: ticket.title.clone(),
            },
            in_reply_to: Some(msg.rfc_message_id.clone()),
            references: vec![msg.rfc_message_id.clone()],
            ticket_id: Some(ticket.id),
        },
    )
    .await?;

    Ok((Some(ticket.id), None, InboundStatus::TicketCreated))
}

/// Everything `enqueue_outbound` needs beyond the transaction.
pub struct OutboundEnqueue<'a> {
    pub mail: &'a MailStack,
    pub org_id: Uuid,
    pub to_addresses: Vec<String>,
    pub kind: &'a core_mail::outbound::OutboundKind,
    /// RFC id this mail answers (normalized before storage).
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub ticket_id: Option<Uuid>,
}

/// Shared outbound enqueue: resolves the from/reply-to identity (org, then
/// the ticket's board when there is one), appends the effective signature,
/// renders the kind, writes the ledger + retry rows, and emits
/// `email.message_recorded` (outcome `queued`). Caller owns the transaction.
pub async fn enqueue_outbound(tx: &mut Tx, req: OutboundEnqueue<'_>) -> ApiResult<()> {
    if req.to_addresses.is_empty() {
        return Ok(());
    }
    let identity = resolve_mail_identity(tx, req.org_id, req.ticket_id, req.mail).await?;
    let mut body = req.kind.body();
    if let Some(signature) = identity.signature.as_deref() {
        if !signature.trim().is_empty() {
            body = format!("{}\n\n{}", body.trim_end(), signature.trim());
        }
    }
    let message_id = emails::enqueue_outbound(
        tx,
        emails::NewOutbound {
            org_id: req.org_id,
            rfc_message_id: emails::new_outbound_message_id(),
            in_reply_to: req.in_reply_to.map(|r| normalize_message_key(&r)),
            references: req
                .references
                .iter()
                .map(|r| normalize_message_key(r))
                .collect(),
            subject: req.kind.subject(),
            from_address: identity.from_address.clone(),
            from_name: identity.from_name.clone(),
            to_addresses: req.to_addresses.clone(),
            body_text: body,
            ticket_id: req.ticket_id,
            transport: identity.transport_label,
        },
    )
    .await?;
    emit_event(
        tx,
        req.org_id,
        types::EMAIL_MESSAGE_RECORDED,
        "email_message",
        Some(message_id),
        &json!({
            "message_id": message_id,
            "direction": "outbound",
            "outcome": "queued",
            "ticket_id": req.ticket_id,
            "comment_id": null,
        }),
    )
    .await?;
    Ok(())
}

/// Enqueues one caller-rendered message (admin probe, digest, notification
/// fan-out) with the org/board identity applied. Returns the ledger message
/// id so callers can point at the delivery ledger.
#[allow(clippy::too_many_arguments)]
pub async fn enqueue_rendered_outbound(
    tx: &mut Tx,
    mail: &MailStack,
    org_id: Uuid,
    to: &str,
    subject: &str,
    body: &str,
    ticket_id: Option<Uuid>,
) -> ApiResult<Uuid> {
    let identity = resolve_mail_identity(tx, org_id, ticket_id, mail).await?;
    let message_id = emails::enqueue_outbound(
        tx,
        emails::NewOutbound {
            org_id,
            rfc_message_id: emails::new_outbound_message_id(),
            in_reply_to: None,
            references: vec![],
            subject: subject.to_string(),
            from_address: identity.from_address.clone(),
            from_name: identity.from_name.clone(),
            to_addresses: vec![to.to_string()],
            body_text: body.to_string(),
            ticket_id,
            transport: identity.transport_label,
        },
    )
    .await?;
    emit_event(
        tx,
        org_id,
        types::EMAIL_MESSAGE_RECORDED,
        "email_message",
        Some(message_id),
        &json!({
            "message_id": message_id,
            "direction": "outbound",
            "outcome": "queued",
            "ticket_id": ticket_id,
            "comment_id": null,
        }),
    )
    .await?;
    Ok(message_id)
}

/// Outbox insert for non-OrgTx contexts (the webhook pipeline). OrgTx's
/// `emit` remains the request-path helper.
pub async fn emit_event(
    tx: &mut Tx,
    org_id: Uuid,
    event_type: &str,
    entity: &str,
    entity_id: Option<Uuid>,
    payload: &serde_json::Value,
) -> ApiResult<()> {
    core_db::outbox::insert(
        tx,
        Uuid::now_v7(),
        org_id,
        event_type,
        entity,
        entity_id.as_ref().map(Uuid::to_string).as_deref(),
        payload,
    )
    .await?;
    Ok(())
}

/// JSON response body for the webhook endpoint.
#[must_use]
pub fn inbound_response(status: InboundStatus) -> serde_json::Value {
    json!({ "status": status.as_str(), "received_at": Utc::now() })
}
