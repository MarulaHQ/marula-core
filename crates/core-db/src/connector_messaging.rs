//! The messaging surface's storage half: channel destinations, posted cards,
//! the inbound-action ledger, and the vendor-identity join (ADR 0032, ADR 0033;
//! migration 0190).
//!
//! Purpose: everything an inbound callback needs to become an audited act.
//! A press arrives with a vendor user id, a channel id and a message id and
//! nothing else that may be trusted. This module answers the four questions
//! that follow:
//!
//! * which connection (and therefore org) owns this vendor workspace — by
//!   *candidate*, never as an authenticated answer (see
//!   [`callback_candidates`]);
//! * which Marula user is that vendor identity ([`identity_for_vendor`]);
//! * which ticket and connection a message belongs to ([`message_by_vendor`]);
//! * what the ticket says, and who may see it ([`ticket_facts`]).
//!
//! Plus the two writes: the card registry row ([`insert_message`]) and the
//! per-press ledger row ([`insert_action`]).
//!
//! Why the ledger is a table of its own rather than an `audit_events` row: it
//! is keyed by the callback digest so idempotency and evidence are one row,
//! and it carries the vendor channel/message/user vocabulary the channel
//! surface has to show. The route also writes the org-wide `audit_events` row,
//! because an operator looks there for "who took this ticket".
//!
//! Non-purpose: no signature verification, no permission decision, no vendor
//! HTTP. This layer assumes the caller has already resolved the org from a
//! verified callback and holds an org-bound transaction; the one deliberate
//! exception is [`callback_candidates`], which runs before any of that is
//! known and is why it goes through the narrow SECURITY DEFINER bridge.
//!
//! Query style matches `connector_connections`: runtime `sqlx::query` with
//! explicit row mapping, because several statements carry `ON CONFLICT`
//! inference against partial predicates and a compiled cache entry would need
//! re-preparing whenever one moved.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::{DbError, DbResult};

// ---- the unauthenticated door's candidates ---------------------------------

/// One connection that *might* own a vendor selector, with its sealed secret.
///
/// Returned by [`callback_candidates`] before any signature check. Every field
/// here is a candidate: the caller verifies the vendor signature over the raw
/// body with `secrets_encrypted` before it may believe `org_id`. A forged
/// Team id therefore selects a different secret and fails verification; it can
/// never select an org.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CallbackCandidate {
    pub r_id: Uuid,
    pub r_org_id: Uuid,
    pub r_installation_id: Uuid,
    pub r_label: String,
    pub r_settings: serde_json::Value,
    pub r_secrets_encrypted: Option<Vec<u8>>,
    pub r_status: String,
}

/// Connections whose shipped settings name this vendor selector.
///
/// `p_vendor` is `slack` (selector: the workspace/team id) or `teams`
/// (selector: the bot's application id). Both live in `connector_connections.
/// settings`, which is non-secret and queryable by design (ADR 0032), so the
/// lookup is one indexed-ish filter and never unseals anything.
///
/// A `disabled` connection is excluded by the bridge function: a destination
/// an operator switched off must not accept presses either.
pub async fn callback_candidates(
    pool: &PgPool,
    vendor: &str,
    selector: &str,
) -> DbResult<Vec<CallbackCandidate>> {
    let rows = sqlx::query_as::<_, CallbackCandidate>(
        "SELECT r_id, r_org_id, r_installation_id, r_label, r_settings, \
                r_secrets_encrypted, r_status \
           FROM connector_callback_candidates($1, $2)",
    )
    .bind(vendor)
    .bind(selector)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

// ---- destinations ----------------------------------------------------------

/// One enabled destination a connection may post a card to.
#[derive(Debug, Clone)]
pub struct Destination {
    pub id: Uuid,
    pub connection_id: Uuid,
    pub binding_kind: String,
    pub label: String,
    /// The vendor's channel id (Slack `C…`, Teams conversation id).
    pub external_id: String,
    /// The workspace/tenant the channel sits in (Slack `T…`, Teams tenant id).
    pub external_parent: Option<String>,
    /// Ids a deep link needs and that differ per kind. Ids only.
    pub vendor_ids: serde_json::Value,
}

fn row_to_destination(r: sqlx::postgres::PgRow) -> Destination {
    Destination {
        id: r.get("id"),
        connection_id: r.get("connection_id"),
        binding_kind: r.get("binding_kind"),
        label: r.get("label"),
        external_id: r.get("external_id"),
        external_parent: r.get("external_parent"),
        vendor_ids: r.get("vendor_ids"),
    }
}

/// The destination a share names, or the connection's first enabled one.
///
/// `channel_ref` is what the share dialog sends: the binding's `external_id`,
/// which is the vendor's own id and the only key the deep-link forms accept
/// safely (0185's comment on `external_id`). A name would be a guess; a wrong
/// or private channel answers a generic vendor error.
pub async fn find_destination(
    conn: &mut PgConnection,
    org_id: Uuid,
    connection_id: Uuid,
    channel_ref: Option<&str>,
    binding_kind: &str,
) -> DbResult<Option<Destination>> {
    let row = match channel_ref {
        Some(external_id) => {
            sqlx::query(
                "SELECT id, connection_id, binding_kind, label, external_id,
                        external_parent, vendor_ids
                   FROM connector_bindings
                  WHERE org_id = $1 AND connection_id = $2 AND binding_kind = $3
                    AND external_id = $4 AND enabled",
            )
            .bind(org_id)
            .bind(connection_id)
            .bind(binding_kind)
            .bind(external_id)
            .fetch_optional(&mut *conn)
            .await?
        }
        None => {
            sqlx::query(
                "SELECT id, connection_id, binding_kind, label, external_id,
                        external_parent, vendor_ids
                   FROM connector_bindings
                  WHERE org_id = $1 AND connection_id = $2 AND binding_kind = $3 AND enabled
                  ORDER BY label, created_at
                  LIMIT 1",
            )
            .bind(org_id)
            .bind(connection_id)
            .bind(binding_kind)
            .fetch_optional(&mut *conn)
            .await?
        }
    };
    Ok(row.map(row_to_destination))
}

/// Every destination a connection offers, for the share dialog's picker.
pub async fn list_destinations(
    conn: &mut PgConnection,
    org_id: Uuid,
    connection_id: Uuid,
) -> DbResult<Vec<Destination>> {
    let rows = sqlx::query(
        "SELECT id, connection_id, binding_kind, label, external_id,
                external_parent, vendor_ids
           FROM connector_bindings
          WHERE org_id = $1 AND connection_id = $2 AND enabled
          ORDER BY label, created_at",
    )
    .bind(org_id)
    .bind(connection_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(row_to_destination).collect())
}

// ---- the message registry --------------------------------------------------

const MESSAGE_COLUMNS: &str = "id, org_id, connection_id, binding_id, ticket_id, channel_id,
     message_id, shared_by, shared_client_id, created_at, updated_at";

/// One card posted into a channel.
#[derive(Debug, Clone)]
pub struct ChannelMessage {
    pub id: Uuid,
    pub org_id: Uuid,
    pub connection_id: Uuid,
    pub binding_id: Uuid,
    pub ticket_id: Uuid,
    pub channel_id: String,
    pub message_id: String,
    /// `None` when the sharer has since left the org.
    pub shared_by: Option<Uuid>,
    /// The client the card was scoped to when it was posted. A re-render may
    /// not widen what the channel already saw.
    pub shared_client_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn row_to_message(r: sqlx::postgres::PgRow) -> ChannelMessage {
    ChannelMessage {
        id: r.get("id"),
        org_id: r.get("org_id"),
        connection_id: r.get("connection_id"),
        binding_id: r.get("binding_id"),
        ticket_id: r.get("ticket_id"),
        channel_id: r.get("channel_id"),
        message_id: r.get("message_id"),
        shared_by: r.get("shared_by"),
        shared_client_id: r.get("shared_client_id"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

/// The row a share writes once the vendor accepted the message.
pub struct NewChannelMessage<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub connection_id: Uuid,
    pub binding_id: Uuid,
    pub ticket_id: Uuid,
    pub channel_id: &'a str,
    pub message_id: &'a str,
    pub shared_by: Option<Uuid>,
    pub shared_client_id: Option<Uuid>,
}

/// Registers a posted card.
///
/// The unique index makes this the idempotent half of a share retry: the same
/// vendor message id under the same connection is one row, and the caller can
/// treat a conflict as "already registered" rather than posting again.
pub async fn insert_message(
    conn: &mut PgConnection,
    row: NewChannelMessage<'_>,
) -> DbResult<ChannelMessage> {
    let row = sqlx::query(&format!(
        "INSERT INTO connector_channel_messages
           (id, org_id, connection_id, binding_id, ticket_id, channel_id, message_id,
            shared_by, shared_client_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         RETURNING {MESSAGE_COLUMNS}"
    ))
    .bind(row.id)
    .bind(row.org_id)
    .bind(row.connection_id)
    .bind(row.binding_id)
    .bind(row.ticket_id)
    .bind(row.channel_id)
    .bind(row.message_id)
    .bind(row.shared_by)
    .bind(row.shared_client_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row_to_message(row))
}

/// The registry row a vendor callback names, if this org posted that message.
///
/// The channel id is part of the lookup on purpose: a message id is unique per
/// channel, not per workspace, so omitting the channel could attribute a press
/// to another channel's card.
pub async fn message_by_vendor(
    conn: &mut PgConnection,
    org_id: Uuid,
    connection_id: Uuid,
    channel_id: &str,
    message_id: &str,
) -> DbResult<Option<ChannelMessage>> {
    let row = sqlx::query(&format!(
        "SELECT {MESSAGE_COLUMNS} FROM connector_channel_messages
          WHERE org_id = $1 AND connection_id = $2 AND channel_id = $3 AND message_id = $4"
    ))
    .bind(org_id)
    .bind(connection_id)
    .bind(channel_id)
    .bind(message_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(row_to_message))
}

// ---- the action ledger -----------------------------------------------------

const ACTION_COLUMNS: &str = "id, org_id, connection_id, callback_key, action, outcome,
     refusal_code, ticket_id, ticket_number, message_id, channel_id, vendor_user_id,
     actor_user_id, detail, created_at";

/// One inbound callback's ledger row.
#[derive(Debug, Clone)]
pub struct ActionRecord {
    pub id: Uuid,
    pub connection_id: Uuid,
    pub callback_key: String,
    pub action: String,
    /// `applied` or `refused`.
    pub outcome: String,
    pub refusal_code: Option<String>,
    pub ticket_id: Option<Uuid>,
    pub ticket_number: Option<String>,
    pub message_id: Option<Uuid>,
    pub channel_id: Option<String>,
    pub vendor_user_id: Option<String>,
    pub actor_user_id: Option<Uuid>,
    pub detail: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

fn row_to_action(r: sqlx::postgres::PgRow) -> ActionRecord {
    ActionRecord {
        id: r.get("id"),
        connection_id: r.get("connection_id"),
        callback_key: r.get("callback_key"),
        action: r.get("action"),
        outcome: r.get("outcome"),
        refusal_code: r.get("refusal_code"),
        ticket_id: r.get("ticket_id"),
        ticket_number: r.get("ticket_number"),
        message_id: r.get("message_id"),
        channel_id: r.get("channel_id"),
        vendor_user_id: r.get("vendor_user_id"),
        actor_user_id: r.get("actor_user_id"),
        detail: r.get("detail"),
        created_at: r.get("created_at"),
    }
}

/// The ledger row for a callback, if it has already been handled.
///
/// This is the replay read. It runs after [`lock_callback`], so a concurrent
/// duplicate's row is committed and visible by the time it answers.
pub async fn action_by_callback(
    conn: &mut PgConnection,
    org_id: Uuid,
    connection_id: Uuid,
    callback_key: &str,
) -> DbResult<Option<ActionRecord>> {
    let row = sqlx::query(&format!(
        "SELECT {ACTION_COLUMNS} FROM connector_action_audit
          WHERE org_id = $1 AND connection_id = $2 AND callback_key = $3"
    ))
    .bind(org_id)
    .bind(connection_id)
    .bind(callback_key)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(row_to_action))
}

/// Serialises two callbacks that carry the same digest, for this transaction.
///
/// Why an advisory lock rather than an insert-first claim: the ledger is
/// append-only for the application role, so a claim row could not be completed
/// with its outcome afterwards. A transaction-scoped lock gives the same
/// guarantee — the second callback waits, then reads the first one's committed
/// row and replays it — while letting the ledger stay one immutable row per
/// callback.
///
/// The key is hashed, so a collision between two unrelated callbacks merely
/// serialises them; it cannot merge their evidence, because the ledger's own
/// unique index is on the exact key.
pub async fn lock_callback(
    conn: &mut PgConnection,
    org_id: Uuid,
    connection_id: Uuid,
    callback_key: &str,
) -> DbResult<()> {
    sqlx::query(
        "SELECT pg_advisory_xact_lock(
                  hashtextextended('connector_action:' || $1 || ':' || $2 || ':' || $3, 0))",
    )
    .bind(org_id)
    .bind(connection_id)
    .bind(callback_key)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The ledger row describing one callback, applied or refused.
pub struct NewAction<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub connection_id: Uuid,
    pub callback_key: &'a str,
    pub action: &'a str,
    pub outcome: &'a str,
    pub refusal_code: Option<&'a str>,
    pub ticket_id: Option<Uuid>,
    pub ticket_number: Option<&'a str>,
    pub message_id: Option<Uuid>,
    pub channel_id: Option<&'a str>,
    pub vendor_user_id: Option<&'a str>,
    pub actor_user_id: Option<Uuid>,
    pub detail: &'a serde_json::Value,
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_action(conn: &mut PgConnection, row: NewAction<'_>) -> DbResult<ActionRecord> {
    let inserted = sqlx::query(&format!(
        "INSERT INTO connector_action_audit
           (id, org_id, connection_id, callback_key, action, outcome, refusal_code,
            ticket_id, ticket_number, message_id, channel_id, vendor_user_id,
            actor_user_id, detail)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
         ON CONFLICT (org_id, connection_id, callback_key) DO NOTHING
         RETURNING {ACTION_COLUMNS}"
    ))
    .bind(row.id)
    .bind(row.org_id)
    .bind(row.connection_id)
    .bind(row.callback_key)
    .bind(row.action)
    .bind(row.outcome)
    .bind(row.refusal_code)
    .bind(row.ticket_id)
    .bind(row.ticket_number)
    .bind(row.message_id)
    .bind(row.channel_id)
    .bind(row.vendor_user_id)
    .bind(row.actor_user_id)
    .bind(row.detail)
    .fetch_optional(&mut *conn)
    .await?;

    match inserted {
        Some(row) => Ok(row_to_action(row)),
        // The advisory lock should make this unreachable for a live retry;
        // reaching it means the same digest was handled in another session
        // between the lock and this insert (or the lock was skipped), and the
        // stored row is still the correct answer.
        None => action_by_callback(conn, row.org_id, row.connection_id, row.callback_key)
            .await?
            .ok_or(DbError::NotFound),
    }
}

/// The ticket's press history, newest first — what the ticket surface shows.
pub async fn list_actions_for_ticket(
    conn: &mut PgConnection,
    org_id: Uuid,
    ticket_id: Uuid,
    limit: i64,
) -> DbResult<Vec<ActionRecord>> {
    let rows = sqlx::query(&format!(
        "SELECT {ACTION_COLUMNS} FROM connector_action_audit
          WHERE org_id = $1 AND ticket_id = $2
          ORDER BY created_at DESC LIMIT $3"
    ))
    .bind(org_id)
    .bind(ticket_id)
    .bind(limit.clamp(1, 200))
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(row_to_action).collect())
}

// ---- vendor identity -------------------------------------------------------

/// The Marula user a vendor identity maps to, if any.
///
/// `identity_kind` is the closed vocabulary migration 0185 defines
/// (`slack_user`, `teams_user`, …). A missing row is the answer "nobody has
/// linked this account", which the caller turns into a typed refusal that names
/// the link step — never a silent no-op and never an act performed as somebody
/// else.
pub async fn identity_for_vendor(
    conn: &mut PgConnection,
    org_id: Uuid,
    connection_id: Uuid,
    identity_kind: &str,
    external_id: &str,
) -> DbResult<Option<Uuid>> {
    let row = sqlx::query(
        "SELECT user_id FROM user_external_identities
          WHERE org_id = $1 AND connection_id = $2 AND identity_kind = $3 AND external_id = $4",
    )
    .bind(org_id)
    .bind(connection_id)
    .bind(identity_kind)
    .bind(external_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|r| r.get("user_id")))
}

// ---- ticket facts for a card ----------------------------------------------

/// The six facts a card renders, plus the two decisions the caller needs.
///
/// Deliberately not the ticket entity: a card shows number, title, client,
/// status, priority and assignee, and nothing else. The body and the notes are
/// absent by construction, so a card cannot leak a note into a channel that the
/// channel's readers were never granted.
#[derive(Debug, Clone)]
pub struct TicketFacts {
    pub ticket_id: Uuid,
    pub number: String,
    pub title: String,
    pub board_id: Uuid,
    pub status_id: Uuid,
    pub status_name: String,
    /// `open` | `in_progress` | `waiting` | `closed`.
    pub status_category: String,
    /// `p1` … `p4`.
    pub priority: String,
    pub client_id: Option<Uuid>,
    pub client_name: Option<String>,
    /// The client's `restricted_to` list. Empty means "everyone in the org".
    pub restricted_to: Vec<Uuid>,
    pub assignee_id: Option<Uuid>,
    pub assignee_name: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// One ticket's card facts, or `None` when the ticket is not in this org.
pub async fn ticket_facts(
    conn: &mut PgConnection,
    org_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<Option<TicketFacts>> {
    let row = sqlx::query(
        "SELECT t.id, t.number, t.title, t.board_id, t.status_id, t.priority,
                t.client_id, t.assignee_id, t.updated_at,
                st.name AS status_name, st.category AS status_category,
                cl.name AS client_name,
                COALESCE(cl.restricted_to, '[]'::jsonb) AS restricted_to,
                u.display_name AS assignee_name
           FROM tickets t
           JOIN board_statuses st ON st.id = t.status_id
           LEFT JOIN clients cl ON cl.id = t.client_id AND cl.org_id = t.org_id
           LEFT JOIN users u ON u.id = t.assignee_id
          WHERE t.org_id = $1 AND t.id = $2",
    )
    .bind(org_id)
    .bind(ticket_id)
    .fetch_optional(&mut *conn)
    .await?;

    let Some(row) = row else { return Ok(None) };
    let raw: serde_json::Value = row.get("restricted_to");
    Ok(Some(TicketFacts {
        ticket_id: row.get("id"),
        number: row.get("number"),
        title: row.get("title"),
        board_id: row.get("board_id"),
        status_id: row.get("status_id"),
        status_name: row.get("status_name"),
        status_category: row.get("status_category"),
        priority: row.get("priority"),
        client_id: row.get("client_id"),
        client_name: row.get("client_name"),
        restricted_to: restriction_list(&raw),
        assignee_id: row.get("assignee_id"),
        assignee_name: row.get("assignee_name"),
        updated_at: row.get("updated_at"),
    }))
}

/// Resolves a ticket by its human number (`T-1042`) inside one org.
///
/// The number is what a slash command receives. Unique per org
/// (`tickets_org_id_number_key`, 0004), so the lookup is exact and cannot pick
/// a sibling.
pub async fn ticket_id_by_number(
    conn: &mut PgConnection,
    org_id: Uuid,
    number: &str,
) -> DbResult<Option<Uuid>> {
    let row = sqlx::query("SELECT id FROM tickets WHERE org_id = $1 AND number = $2")
        .bind(org_id)
        .bind(number)
        .fetch_optional(&mut *conn)
        .await?;
    Ok(row.map(|r| r.get("id")))
}

/// `clients.restricted_to` is a JSON array of user ids; a malformed entry is
/// dropped rather than denied, matching `connector_connections`' rule: an
/// unreadable restriction list must not turn into "nobody may see this".
fn restriction_list(raw: &serde_json::Value) -> Vec<Uuid> {
    raw.as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str()?.parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restriction_list_drops_malformed_entries() {
        let raw = serde_json::json!(["6d657269-6469-616e-0000-000000000001", 7, "nope"]);
        assert_eq!(restriction_list(&raw).len(), 1);
        assert!(restriction_list(&serde_json::json!([])).is_empty());
        assert!(restriction_list(&serde_json::Value::Null).is_empty());
    }
}
