//! worker/csat — CSAT surveys on resolution (Wave 34 lane 2).
//!
//! Purpose: consume `ticket.closed` events via a durable consumer group and
//! run the org's survey policy: send-on-resolve toggle, per-client minimum
//! interval, quiet-hours-aware deferral. The survey row (intent + delivery
//! state + response) lives in `csat_surveys`; the raw response token exists
//! ONLY inside the emailed link — the row stores its SHA-256 hash. Mail
//! rendering + ledger enqueue go through core-api's internal surface, so
//! survey mail gets the branded wrapper, retries, and delivery state like
//! every other outbound message.
//!
//! None-mode honesty: when the org's mail transport resolves to `none` no
//! survey rows are created at all — scorecards keep `csat.available: false`
//! instead of showing a fake empty dataset (ADR 0010 posture). A transport
//! that disappears between queue and send marks the row `skipped` with the
//! typed `transport_not_configured` reason.
//!
//! Low-score automations: a submitted response emits `csat.response_recorded`
//! (core-api, portal route); rules key their `trigger_event` on it.

use core_domain::notifications::QuietHours;
use core_domain::settings as registry;
use core_events::{Envelope, EVENT_STREAM};
use redis::streams::{StreamReadOptions, StreamReadReply};
use redis::AsyncCommands;
use sqlx::PgPool;
use tokio::sync::watch;

const CONSUMER_GROUP: &str = "csat";
const CONSUMER_NAME: &str = "worker";
const BLOCK_MS: usize = 2_000;
const COUNT: usize = 50;
/// Deferred-survey sweep cadence: quiet windows end without an event.

#[derive(Debug, Clone)]
pub struct CsatLoopConfig {
    pub core_api_url: String,
    pub internal_token: Option<String>,
}

pub async fn run_csat(
    pool: PgPool,
    redis_url: String,
    config: CsatLoopConfig,
    mut shutdown: watch::Receiver<bool>,
) {
    let client = match redis::Client::open(redis_url.as_str()) {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(%error, "invalid redis url; csat loop exiting");
            return;
        }
    };
    let mut backoff_ms = 100_u64;
    loop {
        if *shutdown.borrow() {
            return;
        }
        match client.get_multiplexed_async_connection().await {
            Ok(mut conn) => {
                backoff_ms = 100;
                if let Err(error) = conn
                    .xgroup_create_mkstream::<&str, &str, &str, ()>(
                        EVENT_STREAM,
                        CONSUMER_GROUP,
                        "$",
                    )
                    .await
                {
                    let msg = error.to_string();
                    if !msg.contains("BUSYGROUP") {
                        tracing::error!(%error, "csat xgroup create failed");
                        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                        backoff_ms = (backoff_ms * 2).min(5_000);
                        continue;
                    }
                }
                follow_stream(&pool, &config, &mut conn, &mut shutdown).await;
                tracing::warn!("csat stream connection ended; reconnecting");
            }
            Err(error) => {
                tracing::error!(%error, "csat redis connect failed");
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
        backoff_ms = (backoff_ms * 2).min(5_000);
    }
}

/// Recovers this consumer's pending entries, then follows live ones.
/// Between batches it runs the deferred-send sweep: pending surveys whose
/// quiet window has ended (and crash-interrupted sends) go out here.
async fn follow_stream(
    pool: &PgPool,
    config: &CsatLoopConfig,
    conn: &mut redis::aio::MultiplexedConnection,
    shutdown: &mut watch::Receiver<bool>,
) {
    let options = StreamReadOptions::default().count(COUNT);
    if let Ok(reply) = conn
        .xread_options::<_, _, StreamReadReply>(&[EVENT_STREAM], &["0"], &options)
        .await
    {
        for entry in reply.keys.into_iter().flat_map(|k| k.ids) {
            handle_entry(pool, config, conn, &entry).await;
        }
    }

    let options = StreamReadOptions::default()
        .block(BLOCK_MS)
        .count(COUNT)
        .group(CONSUMER_GROUP, CONSUMER_NAME);
    loop {
        if *shutdown.borrow() {
            return;
        }
        tokio::select! {
            _ = shutdown.changed() => return,
            result = conn.xread_options::<_, _, StreamReadReply>(&[EVENT_STREAM], &[">"], &options) => {
                match result {
                    Ok(reply) => {
                        for entry in reply.keys.into_iter().flat_map(|k| k.ids) {
                            handle_entry(pool, config, conn, &entry).await;
                        }
                    }
                    Err(error) => {
                        tracing::error!(%error, "csat xreadgroup failed");
                        return;
                    }
                }
            }
        }
        // Consumer backlog: PEL depth, lag and the age of the oldest unacked
        // entry for THIS group, read after the batch (TASKLIST R7a).
        crate::stream_health::probe(conn, CONSUMER_GROUP).await;
        if let Err(error) = sweep_due(pool, config).await {
            tracing::error!(%error, "csat deferred sweep failed");
        }
    }
}

async fn handle_entry(
    pool: &PgPool,
    config: &CsatLoopConfig,
    conn: &mut redis::aio::MultiplexedConnection,
    entry: &redis::streams::StreamId,
) {
    let Some(raw) = entry.get::<String>("data") else {
        tracing::warn!(entry_id = %entry.id, "stream entry missing data field");
        return;
    };
    let Ok(envelope) = serde_json::from_str::<Envelope>(&raw) else {
        tracing::warn!(entry_id = %entry.id, "unparseable event; acking to skip");
        ack(conn, &entry.id).await;
        return;
    };
    if envelope.event_type != core_domain::events::types::TICKET_CLOSED {
        ack(conn, &entry.id).await;
        return;
    }
    let Some(ticket_id) = ticket_id_of(&envelope) else {
        ack(conn, &entry.id).await;
        return;
    };
    match process_closed(pool, config, envelope.org_id, ticket_id).await {
        Ok(_) => ack(conn, &entry.id).await,
        Err(error) => {
            // NOT acking: the group redelivers after restart, so a
            // transient DB failure retries by design.
            tracing::error!(%error, entry_id = %entry.id, "csat queue failed; leaving unacked");
        }
    }
}

fn ticket_id_of(envelope: &Envelope) -> Option<uuid::Uuid> {
    envelope
        .payload
        .get("ticket_id")
        .and_then(|v| v.as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
}

async fn ack(conn: &mut redis::aio::MultiplexedConnection, id: &str) {
    let _: Result<i64, _> = conn.xack(EVENT_STREAM, CONSUMER_GROUP, &[id]).await;
}

// --- policy ----------------------------------------------------------------

async fn org_bool(pool: &PgPool, org_id: uuid::Uuid, key: &str, default: bool) -> bool {
    match core_db::settings::get_org_value(pool, org_id, key).await {
        Ok(Some(value)) => value.as_bool().unwrap_or(default),
        _ => default,
    }
}

async fn org_int(pool: &PgPool, org_id: uuid::Uuid, key: &str, default: i64) -> i64 {
    match core_db::settings::get_org_value(pool, org_id, key).await {
        Ok(Some(value)) => value.as_i64().unwrap_or(default),
        _ => default,
    }
}

async fn org_text(pool: &PgPool, org_id: uuid::Uuid, key: &str, default: &str) -> String {
    match core_db::settings::get_org_value(pool, org_id, key).await {
        Ok(Some(value)) => value
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| default.to_string()),
        _ => default.to_string(),
    }
}

/// The org's effective quiet-hours window for surveys (empty = none).
async fn org_quiet(pool: &PgPool, org_id: uuid::Uuid, tz: &str) -> Option<QuietHours> {
    let from = org_text(pool, org_id, registry::CSAT_QUIET_FROM, "").await;
    let until = org_text(pool, org_id, registry::CSAT_QUIET_UNTIL, "").await;
    QuietHours::parse(Some(&from), Some(&until), tz)
        .ok()
        .flatten()
}

/// When quiet hours are active, defer to the window's next end boundary;
/// otherwise send immediately. The window model owns the timezone/DST math.
fn defer_until(
    quiet: &QuietHours,
    now: chrono::DateTime<chrono::Utc>,
) -> chrono::DateTime<chrono::Utc> {
    if !quiet.is_quiet(now) {
        return now;
    }
    quiet.next_window_end(now)
}

// --- pipeline --------------------------------------------------------------

/// The full on-resolve decision for one ticket. Returns the survey status.
pub(crate) async fn process_closed(
    pool: &PgPool,
    config: &CsatLoopConfig,
    org_id: uuid::Uuid,
    ticket_id: uuid::Uuid,
) -> anyhow::Result<&'static str> {
    // Master toggle first: off means nothing, not even an audit row.
    if !org_bool(pool, org_id, registry::CSAT_SEND_ON_RESOLVE, false).await {
        return Ok("off");
    }
    // None-mode honesty: no transport ⇒ no surveys, no rows.
    if mail_transport_unavailable(pool, org_id).await {
        return Ok("transport_not_configured");
    }
    let Some((client_id, contact_email)) = survey_recipient(pool, org_id, ticket_id).await? else {
        return Ok("no_recipient");
    };

    // Per-client minimum interval: a recent survey suppresses this one, and
    // the skip is LEDGERED so operators see the policy working.
    let min_interval_hours = org_int(pool, org_id, registry::CSAT_MIN_INTERVAL_HOURS, 168).await;
    if min_interval_hours > 0 {
        if let Some(client_id) = client_id {
            if let Some(last) = core_db::csat::last_sent_for_client(pool, org_id, client_id).await?
            {
                let cutoff = chrono::Utc::now() - chrono::Duration::hours(min_interval_hours);
                if last > cutoff {
                    let (_, hash) = new_token();
                    let ttl_days = org_int(pool, org_id, registry::CSAT_TOKEN_TTL_DAYS, 14).await;
                    let survey_id = core_db::csat::queue(
                        pool,
                        org_id,
                        ticket_id,
                        Some(client_id),
                        &contact_email,
                        &hash,
                        chrono::Utc::now() + chrono::Duration::days(ttl_days),
                        chrono::Utc::now(),
                    )
                    .await?;
                    core_db::csat::mark_skipped(pool, org_id, survey_id, "min_interval").await?;
                    return Ok("skipped_min_interval");
                }
            }
        }
    }

    let (raw, hash) = new_token();
    let ttl_days = org_int(pool, org_id, registry::CSAT_TOKEN_TTL_DAYS, 14).await;
    let tz_name = org_text(pool, org_id, registry::GENERAL_DEFAULT_TIMEZONE, "UTC").await;
    let quiet = org_quiet(pool, org_id, &tz_name).await;
    let now = chrono::Utc::now();
    let scheduled_for = match &quiet {
        Some(q) => defer_until(q, now),
        None => now,
    };
    let survey_id = core_db::csat::queue(
        pool,
        org_id,
        ticket_id,
        client_id,
        &contact_email,
        &hash,
        chrono::Utc::now() + chrono::Duration::days(ttl_days),
        scheduled_for,
    )
    .await?;
    if scheduled_for <= chrono::Utc::now() {
        request_send(
            pool,
            config,
            org_id,
            survey_id,
            ticket_id,
            &raw,
            &contact_email,
        )
        .await?;
    }
    Ok("queued")
}

/// Sends every pending survey whose scheduled time has arrived (quiet
/// windows end without an event; this also recovers crash-interrupted
/// sends). Transport resolution is re-checked so a config removal marks the
/// row skipped instead of looping.
pub(crate) async fn sweep_due(pool: &PgPool, config: &CsatLoopConfig) -> anyhow::Result<usize> {
    let due = core_db::csat::due(pool, chrono::Utc::now(), 200).await?;
    let mut sent = 0_usize;
    for survey in due {
        if mail_transport_unavailable(pool, survey.org_id).await {
            core_db::csat::mark_skipped(pool, survey.org_id, survey.id, "transport_not_configured")
                .await?;
            continue;
        }
        // The raw token is gone after the queueing process exits — a crash
        // between queue and send loses THIS link's usability by design
        // (tokens are single-use, unverifiable from the hash). Regenerate:
        // the sweep re-issues the link only for rows that were never sent,
        // which is safe precisely because nothing was ever delivered.
        let (raw, hash) = new_token();
        sqlx::query(
            "UPDATE csat_surveys SET token_hash = $3 WHERE org_id = $1 AND id = $2 AND status = 'pending'",
        )
        .bind(survey.org_id)
        .bind(survey.id)
        .bind(hash)
        .execute(pool)
        .await?;
        match request_send(
            pool,
            config,
            survey.org_id,
            survey.id,
            survey.ticket_id,
            &raw,
            &survey.contact_email,
        )
        .await
        {
            Ok(()) => sent += 1,
            Err(error) => {
                tracing::error!(%error, survey_id = %survey.id, "csat deferred send failed")
            }
        }
    }
    Ok(sent)
}

/// Renders + enqueues through core-api, which also flips the row to `sent`
/// in the same transaction — row state and ledger state never diverge.
async fn request_send(
    pool: &PgPool,
    config: &CsatLoopConfig,
    org_id: uuid::Uuid,
    survey_id: uuid::Uuid,
    ticket_id: uuid::Uuid,
    raw_token: &str,
    contact_email: &str,
) -> anyhow::Result<()> {
    let Some(token) = config.internal_token.as_deref() else {
        // Fail-closed parity with the email loop: without the shared
        // internal token nothing enqueues; the row stays pending.
        anyhow::bail!("internal token unset; csat mail stays queued");
    };
    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "{}/api/internal/email/csat",
            config.core_api_url.trim_end_matches('/')
        ))
        .header("X-Internal-Token", token)
        .json(&serde_json::json!({
            "org_id": org_id,
            "survey_id": survey_id,
            "ticket_id": ticket_id,
            "contact_email": contact_email,
            "token": raw_token,
        }))
        .send()
        .await?
        .error_for_status()?
        .json::<serde_json::Value>()
        .await?;
    if response["outcome"] == "queued" {
        let now = chrono::Utc::now();
        core_db::csat::mark_sent(pool, org_id, survey_id, now).await?;
    }
    Ok(())
}

/// Transport resolution, worker-side: mirror of the mail_config seam. The
/// authoritative per-org resolution still happens in core-api at enqueue
/// time; this check only decides whether to create/defer rows at all.
async fn mail_transport_unavailable(pool: &PgPool, org_id: uuid::Uuid) -> bool {
    match core_db::settings::get_org_value(pool, org_id, registry::EMAIL_TRANSPORT).await {
        Ok(Some(value)) => value.as_str() == Some("none"),
        _ => {
            // No override: the env fallback decides. Reading the worker's
            // own config here would couple the module to boot config, so
            // the honest default is "attempt and let core-api decide".
            false
        }
    }
}

async fn survey_recipient(
    pool: &PgPool,
    org_id: uuid::Uuid,
    ticket_id: uuid::Uuid,
) -> anyhow::Result<Option<(Option<uuid::Uuid>, String)>> {
    let row = sqlx::query_as::<_, (Option<uuid::Uuid>, Option<String>, Option<String>)>(
        r#"SELECT t.client_id, c.email AS contact_email,
                  cc.email AS client_contact_email
           FROM tickets t
           LEFT JOIN contacts c ON c.org_id = t.org_id AND c.id = t.contact_id
           LEFT JOIN LATERAL (
             SELECT email FROM contacts
             WHERE contacts.org_id = t.org_id
               AND t.client_id IS NOT NULL
               AND contacts.client_id = t.client_id
               AND email IS NOT NULL AND email <> ''
             ORDER BY created_at
             LIMIT 1
           ) cc ON true
           WHERE t.org_id = $1 AND t.id = $2"#,
    )
    .bind(org_id)
    .bind(ticket_id)
    .fetch_optional(pool)
    .await?;
    let Some((client_id, contact_email, client_contact_email)) = row else {
        return Ok(None);
    };
    // The ticket's own contact first, then the client's primary contact.
    let email = contact_email
        .filter(|e| !e.is_empty())
        .or(client_contact_email.filter(|e| !e.is_empty()));
    match email {
        Some(email) => Ok(Some((client_id, email))),
        None => Ok(None),
    }
}

/// 32 random bytes, hex; stored as SHA-256 hex (64 chars) — same shape the
/// auth one-use tokens use.
fn new_token() -> (String, String) {
    use rand::RngCore;
    use sha2::{Digest, Sha256};
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let raw = hex::encode(bytes);
    let hash = hex::encode(Sha256::digest(raw.as_bytes()));
    (raw, hash)
}
