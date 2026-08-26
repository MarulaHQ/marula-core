//! Email message ledger and outbound send-queue queries (TASKLIST 2.4).
//!
//! Purpose: persistence for `email_messages` (idempotent inbound ledger +
//! outbound content), the `email_outbound` retry queue with atomic
//! claiming, inbound routing resolution (board address → org catch-all),
//! and the admin settings upserts. The claim query is the one statement
//! that must run as the TABLE OWNER — callers issue `SET LOCAL ROLE
//! meridian` first (see `claim_due_outbound`).
//!
//! Non-purpose: no transport calls, scan verdicts, or routing policy —
//! core-mail defines traits/math, core-api orchestrates; this module only
//! stores and retrieves.

use crate::{DbError, DbResult};
use core_domain::entities::{EmailDirection, EmailMessage, ScanStatus};
use sqlx::PgConnection;
use uuid::Uuid;

/// Escalates this transaction to the TABLE OWNER role (`meridian`).
///
/// Required ONLY by cross-org system paths — the outbound claim/report
/// endpoints and inbound webhook routing — whose queries span all orgs and
/// therefore cannot pass RLS as the constrained `meridian_app` login.
/// Membership is granted by migration 0014; `SET LOCAL` scopes the switch
/// to the current transaction, auto-reverting on commit/rollback. User
/// request paths must never call this.
pub async fn assume_table_owner(conn: &mut PgConnection) -> DbResult<()> {
    sqlx::query("SET LOCAL ROLE meridian").execute(conn).await?;
    Ok(())
}

/// Domain suffix for Meridian-generated outbound Message-IDs. Stored bare
/// (`uuid@meridian.local`); transport adapters add RFC angle brackets.
pub const OUTBOUND_ID_SUFFIX: &str = "meridian.local";

/// Generates a fresh bare outbound Message-ID.
#[must_use]
pub fn new_outbound_message_id() -> String {
    format!("{}@{OUTBOUND_ID_SUFFIX}", Uuid::now_v7())
}

#[allow(clippy::too_many_arguments)]
fn row_to_message(row: sqlx::postgres::PgRow) -> Result<EmailMessage, sqlx::Error> {
    use sqlx::Row;
    Ok(EmailMessage {
        id: row.get("id"),
        org_id: row.get("org_id"),
        direction: row.get::<String, _>("direction").parse().map_err(
            |e: core_domain::DomainError| sqlx::Error::ColumnDecode {
                index: "direction".into(),
                source: Box::new(e),
            },
        )?,
        rfc_message_id: row.get("rfc_message_id"),
        in_reply_to: row.get("in_reply_to"),
        email_refs: row.get("email_refs"),
        subject: row.get("subject"),
        from_address: row.get("from_address"),
        from_name: row.get("from_name"),
        to_addresses: row.get("to_addresses"),
        body_text: row.get("body_text"),
        ticket_id: row.get("ticket_id"),
        comment_id: row.get("comment_id"),
        scan_status: row.get::<String, _>("scan_status").parse().map_err(
            |e: core_domain::DomainError| sqlx::Error::ColumnDecode {
                index: "scan_status".into(),
                source: Box::new(e),
            },
        )?,
        transport: row.get("transport"),
        raw_ref: row.get("raw_ref"),
        created_at: row.get("created_at"),
    })
}

const MESSAGE_COLS: &str = "id, org_id, direction, rfc_message_id, in_reply_to, email_refs, \
     subject, from_address, from_name, to_addresses, body_text, ticket_id, comment_id, \
     scan_status, transport, raw_ref, created_at";

/// Everything needed to persist a message in either direction.
pub struct NewMessage {
    pub id: Uuid,
    pub org_id: Uuid,
    pub direction: EmailDirection,
    pub rfc_message_id: String,
    pub in_reply_to: Option<String>,
    pub email_refs: Vec<String>,
    pub subject: Option<String>,
    pub from_address: String,
    pub from_name: Option<String>,
    pub to_addresses: Vec<String>,
    pub body_text: Option<String>,
    pub ticket_id: Option<Uuid>,
    pub comment_id: Option<Uuid>,
    pub scan_status: ScanStatus,
    /// Transport label recorded for observability (`postmark`, `smtp`, …).
    pub transport: String,
}

/// Idempotent insert: returns `Some(id)` when THIS call persisted the row,
/// `None` when a message with the same (org, rfc_message_id) already
/// exists — the replay-detection primitive for inbound processing.
pub async fn insert_message(conn: &mut PgConnection, m: &NewMessage) -> DbResult<Option<Uuid>> {
    let row = sqlx::query!(
        r#"INSERT INTO email_messages (id, org_id, direction, rfc_message_id, in_reply_to,
                                       email_refs, subject, from_address, from_name,
                                       to_addresses, body_text, ticket_id, comment_id,
                                       scan_status, transport)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
           ON CONFLICT (org_id, rfc_message_id) DO NOTHING
           RETURNING id"#,
        m.id,
        m.org_id,
        m.direction.as_str(),
        m.rfc_message_id,
        m.in_reply_to,
        &m.email_refs,
        m.subject,
        m.from_address,
        m.from_name,
        &m.to_addresses,
        m.body_text,
        m.ticket_id,
        m.comment_id,
        m.scan_status.as_str(),
        m.transport,
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|r| r.id))
}

pub async fn get_message<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<EmailMessage>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!("SELECT {MESSAGE_COLS} FROM email_messages WHERE org_id = $1 AND id = $2");
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_message(row)?)
}

/// Links a claimed inbound ledger row to its routing outcome after the
/// comment/ticket was created. Part of the insert-gate → act → link
/// sequence that makes replay handling race-free.
pub async fn link_message_targets<'e, E>(
    executor: E,
    org_id: Uuid,
    message_id: Uuid,
    ticket_id: Option<Uuid>,
    comment_id: Option<Uuid>,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query!(
        r#"UPDATE email_messages SET ticket_id = $3, comment_id = $4
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        message_id,
        ticket_id,
        comment_id,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Lists messages linked to a ticket, newest first (the UI thread view).
pub async fn list_for_ticket<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<Vec<EmailMessage>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "SELECT {MESSAGE_COLS} FROM email_messages \
         WHERE org_id = $1 AND ticket_id = $2 ORDER BY created_at DESC"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(ticket_id)
        .fetch_all(executor)
        .await?;
    Ok(rows
        .into_iter()
        .map(row_to_message)
        .collect::<Result<_, _>>()?)
}

// --- threading + routing lookups -------------------------------------------

/// Most recent ticket threaded to ANY of the normalized keys (bare,
/// lowercased Message-IDs from References ∪ In-Reply-To).
pub async fn find_thread_target<'e, E>(
    executor: E,
    org_id: Uuid,
    keys: &[String],
) -> DbResult<Option<Uuid>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    if keys.is_empty() {
        return Ok(None);
    }
    let ticket_id: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT ticket_id FROM email_messages
           WHERE org_id = $1 AND ticket_id IS NOT NULL
             AND (lower(rfc_message_id) = ANY($2) OR lower(in_reply_to) = ANY($2))
           ORDER BY created_at DESC
           LIMIT 1"#,
    )
    .bind(org_id)
    .bind(keys)
    .fetch_optional(executor)
    .await?;
    Ok(ticket_id)
}

/// Case-insensitive subject-token fallback: does `[<number>]` name an
/// existing ticket in this org?
pub async fn find_ticket_id_by_number_ci<'e, E>(
    executor: E,
    org_id: Uuid,
    number: &str,
) -> DbResult<Option<Uuid>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let id: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM tickets WHERE org_id = $1 AND upper(number) = upper($2)",
    )
    .bind(org_id)
    .bind(number.trim())
    .fetch_optional(executor)
    .await?;
    Ok(id)
}

/// Where an inbound recipient address routes: org + target board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InboundRoute {
    pub org_id: Uuid,
    pub board_id: Uuid,
}

/// Resolves recipients (to-before-cc priority preserved by the caller's
/// ordering) to a route. Board-level exact addresses win over org
/// catch-all; both are globally unique by index. Takes a connection so it
/// can run inside the caller's transaction.
pub async fn resolve_inbound_route(
    conn: &mut PgConnection,
    addresses: &[String],
) -> DbResult<Option<InboundRoute>> {
    if addresses.is_empty() {
        return Ok(None);
    }
    let direct: Option<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT org_id, id FROM boards WHERE lower(inbound_address) = ANY($1) LIMIT 1",
    )
    .bind(addresses)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some((org_id, board_id)) = direct {
        return Ok(Some(InboundRoute { org_id, board_id }));
    }
    let catchall: Option<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT org_id, catchall_board_id FROM email_inbound_settings \
         WHERE enabled AND lower(catchall_address) = ANY($1) LIMIT 1",
    )
    .bind(addresses)
    .fetch_optional(conn)
    .await?;
    Ok(catchall.map(|(org_id, board_id)| InboundRoute { org_id, board_id }))
}

// --- settings ---------------------------------------------------------------

pub struct InboundSettings {
    pub id: Uuid,
    pub org_id: Uuid,
    pub catchall_address: String,
    pub catchall_board_id: Uuid,
    pub enabled: bool,
}

pub async fn get_inbound_settings<'e, E>(
    executor: E,
    org_id: Uuid,
) -> DbResult<Option<InboundSettings>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query_as!(
        InboundSettings,
        r#"SELECT id, org_id, catchall_address, catchall_board_id, enabled
           FROM email_inbound_settings WHERE org_id = $1"#,
        org_id
    )
    .fetch_optional(executor)
    .await?;
    Ok(row)
}

/// Upserts the org-level catch-all (one row per org by constraint).
pub async fn upsert_inbound_settings(
    conn: &mut PgConnection,
    org_id: Uuid,
    catchall_address: &str,
    catchall_board_id: Uuid,
) -> DbResult<()> {
    sqlx::query!(
        r#"INSERT INTO email_inbound_settings
             (id, org_id, catchall_address, catchall_board_id)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (org_id) DO UPDATE
             SET catchall_address = EXCLUDED.catchall_address,
                 catchall_board_id = EXCLUDED.catchall_board_id"#,
        Uuid::now_v7(),
        org_id,
        catchall_address.trim().to_ascii_lowercase(),
        catchall_board_id,
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Sets/clears a board's inbound address (additive column on boards).
/// Returns NotFound when the board is not part of this org.
pub async fn set_board_inbound_address<'e, E>(
    executor: E,
    org_id: Uuid,
    board_id: Uuid,
    address: Option<&str>,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let res = sqlx::query!(
        r#"UPDATE boards SET inbound_address = $3
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        board_id,
        address.map(|a| a.trim().to_ascii_lowercase()),
    )
    .execute(executor)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

// --- outbound queue ---------------------------------------------------------

/// Enqueues one rendered outbound message: the ledger row plus its retry
/// record in the SAME transaction as the mutation that caused it. The
/// caller supplies the generated bare Message-ID so replies thread back.
pub struct NewOutbound {
    pub org_id: Uuid,
    pub rfc_message_id: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub subject: String,
    pub from_address: String,
    pub from_name: Option<String>,
    pub to_addresses: Vec<String>,
    pub body_text: String,
    pub ticket_id: Option<Uuid>,
    /// Transport label chosen from config at enqueue time.
    pub transport: String,
}

pub async fn enqueue_outbound(conn: &mut PgConnection, o: NewOutbound) -> DbResult<Uuid> {
    let msg_id = Uuid::now_v7();
    sqlx::query!(
        r#"INSERT INTO email_messages (id, org_id, direction, rfc_message_id, in_reply_to,
                                       email_refs, subject, from_address, from_name,
                                       to_addresses, body_text, ticket_id, scan_status,
                                       transport)
           VALUES ($1, $2, 'outbound', $3, $4, $5, $6, $7, $8, $9, $10, $11, 'skipped', $12)"#,
        msg_id,
        o.org_id,
        o.rfc_message_id,
        o.in_reply_to,
        &o.references,
        o.subject,
        o.from_address,
        o.from_name,
        &o.to_addresses,
        o.body_text,
        o.ticket_id,
        o.transport,
    )
    .execute(&mut *conn)
    .await?;
    let ledger_id = Uuid::now_v7();
    sqlx::query!(
        r#"INSERT INTO email_outbound (id, org_id, email_message_id)
           VALUES ($1, $2, $3)"#,
        ledger_id,
        o.org_id,
        msg_id,
    )
    .execute(&mut *conn)
    .await?;
    Ok(msg_id)
}

/// One message handed to the worker for sending. Mirrors
/// `core_mail::internal::ClaimedOutbound` on the wire.
#[derive(Debug, Clone)]
pub struct ClaimedOutboundRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub attempts: i32,
    pub max_attempts: i32,
    pub rfc_message_id: String,
    pub from_address: String,
    pub from_name: Option<String>,
    pub to_addresses: Vec<String>,
    pub subject: Option<String>,
    pub body_text: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
}

/// Atomically claims due sends: queued rows past their backoff deadline,
/// plus abandoned `sending` rows whose lease expired. MUST run inside a
/// transaction that executed `SET LOCAL ROLE meridian` — it spans all
/// orgs and therefore cannot pass RLS as `meridian_app`. `FOR UPDATE SKIP
/// LOCKED` keeps parallel workers honest; each claim increments `attempts`
/// so result handling can decide sent/dead/retry.
pub async fn claim_due_outbound<'e, E>(
    executor: E,
    limit: i64,
    lease_secs: f64,
) -> DbResult<Vec<ClaimedOutboundRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query!(
        r#"WITH claimed AS (
             UPDATE email_outbound
             SET status = 'sending', claimed_at = now(), updated_at = now(),
                 attempts = attempts + 1
             WHERE id IN (
               SELECT id FROM email_outbound
               WHERE (status = 'queued' AND next_attempt_at <= now())
                  OR (status = 'sending' AND claimed_at < now() - make_interval(secs => $2))
               ORDER BY next_attempt_at
               LIMIT $1
               FOR UPDATE SKIP LOCKED
             )
             RETURNING id, org_id, email_message_id, attempts, max_attempts
           )
           SELECT c.id, c.org_id, c.attempts, c.max_attempts,
                  m.rfc_message_id, m.from_address, m.from_name, m.to_addresses,
                  m.subject, m.body_text, m.in_reply_to, m.email_refs
           FROM claimed c
           JOIN email_messages m ON m.id = c.email_message_id
           ORDER BY c.id"#,
        limit,
        lease_secs,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ClaimedOutboundRow {
            id: r.id,
            org_id: r.org_id,
            attempts: r.attempts,
            max_attempts: r.max_attempts,
            rfc_message_id: r.rfc_message_id,
            from_address: r.from_address,
            from_name: r.from_name,
            to_addresses: r.to_addresses,
            subject: r.subject,
            body_text: r.body_text,
            in_reply_to: r.in_reply_to,
            references: r.email_refs,
        })
        .collect())
}

/// A worker-reported outcome for one claimed send.
#[derive(Debug, Clone)]
pub struct SendOutcome {
    pub id: Uuid,
    pub ok: bool,
    pub provider_message_id: Option<String>,
    pub error: Option<String>,
    /// Backoff before the next attempt when re-queueing (computed by the
    /// caller via `core_mail::outbound::backoff_delay`).
    pub retry_delay_secs: i64,
}

/// Applies one result. Guards on `status='sending'` so late/duplicate
/// reports after lease recovery cannot corrupt state. Returns whether a
/// row was updated.
pub async fn apply_send_outcome<'e, E>(executor: E, o: &SendOutcome) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let res = if o.ok {
        sqlx::query!(
            r#"UPDATE email_outbound
               SET status = 'sent', provider_message_id = $2, last_error = NULL,
                   claimed_at = NULL, updated_at = now()
               WHERE id = $1 AND status = 'sending'"#,
            o.id,
            o.provider_message_id,
        )
        .execute(executor)
        .await?
    } else {
        sqlx::query!(
            r#"UPDATE email_outbound
               SET status = CASE WHEN attempts >= max_attempts THEN 'dead' ELSE 'queued' END,
                   next_attempt_at = CASE WHEN attempts >= max_attempts THEN next_attempt_at
                                          ELSE now() + make_interval(secs => $3) END,
                   last_error = $2, provider_message_id = NULL,
                   claimed_at = NULL, updated_at = now()
               WHERE id = $1 AND status = 'sending'"#,
            o.id,
            o.error,
            o.retry_delay_secs as f64,
        )
        .execute(executor)
        .await?
    };
    Ok(res.rows_affected() > 0)
}
