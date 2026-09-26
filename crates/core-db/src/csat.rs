//! CSAT surveys on resolution (Wave 34 lane 2, `p34-csat-surveys`).
//!
//! One row per resolved ticket: queued intent → sent (tokenized email) →
//! responded (single-use token consumed) — or honestly skipped. The portal
//! consume path matches by token HASH, so a leaked database row never yields
//! a usable link. Rollup queries window on `responded_at` the same way the
//! scorecards module windows on `resolved_at`.

use sqlx::PgPool;
use uuid::Uuid;

use crate::DbResult;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CsatSurveyRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub ticket_id: Uuid,
    pub client_id: Option<Uuid>,
    pub contact_email: String,
    pub token_hash: String,
    pub token_expires_at: chrono::DateTime<chrono::Utc>,
    pub status: String,
    pub skip_reason: Option<String>,
    pub scheduled_for: chrono::DateTime<chrono::Utc>,
    pub sent_at: Option<chrono::DateTime<chrono::Utc>>,
    pub score: Option<i16>,
    pub comment: Option<String>,
    pub responded_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Queue one survey on a resolved ticket. Idempotent per ticket: the UNIQUE
/// (org_id, ticket_id) makes a replayed resolve a no-op (ON CONFLICT DO
/// NOTHING), so at-least-once event delivery cannot double-send.
#[allow(clippy::too_many_arguments)]
pub async fn queue(
    pool: &PgPool,
    org_id: Uuid,
    ticket_id: Uuid,
    client_id: Option<Uuid>,
    contact_email: &str,
    token_hash: &str,
    token_expires_at: chrono::DateTime<chrono::Utc>,
    scheduled_for: chrono::DateTime<chrono::Utc>,
) -> DbResult<Uuid> {
    let id = Uuid::now_v7();
    let inserted: Option<Uuid> = sqlx::query_scalar!(
        r#"INSERT INTO csat_surveys
             (id, org_id, ticket_id, client_id, contact_email,
              token_hash, token_expires_at, scheduled_for)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           ON CONFLICT (org_id, ticket_id) DO NOTHING
           RETURNING id"#,
        id,
        org_id,
        ticket_id,
        client_id,
        contact_email,
        token_hash,
        token_expires_at,
        scheduled_for,
    )
    .fetch_optional(pool)
    .await?;
    match inserted {
        Some(id) => Ok(id),
        None => {
            // Replay: return the existing survey's id so callers settle the
            // row they actually have.
            sqlx::query_scalar!(
                "SELECT id FROM csat_surveys WHERE org_id = $1 AND ticket_id = $2",
                org_id,
                ticket_id
            )
            .fetch_one(pool)
            .await
            .map_err(Into::into)
        }
    }
}

/// When did this client last receive (or respond to) a survey? Drives the
/// per-client minimum-interval policy.
pub async fn last_sent_for_client(
    pool: &PgPool,
    org_id: Uuid,
    client_id: Uuid,
) -> DbResult<Option<chrono::DateTime<chrono::Utc>>> {
    let row = sqlx::query_scalar!(
        r#"SELECT MAX(COALESCE(sent_at, created_at))
           FROM csat_surveys
           WHERE org_id = $1 AND client_id = $2
             AND status IN ('pending', 'sent', 'responded')"#,
        org_id,
        client_id,
    )
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Surveys due to send (queued, past their scheduled time), oldest first.
pub async fn due(
    pool: &PgPool,
    now: chrono::DateTime<chrono::Utc>,
    cap: i64,
) -> DbResult<Vec<CsatSurveyRow>> {
    let rows = sqlx::query_as!(
        CsatSurveyRow,
        r#"SELECT id, org_id, ticket_id, client_id, contact_email, token_hash,
                  token_expires_at, status, skip_reason, scheduled_for, sent_at,
                  score, comment, responded_at, created_at
           FROM csat_surveys
           WHERE status = 'pending' AND scheduled_for <= $1
           ORDER BY scheduled_for
           LIMIT $2"#,
        now,
        cap,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// One survey row by id.
pub async fn get<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<Option<CsatSurveyRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query_as!(
        CsatSurveyRow,
        r#"SELECT id, org_id, ticket_id, client_id, contact_email, token_hash,
                  token_expires_at, status, skip_reason, scheduled_for, sent_at,
                  score, comment, responded_at, created_at
           FROM csat_surveys WHERE org_id = $1 AND id = $2"#,
        org_id,
        id,
    )
    .fetch_optional(executor)
    .await?;
    Ok(row)
}

/// A due survey whose transport vanished mid-sweep: record the typed
/// not-configured state instead of looping forever on a `none` transport.
pub async fn mark_skipped(pool: &PgPool, org_id: Uuid, id: Uuid, reason: &str) -> DbResult<()> {
    sqlx::query!(
        r#"UPDATE csat_surveys
           SET status = 'skipped', skip_reason = $3, updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        id,
        reason,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Mark sent (single-use token delivered). Returns the id only when the
/// row actually transitioned (`pending` → `sent`) — the internal enqueue
/// handler uses that to keep retries inert, so row state and ledger state
/// never diverge.
pub async fn mark_sent<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    sent_at: chrono::DateTime<chrono::Utc>,
) -> DbResult<Option<Uuid>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query!(
        r#"UPDATE csat_surveys
           SET status = 'sent', sent_at = $3, updated_at = now()
           WHERE org_id = $1 AND id = $2 AND status = 'pending'
           RETURNING id"#,
        org_id,
        id,
        sent_at,
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|r| r.id))
}

/// The portal GET (pre-submit) view: same matching rules as `consume` but
/// read-only, so rendering the form never burns the token.
pub async fn peek<'e, E>(
    executor: E,
    token_hash: &str,
    org_id: Uuid,
    client_id: Uuid,
    now: chrono::DateTime<chrono::Utc>,
) -> DbResult<Option<CsatSurveyRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query_as!(
        CsatSurveyRow,
        r#"SELECT id, org_id, ticket_id, client_id, contact_email, token_hash,
                  token_expires_at, status, skip_reason, scheduled_for, sent_at,
                  score, comment, responded_at, created_at
           FROM csat_surveys
           WHERE org_id = $1 AND token_hash = $2
             AND client_id = $3
             AND status IN ('sent', 'responded')
             AND token_expires_at > $4"#,
        org_id,
        token_hash,
        client_id,
        now,
    )
    .fetch_optional(executor)
    .await?;
    Ok(row)
}

/// The single-use token consume path: match by hash, require unexpired and
/// unconsumed. `None` covers used/expired/unknown alike — the portal route
/// answers one honest "this link is invalid, used, or expired".
pub struct ConsumeAttempt {
    pub survey_id: Uuid,
    pub org_id: Uuid,
    pub ticket_id: Uuid,
    pub client_id: Option<Uuid>,
    pub token_expires_at: chrono::DateTime<chrono::Utc>,
}

pub async fn consume<'e, E>(
    executor: E,
    token_hash: &str,
    org_id: Uuid,
    client_id: Uuid,
    now: chrono::DateTime<chrono::Utc>,
) -> DbResult<Option<ConsumeAttempt>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query!(
        r#"UPDATE csat_surveys
           SET status = 'responded', responded_at = $4, updated_at = now()
           WHERE org_id = $1 AND token_hash = $2
             AND status = 'sent'
             AND token_expires_at > $4
             AND client_id = $3
           RETURNING id, org_id, ticket_id, client_id, token_expires_at"#,
        org_id,
        token_hash,
        client_id,
        now,
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|r| ConsumeAttempt {
        survey_id: r.id,
        org_id: r.org_id,
        ticket_id: r.ticket_id,
        client_id: r.client_id,
        token_expires_at: r.token_expires_at,
    }))
}

/// Store score + comment after the consume transition (survey row already
/// flipped to `responded`).
pub async fn record_response<'e, E>(
    executor: E,
    org_id: Uuid,
    survey_id: Uuid,
    score: i16,
    comment: Option<&str>,
) -> DbResult<Option<Uuid>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query!(
        r#"UPDATE csat_surveys
           SET score = $3, comment = $4, updated_at = now()
           WHERE org_id = $1 AND id = $2 AND status = 'responded' AND score IS NULL
           RETURNING ticket_id"#,
        org_id,
        survey_id,
        score,
        comment,
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|r| r.ticket_id))
}

/// The assignee of a survey's ticket at response time — used for the
/// per-tech rollup event payload. `None` when the ticket row vanished.
pub async fn ticket_assignee<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<Option<Option<Uuid>>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT assignee_id FROM tickets WHERE org_id = $1 AND id = $2"#,
        org_id,
        ticket_id,
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|r| r.assignee_id))
}

/// The client's primary contact email for survey delivery: first contact
/// with a non-empty address, oldest first (stable pick). `None` = no
/// contactable address — the survey is skipped, never guessed.
pub async fn primary_contact_email(
    pool: &PgPool,
    org_id: Uuid,
    client_id: Uuid,
) -> DbResult<Option<String>> {
    let email: Option<String> = sqlx::query_scalar!(
        r#"SELECT email FROM contacts
           WHERE org_id = $1 AND client_id = $2
             AND email IS NOT NULL AND email <> ''
           ORDER BY created_at
           LIMIT 1"#,
        org_id,
        client_id,
    )
    .fetch_optional(pool)
    .await?
    .flatten();
    Ok(email)
}

/// Whether any survey row exists for the org — the honest gate that flips
/// the scorecards `CsatSection.available` stub true.
pub async fn has_surveys<'e, E>(executor: E, org_id: Uuid) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let n = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM csat_surveys WHERE org_id = $1)"#,
        org_id,
    )
    .fetch_one(executor)
    .await?;
    Ok(n.unwrap_or(false))
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CsatUserRollupRow {
    pub user_id: Uuid,
    pub responses: i64,
    pub avg_score: Option<f64>,
    pub low_scores: i64,
}

/// Per-tech rollup over responses in the window (matched on the ticket's
/// assignee at response time — the same attribution the scorecards module
/// uses for resolution stats).
pub async fn rollup_for_users<'e, E>(
    executor: E,
    org_id: Uuid,
    user_ids: &[Uuid],
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    low_threshold: i16,
) -> DbResult<Vec<CsatUserRollupRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query_as!(
        CsatUserRollupRow,
        r#"SELECT t.assignee_id AS "user_id!",
                  COUNT(*)::bigint AS "responses!",
                  AVG(s.score)::float8 AS avg_score,
                  COUNT(*) FILTER (WHERE s.score <= $5)::bigint AS "low_scores!"
           FROM csat_surveys s
           JOIN tickets t ON t.org_id = s.org_id AND t.id = s.ticket_id
           WHERE s.org_id = $1
             AND t.assignee_id = ANY($2)
             AND s.responded_at IS NOT NULL
             AND s.responded_at >= $3
             AND s.responded_at < $4
           GROUP BY t.assignee_id"#,
        org_id,
        user_ids,
        from,
        to,
        low_threshold,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CsatClientRollupRow {
    pub client_id: Uuid,
    pub client_name: String,
    pub responses: i64,
    pub avg_score: Option<f64>,
    pub low_scores: i64,
}

/// Per-client rollup over responses in the window.
pub async fn rollup_for_clients<'e, E>(
    executor: E,
    org_id: Uuid,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    low_threshold: i16,
) -> DbResult<Vec<CsatClientRollupRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query_as!(
        CsatClientRollupRow,
        r#"SELECT s.client_id AS "client_id!",
                  c.name AS "client_name!",
                  COUNT(*)::bigint AS "responses!",
                  AVG(s.score)::float8 AS avg_score,
                  COUNT(*) FILTER (WHERE s.score <= $4)::bigint AS "low_scores!"
           FROM csat_surveys s
           JOIN clients c ON c.org_id = s.org_id AND c.id = s.client_id
           WHERE s.org_id = $1
             AND s.client_id IS NOT NULL
             AND s.responded_at IS NOT NULL
             AND s.responded_at >= $2
             AND s.responded_at < $3
           GROUP BY s.client_id, c.name"#,
        org_id,
        from,
        to,
        low_threshold,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}
