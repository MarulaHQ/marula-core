//! core-db/alerting — persistence for the alert ingestor (the private build tracker
//! Phase 3, migration 0037).
//!
//! Purpose: typed queries for the three ingest tables — alert sources
//! (per-org ingest configuration with the HMAC secret sealed at rest),
//! the normalized alert ledger (UNIQUE (org, source, dedup_key) is the
//! replay gate), and the storm-collapse groups (a partial unique index
//! allows exactly one OPEN group per correlation key).
//!
//! The transactional storm logic (lock group → collapse-or-create →
//! ticket creation) lives in the API route; this module only owns SQL.
//!
//! Non-purpose: no HMAC math, no secret sealing decisions (callers own
//! key material), no outbox emission (routes emit through their tx ctx).

use crate::{DbError, DbResult};
use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

// --- alert sources -----------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AlertSourceRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    /// Public ingest routing id — globally unique, never secret.
    pub source_ref: String,
    /// AES-256-GCM ciphertext (`nonce || ct`). Never logged, never
    /// returned over the wire.
    pub secret_encrypted: Vec<u8>,
    pub board_id: Uuid,
    pub client_id: Option<Uuid>,
    pub correlation_window_secs: i32,
    pub enabled: bool,
    /// When set, this source is a generic M2M webhook receiver that emits
    /// domain-event envelopes (Wave 43 lane 1). NULL = alert source (Wave 9).
    pub event_type: Option<String>,
    /// Payload contract pin; NULL defaults to 'core-domain/v1' at ingest.
    pub payload_schema_ref: Option<String>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const SOURCE_COLS: &str = "id, org_id, name, source_ref, secret_encrypted, board_id, client_id, \
     correlation_window_secs, enabled, event_type, payload_schema_ref, created_by, created_at, updated_at";
const SOURCE_COLS_FOR_SELECT: &str = "SELECT id, org_id, name, source_ref, secret_encrypted, \
     board_id, client_id, correlation_window_secs, enabled, event_type, payload_schema_ref, \
     created_by, created_at, updated_at";

pub struct NewAlertSource<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: &'a str,
    pub source_ref: &'a str,
    pub secret_encrypted: &'a [u8],
    pub board_id: Uuid,
    pub client_id: Option<Uuid>,
    pub correlation_window_secs: i32,
    pub event_type: Option<&'a str>,
    pub payload_schema_ref: Option<&'a str>,
    pub created_by: Uuid,
}

pub async fn insert_alert_source<'e, E>(
    executor: E,
    s: NewAlertSource<'_>,
) -> DbResult<AlertSourceRow>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "INSERT INTO alert_sources \
             (id, org_id, name, source_ref, secret_encrypted, board_id, \
              client_id, correlation_window_secs, event_type, payload_schema_ref, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
         RETURNING {SOURCE_COLS}"
    );
    sqlx::query_as::<_, AlertSourceRow>(&sql)
        .bind(s.id)
        .bind(s.org_id)
        .bind(s.name)
        .bind(s.source_ref)
        .bind(s.secret_encrypted)
        .bind(s.board_id)
        .bind(s.client_id)
        .bind(s.correlation_window_secs)
        .bind(s.event_type)
        .bind(s.payload_schema_ref)
        .bind(s.created_by)
        .fetch_one(executor)
        .await
        .map_err(Into::into)
}

pub async fn get_alert_source<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<AlertSourceRow>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!("{SOURCE_COLS_FOR_SELECT} FROM alert_sources WHERE id = $1 AND org_id = $2");
    sqlx::query_as::<_, AlertSourceRow>(&sql)
        .bind(id)
        .bind(org_id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)
}

pub async fn list_alert_sources<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<AlertSourceRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "{SOURCE_COLS_FOR_SELECT} FROM alert_sources WHERE org_id = $1 \
         ORDER BY created_at DESC, id DESC"
    );
    sqlx::query_as::<_, AlertSourceRow>(&sql)
        .bind(org_id)
        .fetch_all(executor)
        .await
        .map_err(Into::into)
}

/// Patchable source config; `None` leaves a field untouched.
#[derive(Debug, Default, Clone)]
pub struct AlertSourcePatch<'a> {
    pub name: Option<&'a str>,
    pub board_id: Option<Uuid>,
    /// `Some(None)` clears; `Some(Some(v))` sets.
    pub client_id: Option<Option<Uuid>>,
    pub correlation_window_secs: Option<i32>,
    pub enabled: Option<bool>,
    pub event_type: Option<Option<&'a str>>,
    pub payload_schema_ref: Option<Option<&'a str>>,
}

pub async fn update_alert_source<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    p: AlertSourcePatch<'_>,
) -> DbResult<AlertSourceRow>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "UPDATE alert_sources SET \
             name = COALESCE($3, name), \
             board_id = COALESCE($4, board_id), \
             client_id = COALESCE($5, client_id), \
             correlation_window_secs = COALESCE($6, correlation_window_secs), \
             enabled = COALESCE($7, enabled), \
             event_type = COALESCE($8, event_type), \
             payload_schema_ref = COALESCE($9, payload_schema_ref), \
             updated_at = now() \
         WHERE id = $1 AND org_id = $2 \
         RETURNING {SOURCE_COLS}"
    );
    sqlx::query_as::<_, AlertSourceRow>(&sql)
        .bind(id)
        .bind(org_id)
        .bind(p.name)
        .bind(p.board_id)
        .bind(p.client_id)
        .bind(p.correlation_window_secs)
        .bind(p.enabled)
        .bind(p.event_type)
        .bind(p.payload_schema_ref)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)
}

/// The pre-auth lookup row: everything HMAC verification and org binding
/// need, resolved via the narrow SECURITY DEFINER function (0014 seam
/// pattern) because the request cannot know its org before auth.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AlertSourceAuth {
    pub r_id: Uuid,
    pub r_org_id: Uuid,
    pub r_source_ref: String,
    pub r_board_id: Uuid,
    pub r_client_id: Option<Uuid>,
    pub r_secret_encrypted: Vec<u8>,
    pub r_correlation_window_secs: i32,
    pub r_enabled: bool,
    pub r_event_type: Option<String>,
    pub r_payload_schema_ref: Option<String>,
}

pub async fn auth_lookup_source<'e, E>(
    executor: E,
    source_ref: &str,
) -> DbResult<Option<AlertSourceAuth>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query_as::<_, AlertSourceAuth>(
        "SELECT r_id, r_org_id, r_source_ref, r_board_id, r_client_id, r_secret_encrypted, \
                r_correlation_window_secs, r_enabled, r_event_type, r_payload_schema_ref \
         FROM alert_source_auth_lookup($1)",
    )
    .bind(source_ref)
    .fetch_optional(executor)
    .await
    .map_err(Into::into)
}

// --- alerts ------------------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AlertRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub source_id: Uuid,
    pub dedup_key: String,
    pub correlation_key: String,
    pub title: String,
    pub description: Option<String>,
    pub severity: String,
    pub occurred_at: DateTime<Utc>,
    pub attributes: Value,
    pub incident_ticket_id: Option<Uuid>,
    pub received_at: DateTime<Utc>,
    /// Triage state (0089): "open" | "acknowledged" | "resolved".
    pub status: String,
    pub acked_by: Option<Uuid>,
    pub acked_at: Option<DateTime<Utc>>,
    pub resolved_by: Option<Uuid>,
    pub resolved_at: Option<DateTime<Utc>>,
}

const ALERT_COLS: &str = "id, org_id, source_id, dedup_key, correlation_key, title, description, \
     severity, occurred_at, attributes, incident_ticket_id, received_at, \
     status, acked_by, acked_at, resolved_by, resolved_at";
const ALERT_COLS_FOR_SELECT: &str = "SELECT id, org_id, source_id, dedup_key, correlation_key, \
     title, description, severity, occurred_at, attributes, incident_ticket_id, received_at, \
     status, acked_by, acked_at, resolved_by, resolved_at";

pub struct NewAlert<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub source_id: Uuid,
    pub dedup_key: &'a str,
    pub correlation_key: &'a str,
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub severity: &'a str,
    pub occurred_at: DateTime<Utc>,
    pub attributes: &'a Value,
}

/// Inserts one alert. `Ok(None)` means the (org, source, dedup_key)
/// replay gate caught a duplicate — nothing was written and the caller
/// must answer with the stored row instead.
pub async fn insert_alert<'e, E>(executor: E, a: NewAlert<'_>) -> DbResult<Option<AlertRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "INSERT INTO alerts (id, org_id, source_id, dedup_key, correlation_key, title, \
                             description, severity, occurred_at, attributes) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
         ON CONFLICT (org_id, source_id, dedup_key) DO NOTHING \
         RETURNING {ALERT_COLS}"
    );
    sqlx::query_as::<_, AlertRow>(&sql)
        .bind(a.id)
        .bind(a.org_id)
        .bind(a.source_id)
        .bind(a.dedup_key)
        .bind(a.correlation_key)
        .bind(a.title)
        .bind(a.description)
        .bind(a.severity)
        .bind(a.occurred_at)
        .bind(a.attributes)
        .fetch_optional(executor)
        .await
        .map_err(Into::into)
}

/// The stored row for a replayed dedup_key (read AFTER a lost insert race).
pub async fn get_alert_by_dedup_key<'e, E>(
    executor: E,
    org_id: Uuid,
    source_id: Uuid,
    dedup_key: &str,
) -> DbResult<AlertRow>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "{ALERT_COLS_FOR_SELECT} FROM alerts \
         WHERE org_id = $1 AND source_id = $2 AND dedup_key = $3"
    );
    sqlx::query_as::<_, AlertRow>(&sql)
        .bind(org_id)
        .bind(source_id)
        .bind(dedup_key)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)
}

/// Newest-first browse for the alerts list endpoint.
#[derive(Debug, Default, Clone)]
pub struct AlertFilter {
    pub source_id: Option<Uuid>,
    pub ticket_id: Option<Uuid>,
    /// Triage filter (0089): "open" | "acknowledged" | "resolved" —
    /// None returns all states.
    pub status: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

pub async fn list_alerts<'e, E>(
    executor: E,
    org_id: Uuid,
    f: AlertFilter,
) -> DbResult<Vec<AlertRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "{ALERT_COLS_FOR_SELECT} FROM alerts \
         WHERE org_id = $1 \
           AND ($2::uuid IS NULL OR source_id = $2) \
           AND ($3::uuid IS NULL OR incident_ticket_id = $3) \
           AND ($4::text IS NULL OR status = $4) \
         ORDER BY received_at DESC, id DESC \
         LIMIT LEAST(GREATEST($5, 1), 200) OFFSET GREATEST($6, 0)"
    );
    sqlx::query_as::<_, AlertRow>(&sql)
        .bind(org_id)
        .bind(f.source_id)
        .bind(f.ticket_id)
        .bind(f.status)
        .bind(f.limit)
        .bind(f.offset)
        .fetch_all(executor)
        .await
        .map_err(Into::into)
}

/// One alert by id (RLS org-scoped). The ack/resolve routes re-read the
/// row FIRST so "already handled" is a typed conflict, not a silent
/// no-op and not a lost 404.
pub async fn get_alert<'e, E>(executor: E, org_id: Uuid, alert_id: Uuid) -> DbResult<AlertRow>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!("{ALERT_COLS_FOR_SELECT} FROM alerts WHERE org_id = $1 AND id = $2");
    sqlx::query_as::<_, AlertRow>(&sql)
        .bind(org_id)
        .bind(alert_id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)
}

/// Acknowledges an OPEN alert: status → "acknowledged" with the actor +
/// timestamp. `Ok(None)` = the row exists but is not open (already
/// handled) — the caller maps that to a typed conflict; row-not-found
/// surfaces as `DbError::NotFound` from `get_alert` beforehand.
pub async fn acknowledge_alert<'e, E>(
    executor: E,
    org_id: Uuid,
    alert_id: Uuid,
    actor_id: Uuid,
) -> DbResult<Option<AlertRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "UPDATE alerts SET status = 'acknowledged', acked_by = $3, acked_at = now() \
         WHERE org_id = $1 AND id = $2 AND status = 'open' \
         RETURNING {ALERT_COLS}"
    );
    sqlx::query_as::<_, AlertRow>(&sql)
        .bind(org_id)
        .bind(alert_id)
        .bind(actor_id)
        .fetch_optional(executor)
        .await
        .map_err(Into::into)
}

/// Decision-model auto-ack (Wave 23 lane 3): acknowledges an OPEN alert
/// above the alert-triage thresholds (ADR 0010 #4). The actor is
/// optional — the deploy's bot principal when configured, NULL
/// otherwise; decision-model provenance rides the decision_answers row
/// + request id, never a fabricated user.
pub async fn decision_auto_ack_alert<'e, E>(
    executor: E,
    org_id: Uuid,
    alert_id: Uuid,
    actor_id: Option<Uuid>,
) -> DbResult<Option<AlertRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "UPDATE alerts SET status = 'acknowledged', acked_by = $3, acked_at = now() \
         WHERE org_id = $1 AND id = $2 AND status = 'open' \
         RETURNING {ALERT_COLS}"
    );
    sqlx::query_as::<_, AlertRow>(&sql)
        .bind(org_id)
        .bind(alert_id)
        .bind(actor_id)
        .fetch_optional(executor)
        .await
        .map_err(Into::into)
}

/// Resolves an alert from ANY live state (open or acknowledged).
/// `Ok(None)` = the row exists but is already resolved — typed conflict
/// at the route. A resolve straight from `open` leaves `acked_*` NULL
/// (no acknowledgement ever happened — the row renders that honestly).
pub async fn resolve_alert<'e, E>(
    executor: E,
    org_id: Uuid,
    alert_id: Uuid,
    actor_id: Uuid,
) -> DbResult<Option<AlertRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "UPDATE alerts SET status = 'resolved', resolved_by = $3, resolved_at = now() \
         WHERE org_id = $1 AND id = $2 AND status IN ('open','acknowledged') \
         RETURNING {ALERT_COLS}"
    );
    sqlx::query_as::<_, AlertRow>(&sql)
        .bind(org_id)
        .bind(alert_id)
        .bind(actor_id)
        .fetch_optional(executor)
        .await
        .map_err(Into::into)
}

/// Links an alert to the incident ticket it folded into. Called once per
/// alert, inside the ingest transaction.
pub async fn link_alert_to_ticket<'e, E>(
    executor: E,
    alert_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(
        "UPDATE alerts SET incident_ticket_id = $2 WHERE id = $1 AND incident_ticket_id IS NULL",
    )
    .bind(alert_id)
    .bind(ticket_id)
    .execute(executor)
    .await?;
    Ok(())
}

// --- storm-collapse groups ---------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AlertIncidentRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub source_id: Uuid,
    pub correlation_key: String,
    pub ticket_id: Uuid,
    pub alert_count: i32,
    pub first_alert_at: DateTime<Utc>,
    pub last_alert_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}

const INCIDENT_COLS: &str = "id, org_id, source_id, correlation_key, ticket_id, alert_count, \
     first_alert_at, last_alert_at, closed_at";
const INCIDENT_COLS_FOR_SELECT: &str = "SELECT id, org_id, source_id, correlation_key, ticket_id, \
     alert_count, first_alert_at, last_alert_at, closed_at";

/// Locks (FOR UPDATE) the OPEN storm group for one correlation key, so
/// concurrent ingest deliveries serialize their collapse decision. The
/// partial unique index guarantees at most one row can come back.
pub async fn lock_open_incident<'e, E>(
    executor: E,
    org_id: Uuid,
    source_id: Uuid,
    correlation_key: &str,
) -> DbResult<Option<AlertIncidentRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "{INCIDENT_COLS_FOR_SELECT} FROM alert_incidents \
         WHERE org_id = $1 AND source_id = $2 AND correlation_key = $3 AND closed_at IS NULL \
         FOR UPDATE"
    );
    sqlx::query_as::<_, AlertIncidentRow>(&sql)
        .bind(org_id)
        .bind(source_id)
        .bind(correlation_key)
        .fetch_optional(executor)
        .await
        .map_err(Into::into)
}

/// Opens the storm group for a freshly anchored incident ticket.
pub async fn open_incident<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    source_id: Uuid,
    correlation_key: &str,
    ticket_id: Uuid,
    first_alert_at: DateTime<Utc>,
) -> DbResult<AlertIncidentRow>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "INSERT INTO alert_incidents (id, org_id, source_id, correlation_key, ticket_id, \
                                      alert_count, first_alert_at, last_alert_at) \
         VALUES ($1, $2, $3, $4, $5, 1, $6, $6) \
         RETURNING {INCIDENT_COLS}"
    );
    sqlx::query_as::<_, AlertIncidentRow>(&sql)
        .bind(id)
        .bind(org_id)
        .bind(source_id)
        .bind(correlation_key)
        .bind(ticket_id)
        .bind(first_alert_at)
        .fetch_one(executor)
        .await
        .map_err(Into::into)
}

/// Folds one more alert into the open group: bumps the counter and rolls
/// the window anchor forward. The row must already be locked by
/// [`lock_open_incident`].
pub async fn attach_alert_to_incident<'e, E>(
    executor: E,
    incident_id: Uuid,
    at: DateTime<Utc>,
) -> DbResult<AlertIncidentRow>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "UPDATE alert_incidents \
         SET alert_count = alert_count + 1, last_alert_at = $2 \
         WHERE id = $1 \
         RETURNING {INCIDENT_COLS}"
    );
    sqlx::query_as::<_, AlertIncidentRow>(&sql)
        .bind(incident_id)
        .bind(at)
        .fetch_one(executor)
        .await
        .map_err(Into::into)
}

/// Closes an expired group so a fresh incident can anchor. Must run on a
/// row already locked by [`lock_open_incident`].
pub async fn close_incident<'e, E>(executor: E, incident_id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query("UPDATE alert_incidents SET closed_at = now() WHERE id = $1")
        .bind(incident_id)
        .execute(executor)
        .await?;
    Ok(())
}

/// The group behind a ticket — used to answer replayed dedup_keys with
/// their collapse context.
pub async fn get_incident_by_ticket<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<Option<AlertIncidentRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "{INCIDENT_COLS_FOR_SELECT} FROM alert_incidents \
         WHERE org_id = $1 AND ticket_id = $2 \
         ORDER BY created_at DESC, id DESC LIMIT 1"
    );
    sqlx::query_as::<_, AlertIncidentRow>(&sql)
        .bind(org_id)
        .bind(ticket_id)
        .fetch_optional(executor)
        .await
        .map_err(Into::into)
}

// --- generic webhook ingest: replay gate + rate limit -------------------------

/// Records one ingest delivery. `Ok(true)` = first time for this dedup_key;
/// `Ok(false)` = the UNIQUE (org, source, dedup_key) fence caught a replay.
/// The insert is idempotent so at-least-once callers can retry safely.
pub async fn insert_webhook_dedup(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org_id: Uuid,
    source_id: Uuid,
    dedup_key: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
) -> DbResult<bool> {
    let res = sqlx::query!(
        r#"INSERT INTO webhook_ingest_dedup (org_id, source_id, dedup_key, occurred_at)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (org_id, source_id, dedup_key) DO NOTHING"#,
        org_id,
        source_id,
        dedup_key,
        occurred_at,
    )
    .execute(&mut **tx)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Counts ingestions for one source within the last `window_secs`. Used to
/// enforce a per-source rate ceiling at ingest time.
pub async fn count_recent_ingests<'e, E>(
    executor: E,
    org_id: Uuid,
    source_id: Uuid,
    window_secs: i32,
) -> DbResult<i64>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row: (i64,) = sqlx::query_as(
        r#"SELECT COUNT(*) FROM webhook_ingest_dedup
           WHERE org_id = $1 AND source_id = $2
             AND occurred_at > now() - ($3 || ' seconds')::interval"#,
    )
    .bind(org_id)
    .bind(source_id)
    .bind(window_secs)
    .fetch_one(executor)
    .await?;
    Ok(row.0)
}
