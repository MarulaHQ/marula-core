//! core-db/dead_work — the operator's view of abandoned work, and the two
//! verbs that act on it.
//!
//! Purpose: read [`ops.dead_work`](crate) (migration 0144) — one row shape over
//! the four ledgers that can abandon a unit of work — and requeue or cancel one
//! row. Every statement here is org-scoped: the caller passes an
//! `app.org_id`-bound transaction and row-level security does the rest, which
//! is why there is no `org_id` filter to forget.
//!
//! Non-purpose: no audit rows (the route writes them, next to the actor it
//! authenticated), no scheduling (a redriven row is due now; the queue's own
//! claim decides when it runs), and no replay of a dead stream event — see
//! [`RedriveOutcome::NotReplayable`].

use crate::DbResult;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Which ledger a dead-work row came from. The wire vocabulary is this one:
/// `job`, `email`, `webhook`, `event`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeadWorkSource {
    /// A `jobs` row that is not a webhook delivery.
    Job,
    /// An `email_outbound` row.
    Email,
    /// A `webhook.deliver` job — the same table as [`Self::Job`], split out
    /// because the operator's question about it is different ("is the
    /// destination broken?") and because its subject needs the subscription's
    /// URL.
    Webhook,
    /// A stream entry whose handler exhausted its attempt budget
    /// (`dead_events`, R2).
    Event,
}

impl DeadWorkSource {
    pub const ALL: [Self; 4] = [Self::Job, Self::Email, Self::Webhook, Self::Event];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Job => "job",
            Self::Email => "email",
            Self::Webhook => "webhook",
            Self::Event => "event",
        }
    }

    /// Parses a path segment. Case-insensitive so a hand-typed URL works;
    /// an unknown token is `None` rather than a default, because defaulting a
    /// typo to `job` would redrive the wrong ledger.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let lower = value.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|s| s.as_str() == lower)
    }
}

impl std::fmt::Display for DeadWorkSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One row of `ops.dead_work`.
#[derive(Debug, Clone)]
pub struct DeadWorkRow {
    pub source: DeadWorkSource,
    /// The row id in that source — the argument the verbs take.
    pub id: Uuid,
    pub org_id: Option<Uuid>,
    /// What a human calls this: a job kind and ticket, an email subject and
    /// recipients, a webhook URL and event type, a consumer group and event
    /// type.
    pub subject: String,
    pub attempts: i32,
    pub last_error: Option<String>,
    /// When it was abandoned — not when it was enqueued.
    pub dead_since: DateTime<Utc>,
    /// Whether [`redrive`] can requeue it.
    pub replayable: bool,
    /// The cancellation decision, when an operator has made one.
    pub cancelled_at: Option<DateTime<Utc>>,
}

/// Filters for the list. `source = None` means every source.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeadWorkFilter {
    pub source: Option<DeadWorkSource>,
    /// Cancelled rows are hidden by default: the list exists to show work that
    /// still needs a decision. An operator reviewing past decisions asks for
    /// them explicitly.
    pub include_cancelled: bool,
    pub limit: i64,
    pub offset: i64,
}

/// Reads one page of the union, newest abandonment first.
///
/// A dead event whose envelope never parsed has no `org_id` and is therefore
/// invisible here: an org surface cannot own a row it cannot attribute. Those
/// entries are counted by the worker's own logs and, when R4 lands, by the
/// consumer-gap detector.
pub async fn list(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    org_id: Uuid,
    filter: DeadWorkFilter,
) -> DbResult<Vec<DeadWorkRow>> {
    let rows = sqlx::query(
        r#"SELECT source, id, org_id, subject, attempts, last_error, dead_since,
                  replayable, cancelled_at
             FROM ops.dead_work
            WHERE org_id = $1
              AND ($2::text IS NULL OR source = $2)
              AND ($3 OR cancelled_at IS NULL)
            ORDER BY dead_since DESC, id DESC
            LIMIT $4 OFFSET $5"#,
    )
    .bind(org_id)
    .bind(filter.source.map(DeadWorkSource::as_str))
    .bind(filter.include_cancelled)
    .bind(filter.limit.clamp(1, 500))
    .bind(filter.offset.max(0))
    .fetch_all(&mut **tx)
    .await?;
    rows.iter()
        .map(|r| {
            use sqlx::Row;
            let raw: String = r.get("source");
            Ok(DeadWorkRow {
                source: DeadWorkSource::parse(&raw).ok_or_else(|| {
                    crate::DbError::Conflict(format!("unknown dead-work source {raw:?}"))
                })?,
                id: r.get("id"),
                org_id: r.get("org_id"),
                subject: r.get("subject"),
                attempts: r.get("attempts"),
                last_error: r.get("last_error"),
                dead_since: r.get("dead_since"),
                replayable: r.get("replayable"),
                cancelled_at: r.get("cancelled_at"),
            })
        })
        .collect()
}

/// What a redrive did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedriveOutcome {
    /// The row is back in its ledger, due now, with a fresh attempt budget.
    Requeued,
    /// The row is no longer dead — someone redrove it, cancelled it, or (for a
    /// job) it is running again. Nothing changed, so nothing is audited.
    NotDead,
    /// No such row in this org.
    NotFound,
    /// The source cannot be replayed from this surface: a dead stream entry's
    /// payload is a raw stream body, and re-publishing needs the relay's stream
    /// writer. Refused rather than silently dropped.
    NotReplayable,
    /// An operator cancelled this row already. Cancel is terminal: reviving it
    /// would make the decision a suggestion, and the audit trail would say two
    /// contradictory things about one row.
    Cancelled,
}

/// Requeues one dead row in place.
///
/// In place, not as a new row, and that is the whole idempotency story:
///
/// * A webhook job keeps its `dedupe_key`, so the redrive cannot produce a
///   second delivery for `(webhook, event)` — a revival that inserted a new job
///   would either collide with the key and do nothing, or (without it) deliver
///   the same event twice.
/// * The attempt budget restarts at zero. `job.attempts >= job.max_attempts` is
///   what dead-letters a retry, so a redrive that left the counter at the limit
///   would re-dead on the first failure — a verb that only pretends.
/// * `WHERE status = 'dead'` makes a second redrive a no-op. Two operators
///   clicking at once produce one requeue and one [`Self::NotDead`], so the
///   caller writes one audit row rather than two for one action.
pub async fn redrive(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    org_id: Uuid,
    source: DeadWorkSource,
    id: Uuid,
) -> DbResult<RedriveOutcome> {
    match source {
        DeadWorkSource::Job | DeadWorkSource::Webhook => {
            // The source is authoritative: a `webhook` row is a
            // `webhook.deliver` job and a `job` row is anything else. Without
            // the kind predicate a script that guessed the source could requeue
            // a delivery as if it were a workflow wake, and the audit row would
            // name the wrong ledger.
            let res = sqlx::query(
                r#"UPDATE jobs
                      SET status = 'pending', attempts = 0, run_after = now(),
                          last_error = NULL, claimed_at = NULL, claimed_by = NULL,
                          updated_at = now()
                    WHERE id = $1 AND org_id = $2 AND status = 'dead'
                      AND (kind = 'webhook.deliver') = $3"#,
            )
            .bind(id)
            .bind(org_id)
            .bind(source == DeadWorkSource::Webhook)
            .execute(&mut **tx)
            .await?;
            if res.rows_affected() > 0 {
                Ok(RedriveOutcome::Requeued)
            } else if cancelled(tx, source, id).await? {
                Ok(RedriveOutcome::Cancelled)
            } else if job_exists(tx, org_id, id, source == DeadWorkSource::Webhook).await? {
                Ok(RedriveOutcome::NotDead)
            } else {
                Ok(RedriveOutcome::NotFound)
            }
        }
        DeadWorkSource::Email => {
            let res = sqlx::query(
                r#"UPDATE email_outbound
                      SET status = 'queued', attempts = 0, next_attempt_at = now(),
                          last_error = NULL, claimed_at = NULL, updated_at = now()
                    WHERE id = $1 AND org_id = $2 AND status = 'dead'"#,
            )
            .bind(id)
            .bind(org_id)
            .execute(&mut **tx)
            .await?;
            if res.rows_affected() > 0 {
                Ok(RedriveOutcome::Requeued)
            } else if cancelled(tx, source, id).await? {
                Ok(RedriveOutcome::Cancelled)
            } else if email_exists(tx, org_id, id).await? {
                Ok(RedriveOutcome::NotDead)
            } else {
                Ok(RedriveOutcome::NotFound)
            }
        }
        DeadWorkSource::Event => {
            // The row may exist, but this surface cannot requeue it: no stream
            // writer lives in core-api (§4.1 keeps the relay in the worker), and
            // an unparseable payload has no envelope to re-publish at all.
            if cancelled(tx, source, id).await? {
                Ok(RedriveOutcome::Cancelled)
            } else if event_exists(tx, org_id, id).await? {
                Ok(RedriveOutcome::NotReplayable)
            } else {
                Ok(RedriveOutcome::NotFound)
            }
        }
    }
}

async fn job_exists(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    org_id: Uuid,
    id: Uuid,
    webhook: bool,
) -> DbResult<bool> {
    let found: Option<(i64,)> = sqlx::query_as(
        "SELECT 1::bigint FROM jobs
          WHERE id = $1 AND org_id = $2 AND (kind = 'webhook.deliver') = $3",
    )
    .bind(id)
    .bind(org_id)
    .bind(webhook)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(found.is_some())
}

async fn email_exists(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<bool> {
    let found: Option<(i64,)> =
        sqlx::query_as("SELECT 1::bigint FROM email_outbound WHERE id = $1 AND org_id = $2")
            .bind(id)
            .bind(org_id)
            .fetch_optional(&mut **tx)
            .await?;
    Ok(found.is_some())
}

async fn event_exists(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<bool> {
    let found: Option<(i64,)> =
        sqlx::query_as("SELECT 1::bigint FROM dead_events WHERE id = $1 AND org_id = $2")
            .bind(id)
            .bind(org_id)
            .fetch_optional(&mut **tx)
            .await?;
    Ok(found.is_some())
}

/// What a cancel did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    Cancelled,
    /// An earlier cancellation stands; this call changed nothing and must not
    /// write a second audit row for the same decision.
    AlreadyCancelled,
    NotFound,
}

/// Records that an operator gave up on one dead row.
///
/// Two things happen, and they are deliberately separable. The cancellation
/// row is always written — that is the durable decision. Where the source's
/// ledger has a terminal state that means "abandoned", the work item moves into
/// it too, so the dead depth that an alert reads actually falls:
///
/// * `jobs` — `failed`, a status the queue has always declared and never
///   written. The dead counts (`queue::stats`, and the age metric beside it)
///   filter on `dead`, so a cancelled job stops firing them.
/// * `email_outbound` — the status moves to `cancelled` (0175). It used to stay
///   `dead` for want of a spare terminal value, which meant the operator's
///   decision never reached a count: the summary strip and any alert over the
///   dead bucket kept reporting mail nobody will retry (Wave 62 lane
///   `p62-data-truth`).
/// * `dead_events` — stays as it is; its redrive belongs to R4.
///
/// The parent rows are not deleted. The audit row is the record that survives
/// retention, and R6 prunes these tables on its own schedule.
pub async fn cancel(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    org_id: Uuid,
    source: DeadWorkSource,
    id: Uuid,
    reason: &str,
    actor_user_id: Option<Uuid>,
) -> DbResult<CancelOutcome> {
    // An existing decision answers first, whatever state the row is in now: the
    // operator's question is "did we already give up on this", and a cancelled
    // job has left the dead set so asking the row first would report "no such
    // work" about work we cancelled.
    if cancelled(tx, source, id).await? {
        return Ok(CancelOutcome::AlreadyCancelled);
    }

    // The row has to be dead work before the decision is recorded. Doing this
    // before the insert matters: a source or id that names nothing (a typo,
    // another org's row, a row already requeued) is refused rather than
    // recorded, so no caller can commit a decision about work it never saw.
    if !row_is_dead(tx, org_id, source, id).await? {
        return Ok(CancelOutcome::NotFound);
    }

    // The insert is also the lock: two operators at once reach it together, and
    // the loser's `ON CONFLICT` reports zero rows and moves nothing.
    let inserted = sqlx::query(
        r#"INSERT INTO dead_work_cancellations (source, id, org_id, reason, cancelled_by)
           VALUES ($1, $2, $3, $4, $5)
           ON CONFLICT (source, id) DO NOTHING"#,
    )
    .bind(source.as_str())
    .bind(id)
    .bind(org_id)
    .bind(reason)
    .bind(actor_user_id)
    .execute(&mut **tx)
    .await?;
    if inserted.rows_affected() == 0 {
        return Ok(CancelOutcome::AlreadyCancelled);
    }

    if matches!(source, DeadWorkSource::Email) {
        sqlx::query(
            r#"UPDATE email_outbound SET status = 'cancelled', updated_at = now()
                WHERE id = $1 AND org_id = $2 AND status = 'dead'"#,
        )
        .bind(id)
        .bind(org_id)
        .execute(&mut **tx)
        .await?;
    }
    if matches!(source, DeadWorkSource::Job | DeadWorkSource::Webhook) {
        sqlx::query(
            r#"UPDATE jobs SET status = 'failed', claimed_at = NULL, claimed_by = NULL,
                               updated_at = now()
                WHERE id = $1 AND org_id = $2 AND status = 'dead'
                  AND (kind = 'webhook.deliver') = $3"#,
        )
        .bind(id)
        .bind(org_id)
        .bind(source == DeadWorkSource::Webhook)
        .execute(&mut **tx)
        .await?;
    }
    // Dead events keep their ledger state; the cancellation row, plus the
    // `replayable` column 0176 derives from it, is the whole record.
    Ok(CancelOutcome::Cancelled)
}

/// Whether an operator has already given up on this row.
async fn cancelled(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    source: DeadWorkSource,
    id: Uuid,
) -> DbResult<bool> {
    let found: Option<(i64,)> = sqlx::query_as(
        "SELECT 1::bigint FROM dead_work_cancellations WHERE source = $1 AND id = $2",
    )
    .bind(source.as_str())
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(found.is_some())
}

/// Whether this id names a DEAD row of this source in this org.
///
/// "Dead" is the union's own predicate, not mere existence: the verbs act on
/// what the list shows, so a pending job or a sent email is not cancellable
/// through this surface and a mismatched source (a webhook delivery named as a
/// plain job) is refused rather than acted on.
async fn row_is_dead(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    org_id: Uuid,
    source: DeadWorkSource,
    id: Uuid,
) -> DbResult<bool> {
    let found: Option<(i64,)> = match source {
        DeadWorkSource::Job | DeadWorkSource::Webhook => {
            sqlx::query_as(
                "SELECT 1::bigint FROM jobs
              WHERE id = $1 AND org_id = $2 AND status = 'dead'
                AND (kind = 'webhook.deliver') = $3",
            )
            .bind(id)
            .bind(org_id)
            .bind(source == DeadWorkSource::Webhook)
            .fetch_optional(&mut **tx)
            .await?
        }
        DeadWorkSource::Email => {
            sqlx::query_as(
                "SELECT 1::bigint FROM email_outbound
              WHERE id = $1 AND org_id = $2 AND status = 'dead'",
            )
            .bind(id)
            .bind(org_id)
            .fetch_optional(&mut **tx)
            .await?
        }
        DeadWorkSource::Event => {
            sqlx::query_as("SELECT 1::bigint FROM dead_events WHERE id = $1 AND org_id = $2")
                .bind(id)
                .bind(org_id)
                .fetch_optional(&mut **tx)
                .await?
        }
    };
    Ok(found.is_some())
}

/// One webhook destination as the operator surface reads it.
#[derive(Debug, Clone)]
pub struct DestinationRow {
    pub id: Uuid,
    pub url: String,
    pub enabled: bool,
    pub consecutive_failures: i32,
    pub paused_at: Option<DateTime<Utc>>,
    pub paused_reason: Option<String>,
}

/// Webhook destinations, paused ones first, then the ones closest to pausing.
///
/// `only_paused` is for the operator who wants the list of what is currently
/// suppressed; the full list is for the one deciding whether a destination is
/// about to join it, which is why the counter is on the row.
pub async fn list_destinations(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    org_id: Uuid,
    only_paused: bool,
) -> DbResult<Vec<DestinationRow>> {
    let rows = sqlx::query(
        r#"SELECT id, url, enabled, consecutive_failures, paused_at, paused_reason
             FROM webhooks
            WHERE org_id = $1 AND (NOT $2 OR paused_at IS NOT NULL)
            ORDER BY paused_at DESC NULLS LAST, consecutive_failures DESC, created_at DESC"#,
    )
    .bind(org_id)
    .bind(only_paused)
    .fetch_all(&mut **tx)
    .await?;
    use sqlx::Row;
    Ok(rows
        .iter()
        .map(|r| DestinationRow {
            id: r.get("id"),
            url: r.get("url"),
            enabled: r.get("enabled"),
            consecutive_failures: r.get("consecutive_failures"),
            paused_at: r.get("paused_at"),
            paused_reason: r.get("paused_reason"),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_source_round_trips_through_its_wire_token() {
        for source in DeadWorkSource::ALL {
            assert_eq!(DeadWorkSource::parse(source.as_str()), Some(source));
            assert_eq!(
                DeadWorkSource::parse(&source.as_str().to_ascii_uppercase()),
                Some(source)
            );
        }
    }

    #[test]
    fn an_unknown_source_is_refused_rather_than_defaulted() {
        // A typo must not requeue the wrong ledger: `jobs` and `email` share no
        // ids, but the operator's intent is about one row, not any row.
        for bad in ["", "jobs", "events", "webhooks", "mail"] {
            assert_eq!(DeadWorkSource::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_wire_tokens_are_the_ones_the_view_emits() {
        // The migration's CHECK and the view's literals use these exact words;
        // a rename here without one there would make every row unreadable.
        assert_eq!(
            DeadWorkSource::ALL.map(DeadWorkSource::as_str),
            ["job", "email", "webhook", "event"]
        );
    }
}
