//! Email message ledger and outbound send-queue queries (the private build tracker 2.4).
//!
//! Purpose: persistence for `email_messages` (idempotent inbound ledger +
//! outbound content), the `email_outbound` retry queue with atomic
//! claiming, inbound routing resolution (board address → org catch-all),
//! and the admin settings upserts. The claim query is the one statement
//! that must run as the TABLE OWNER — callers issue `SET LOCAL ROLE
//! marula` first (see `claim_due_outbound`).
//!
//! Non-purpose: no transport calls, scan verdicts, or routing policy —
//! core-mail defines traits/math, core-api orchestrates; this module only
//! stores and retrieves.

use crate::{DbError, DbResult};
use core_domain::entities::{EmailDirection, EmailMessage, ScanStatus};
use core_domain::settings::{board_identity_key, board_identity_parts, BoardIdentityField};
use sqlx::PgConnection;
use uuid::Uuid;

/// Domain suffix for Marula-generated outbound Message-IDs. Stored bare
/// (`uuid@marula.local`); transport adapters add RFC angle brackets.
pub const OUTBOUND_ID_SUFFIX: &str = "marula.local";

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

/// Recently quarantined inbound messages, newest first — the self-host
/// detections list (Wave 36 lane 3). Callers run it on an org-scoped
/// connection so RLS applies; `limit` is caller-bounded.
pub async fn list_recent_detections<'e, E>(
    executor: E,
    org_id: Uuid,
    limit: i64,
) -> DbResult<Vec<EmailMessage>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "SELECT {MESSAGE_COLS} FROM email_messages \
         WHERE org_id = $1 AND direction = 'inbound' AND scan_status = 'infected' \
         ORDER BY created_at DESC LIMIT $2"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(limit)
        .fetch_all(executor)
        .await?;
    rows.into_iter()
        .map(|row| row_to_message(row).map_err(DbError::from))
        .collect()
}

/// Scan-surface freshness for the self-host pane: newest inbound scan,
/// newest recorded quarantine, and the lifetime quarantine count.
pub struct ScanFreshness {
    pub last_scan_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_detection_at: Option<chrono::DateTime<chrono::Utc>>,
    pub detection_count: i64,
}

pub async fn scan_freshness<'e, E>(executor: E, org_id: Uuid) -> DbResult<ScanFreshness>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query_as::<
        _,
        (
            Option<chrono::DateTime<chrono::Utc>>,
            Option<chrono::DateTime<chrono::Utc>>,
            i64,
        ),
    >(
        r#"SELECT
             max(created_at) FILTER (WHERE direction = 'inbound'),
             max(created_at) FILTER (WHERE direction = 'inbound' AND scan_status = 'infected'),
             count(*) FILTER (WHERE direction = 'inbound' AND scan_status = 'infected')
           FROM email_messages WHERE org_id = $1"#,
    )
    .bind(org_id)
    .fetch_one(executor)
    .await?;
    Ok(ScanFreshness {
        last_scan_at: row.0,
        last_detection_at: row.1,
        detection_count: row.2,
    })
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

/// Delivery state of one outbound message, projected from the
/// `email_outbound` retry ledger (0014). `None`-shaped statuses (inbound
/// rows) simply have no row here.
#[derive(Debug, Clone, PartialEq)]
pub struct OutboundDelivery {
    pub email_message_id: Uuid,
    /// queued | sending | sent | dead
    pub status: String,
    pub attempts: i32,
    pub max_attempts: i32,
    pub last_error: Option<String>,
}

/// Delivery rows for the given outbound messages, creation-ordered. At
/// most one ledger row exists per message today (enqueue is exactly-once
/// inside the causing transaction); if that ever loosens, the caller keeps
/// the FIRST row per message — deterministic by creation order.
pub async fn outbound_delivery_for_messages<'e, E>(
    executor: E,
    org_id: Uuid,
    message_ids: &[Uuid],
) -> DbResult<Vec<OutboundDelivery>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    if message_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query!(
        r#"SELECT email_message_id, status, attempts, max_attempts, last_error
           FROM email_outbound
           WHERE org_id = $1 AND email_message_id = ANY($2)
           ORDER BY created_at, id"#,
        org_id,
        message_ids
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| OutboundDelivery {
            email_message_id: r.email_message_id,
            status: r.status,
            attempts: r.attempts,
            max_attempts: r.max_attempts,
            last_error: r.last_error,
        })
        .collect())
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

// --- SECURITY DEFINER seams (the only cross-org statements) ------------------

/// Production route resolution for the RLS-constrained `marula_app`
/// login: runs `resolve_inbound_email_route` (SECURITY DEFINER, migration
/// 0014). Takes a connection usable inside the caller's transaction.
pub async fn resolve_inbound_route_for_app(
    conn: &mut PgConnection,
    addresses: &[String],
) -> DbResult<Option<InboundRoute>> {
    if addresses.is_empty() {
        return Ok(None);
    }
    let row: Option<(Uuid, Uuid)> =
        sqlx::query_as("SELECT r_org_id, r_board_id FROM resolve_inbound_email_route($1)")
            .bind(addresses)
            .fetch_optional(conn)
            .await?;
    Ok(row.map(|(org_id, board_id)| InboundRoute { org_id, board_id }))
}

/// Production claim path: calls `claim_due_email_outbound` (SECURITY
/// DEFINER). Works from any login without weakening RLS elsewhere. The
/// clock is the database's now() — deterministic tests and the CI harness
/// use [`claim_due_outbound`] on an owner-role connection instead.
pub async fn claim_due_outbound_for_app(
    conn: &mut PgConnection,
    limit: i32,
    lease_secs: i64,
) -> DbResult<Vec<ClaimedOutboundRow>> {
    let rows = sqlx::query!(
        r#"SELECT r_ledger_id, r_org_id, r_attempts, r_max_attempts,
                  r_rfc_message_id, r_from_address, r_from_name, r_to_addresses,
                  r_subject, r_body_text, r_in_reply_to, r_refs
           FROM claim_due_email_outbound($1, make_interval(secs => $2))"#,
        limit,
        lease_secs as f64,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ClaimedOutboundRow {
            // RETURNS TABLE columns are nullable to SQL; the function
            // guarantees values, so defaults only satisfy types.
            id: r.r_ledger_id.unwrap_or_default(),
            org_id: r.r_org_id.unwrap_or_default(),
            attempts: r.r_attempts.unwrap_or(0),
            max_attempts: r.r_max_attempts.unwrap_or(8),
            rfc_message_id: r.r_rfc_message_id.unwrap_or_default(),
            from_address: r.r_from_address.unwrap_or_default(),
            from_name: r.r_from_name,
            to_addresses: r.r_to_addresses.unwrap_or_default(),
            subject: r.r_subject,
            body_text: r.r_body_text,
            in_reply_to: r.r_in_reply_to,
            references: r.r_refs.unwrap_or_default(),
        })
        .collect())
}

/// Production result-application path via `apply_email_send_result`
/// (SECURITY DEFINER). Returns whether the report changed a row.
pub async fn apply_send_outcome_for_app(
    conn: &mut PgConnection,
    o: &SendOutcome,
) -> DbResult<bool> {
    let applied: Option<bool> = sqlx::query_scalar(
        "SELECT apply_email_send_result($1, $2, $3, $4, make_interval(secs => $5), $6)",
    )
    .bind(o.id)
    .bind(o.ok)
    .bind(o.provider_message_id.clone())
    .bind(o.error.clone())
    .bind(o.retry_delay_secs as f64)
    .bind(o.permanent)
    .fetch_one(conn)
    .await?;
    Ok(applied.unwrap_or(false))
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

/// Toggles the org catch-all's enabled state (no-op when no row exists).
pub async fn set_inbound_enabled(
    conn: &mut PgConnection,
    org_id: Uuid,
    enabled: bool,
) -> DbResult<bool> {
    let res = sqlx::query!(
        "UPDATE email_inbound_settings SET enabled = $2 WHERE org_id = $1",
        org_id,
        enabled
    )
    .execute(&mut *conn)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Every board in the org with its inbound address — the routing table's
/// backing query (board name + address + identity live in three homes; the
/// API composes them).
pub async fn list_board_routes(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<(Uuid, String, Option<String>)>> {
    let rows = sqlx::query!(
        r#"SELECT id, name, inbound_address FROM boards
           WHERE org_id = $1 ORDER BY is_default DESC, name"#,
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.id, r.name, r.inbound_address))
        .collect())
}

// --- Wave 34 lane 8: per-board outbound identities --------------------------
//
// Stored as org-scope settings under board-qualified keys
// (`email.board.<board_id>.<field>`, core-domain owns the key grammar).
// Absent field = fall through to the org identity; the settings model holds
// them, so no schema change was required.

/// The four optional per-board outbound identity fields.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoardIdentity {
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub reply_to: Option<String>,
    pub signature: Option<String>,
}

impl BoardIdentity {
    pub fn is_empty(&self) -> bool {
        self.from_address.is_none()
            && self.from_name.is_none()
            && self.reply_to.is_none()
            && self.signature.is_none()
    }

    fn set(&mut self, field: BoardIdentityField, value: Option<String>) {
        match field {
            BoardIdentityField::FromAddress => self.from_address = value,
            BoardIdentityField::FromName => self.from_name = value,
            BoardIdentityField::ReplyTo => self.reply_to = value,
            BoardIdentityField::Signature => self.signature = value,
        }
    }
}

/// Loads one board's identity. Unknown/foreign keys are skipped; only the
/// four known fields are read (a typo can never surface as configuration).
pub async fn get_board_identity(
    conn: &mut PgConnection,
    org_id: Uuid,
    board_id: Uuid,
) -> DbResult<BoardIdentity> {
    let rows = crate::settings::list_org_values_with_prefix(
        &mut *conn,
        org_id,
        &format!("email.board.{board_id}."),
    )
    .await?;
    let mut identity = BoardIdentity::default();
    for row in rows {
        let Some((id, field)) = board_identity_parts(&row.key) else {
            continue;
        };
        if id != board_id {
            continue;
        }
        let value = row
            .value
            .as_str()
            .map(str::to_string)
            .filter(|v| !v.trim().is_empty());
        identity.set(field, value);
    }
    Ok(identity)
}

/// Sets or clears one board identity field. `None`/blank clears (the row is
/// deleted so the org identity applies again). Returns NotFound when the
/// board is not in this org. `updated_by` is the ACTING USER — `settings.
/// updated_by` references `users(id)`, so passing the org id here fails the
/// FK (Wave 34 lane 6 fix for the fast-forwarded lane-8 code).
pub async fn set_board_identity_field(
    tx: &mut PgConnection,
    org_id: Uuid,
    board_id: Uuid,
    field: BoardIdentityField,
    value: Option<&str>,
    updated_by: Uuid,
) -> DbResult<()> {
    let exists: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM boards WHERE org_id = $1 AND id = $2")
            .bind(org_id)
            .bind(board_id)
            .fetch_optional(&mut *tx)
            .await?;
    if exists.is_none() {
        return Err(DbError::NotFound);
    }
    let key = board_identity_key(board_id, field);
    let trimmed = value.map(str::trim).filter(|v| !v.is_empty());
    match trimmed {
        Some(v) => {
            crate::settings::upsert_org_setting(
                tx,
                org_id,
                &key,
                &serde_json::json!(v),
                updated_by,
            )
            .await?;
        }
        None => crate::settings::delete_org_setting(tx, org_id, &key).await?,
    }
    Ok(())
}

/// Resolves the board Reply-To for an outbound message by matching the
/// message's From address against board identity From addresses. Used at
/// send time (the claim carries from_address; Reply-To is not a ledger
/// column).
pub async fn board_reply_to_for_from_address(
    conn: &mut PgConnection,
    org_id: Uuid,
    from_address: &str,
) -> DbResult<Option<String>> {
    let reply_to: Option<String> = sqlx::query_scalar(
        r#"SELECT r.value #>> '{}'
           FROM settings f
           JOIN settings r
             ON r.org_id = f.org_id AND r.scope = f.scope
            AND r.key = replace(f.key, '.from_address', '.reply_to')
           WHERE f.org_id = $1 AND f.scope = 'org'
             AND f.key LIKE 'email.board.%.from_address'
             AND lower(f.value #>> '{}') = lower($2)
           LIMIT 1"#,
    )
    .bind(org_id)
    .bind(from_address)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(reply_to.filter(|v| !v.trim().is_empty()))
}

/// Resolves the effective Reply-To for an outbound message: the board
/// identity whose From address matches wins, else the org-level
/// `email.reply_to` setting. `None` sends no Reply-To header.
pub async fn resolved_reply_to(
    conn: &mut PgConnection,
    org_id: Uuid,
    from_address: &str,
) -> DbResult<Option<String>> {
    if let Some(board) = board_reply_to_for_from_address(&mut *conn, org_id, from_address).await? {
        return Ok(Some(board));
    }
    let org =
        crate::settings::get_org_value(&mut *conn, org_id, core_domain::settings::EMAIL_REPLY_TO)
            .await?;
    Ok(org
        .and_then(|v| v.as_str().map(str::to_string))
        .filter(|v| !v.trim().is_empty()))
}

// --- Wave 34 lane 8: outbound delivery views --------------------------------

/// One row of the admin delivery ledger: the outbound message joined with
/// its retry state.
#[derive(Debug, Clone, PartialEq)]
pub struct OutboundLedgerRow {
    pub message_id: Uuid,
    pub rfc_message_id: String,
    pub subject: Option<String>,
    pub to_addresses: Vec<String>,
    pub from_address: String,
    /// queued | sending | sent | dead
    pub status: String,
    pub attempts: i32,
    pub max_attempts: i32,
    pub last_error: Option<String>,
    pub ticket_id: Option<Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Which rung of the greeting/signature chain supplied each block for
    /// THIS message (Wave 42), or `None` for a row written before the lane
    /// recorded it.
    pub greeting_source: Option<String>,
    pub signature_source: Option<String>,
}

/// Newest-first page of the org's outbound ledger, optionally filtered by
/// status. `before` pages on `(created_at, id)` like the notification dock.
pub async fn list_outbound_ledger(
    conn: &mut PgConnection,
    org_id: Uuid,
    status: Option<&str>,
    before: Option<(chrono::DateTime<chrono::Utc>, Uuid)>,
    limit: i64,
) -> DbResult<Vec<OutboundLedgerRow>> {
    let rows = sqlx::query!(
        r#"SELECT m.id, m.rfc_message_id, m.subject, m.to_addresses, m.from_address,
                  m.ticket_id, m.created_at, m.greeting_source, m.signature_source,
                  q.status, q.attempts, q.max_attempts,
                  q.last_error, q.updated_at
           FROM email_messages m
           JOIN email_outbound q ON q.email_message_id = m.id
           WHERE m.org_id = $1 AND m.direction = 'outbound'
             AND ($2::text IS NULL OR q.status = $2)
             AND ($3::timestamptz IS NULL OR (m.created_at, m.id) < ($3, $4))
           ORDER BY m.created_at DESC, m.id DESC
           LIMIT $5"#,
        org_id,
        status,
        before.map(|b| b.0),
        before.map(|b| b.1),
        limit,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| OutboundLedgerRow {
            message_id: r.id,
            rfc_message_id: r.rfc_message_id,
            subject: r.subject,
            to_addresses: r.to_addresses,
            from_address: r.from_address,
            status: r.status,
            attempts: r.attempts,
            max_attempts: r.max_attempts,
            last_error: r.last_error,
            ticket_id: r.ticket_id,
            created_at: r.created_at,
            updated_at: r.updated_at,
            greeting_source: r.greeting_source,
            signature_source: r.signature_source,
        })
        .collect())
}

/// Counts per delivery status for the summary strip. Statuses with zero
/// rows are absent — the caller decides how to render the complete set.
pub async fn outbound_status_counts(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<(String, i64)>> {
    let rows = sqlx::query!(
        r#"SELECT q.status, COUNT(*) AS count
           FROM email_outbound q
           WHERE q.org_id = $1
           GROUP BY q.status
           ORDER BY q.status"#,
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.status, r.count.unwrap_or(0)))
        .collect())
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
    /// Which rung of the resolution chain supplied the signature that went
    /// out (Wave 42) — `None` for a message whose ledger row predates the
    /// lane, which the delivery view reads as `unset` rather than a guess.
    pub signature_source: Option<String>,
    /// The same for the greeting. `None` on a message that had no greeting
    /// block at all is the same shape as "not recorded", and both read as
    /// `unset`: the column says what was recorded, not what was true.
    pub greeting_source: Option<String>,
}

pub async fn enqueue_outbound(conn: &mut PgConnection, o: NewOutbound) -> DbResult<Uuid> {
    let msg_id = Uuid::now_v7();
    sqlx::query!(
        r#"INSERT INTO email_messages (id, org_id, direction, rfc_message_id, in_reply_to,
                                       email_refs, subject, from_address, from_name,
                                       to_addresses, body_text, ticket_id, scan_status,
                                       transport, signature_source, greeting_source)
           VALUES ($1, $2, 'outbound', $3, $4, $5, $6, $7, $8, $9, $10, $11, 'skipped', $12, $13, $14)"#,
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
        o.signature_source,
        o.greeting_source,
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

/// Owner-connection variant of the claim used by tests and the CI mail
/// harness where the simulated clock must be injected (`as_of`). Requires
/// an owner-role connection (table owner bypasses RLS absent FORCE);
/// production endpoints call [`claim_due_outbound_for_app`] instead.
/// `org_filter` scopes the batch to one org (`None` = every org).
pub async fn claim_due_outbound<'e, E>(
    executor: E,
    org_filter: Option<Uuid>,
    limit: i64,
    lease_secs: f64,
    as_of: chrono::DateTime<chrono::Utc>,
) -> DbResult<Vec<ClaimedOutboundRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query!(
        r#"WITH claimed AS (
             UPDATE email_outbound
             SET status = 'sending', claimed_at = $3, updated_at = $3,
                 attempts = attempts + 1
             WHERE id IN (
               SELECT id FROM email_outbound
               WHERE (($4::uuid IS NULL OR org_id = $4)
                 AND ((status = 'queued' AND next_attempt_at <= $3)
                   OR (status = 'sending' AND claimed_at < $3 - make_interval(secs => $2))))
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
        as_of,
        org_filter,
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
    /// The transport says retrying this identical message cannot succeed —
    /// a bad address, an inactive recipient, a refused sender. The ledger
    /// dead-letters it on this report instead of spending the rest of its
    /// attempt budget proving the same thing, which is what used to happen and
    /// is why a dead email showed up an hour late with eight identical errors.
    pub permanent: bool,
}

/// Owner-connection result application with injectable clock — the test/
/// harness counterpart of [`apply_send_outcome_for_app`]. Guards on
/// `status='sending'` so late/duplicate reports after lease recovery are
/// inert. Returns whether a row was updated.
pub async fn apply_send_outcome<'e, E>(
    executor: E,
    o: &SendOutcome,
    as_of: chrono::DateTime<chrono::Utc>,
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let res = if o.ok {
        // Runtime statement, not `query!`: the workspace builds with
        // `SQLX_OFFLINE=true` against the committed `.sqlx` cache, and this
        // lane cannot run `cargo sqlx prepare`. The DB suite
        // (`tests/email_queries.rs`) executes both arms against a migrated
        // database, which is what proves they parse and bind.
        sqlx::query(
            r#"UPDATE email_outbound
               SET status = 'sent', provider_message_id = $2, last_error = NULL,
                   claimed_at = NULL, updated_at = $3
               WHERE id = $1 AND status = 'sending'"#,
        )
        .bind(o.id)
        .bind(o.provider_message_id.clone())
        .bind(as_of)
        .execute(executor)
        .await?
    } else {
        sqlx::query(
            r#"UPDATE email_outbound
               SET status = CASE WHEN $5 OR attempts >= max_attempts THEN 'dead' ELSE 'queued' END,
                   next_attempt_at = CASE WHEN $5 OR attempts >= max_attempts THEN next_attempt_at
                                          ELSE ($4::timestamptz + make_interval(secs => $3)) END,
                   last_error = $2, provider_message_id = NULL,
                   claimed_at = NULL, updated_at = $4
               WHERE id = $1 AND status = 'sending'"#,
        )
        .bind(o.id)
        .bind(o.error.clone())
        .bind(o.retry_delay_secs as f64)
        .bind(as_of)
        .bind(o.permanent)
        .execute(executor)
        .await?
    };
    Ok(res.rows_affected() > 0)
}
