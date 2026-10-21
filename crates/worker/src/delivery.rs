//! worker/delivery — scheduled report delivery (the private build tracker R4).
//!
//! Purpose: claim due `report_schedules` rows, render the referenced R3
//! definition as CSV over the `ops.v1_*` read-model views, and mail it to
//! each recipient through the `MailTransport` seam — every recipient's
//! copy governed by their notification preference chain (kind
//! `report_delivery`: disabled channel and quiet hours both suppress).
//! Transport-unset deployments get the documented none mode: the run is
//! LEDGERED as `skipped` and logged, the schedule advances, and the loop
//! never fails — a missing mail provider must not wedge reporting.
//!
//! Delivery semantics: claiming and advancing happen in one transaction
//! (`FOR UPDATE SKIP LOCKED` + compare-set advance), so a slot fires
//! AT-MOST-ONCE per schedule row even with overlapping workers; a worker
//! crash between advance and send loses that slot's run, and the
//! append-only `report_deliveries` ledger makes every outcome — sent,
//! suppressed, failed, skipped — visible. Per-recipient send failures
//! never fail the run: they land in the ledger's detail counters and the
//! schedule's `last_error`.
//!
//! Report execution: the stored spec compiles via the compiler's
//! execution-lane seam (`ReportSpec::compile_windowed`, OrgWide —
//! schedules are admin-managed org-wide surface) and the run-time window
//! rides `ViewId::time_anchor` INSIDE the compiler, so the statement is
//! 100% compiler output with every value a bound parameter — exactly
//! like the interactive lanes, summary parity included.
//!
//! Non-purpose: no schedule CRUD (core-api owns the admin surface and
//! the schedule events), no cadence persistence (this module computes
//! the next slot from the row), no in-app delivery (the notification
//! inbox is not a report transport), no MIME attachments (the CSV rides
//! the transport's text body — the seam stays attachment-free).

use crate::config::DeliveryLoopConfig;
use chrono::{DateTime, Duration, Utc};
use core_domain::entities::{NotificationKind, User};
use core_domain::notifications::{decide_channel, Decision, DeliveryChannel, QuietHours};
use core_mail::{OutboundEmail, TransportKind};
use core_reports::{Bind, ReportSpec, Scope};
use sqlx::{Column, PgPool, Row};
use std::sync::Arc;
use std::time::Duration as StdDuration;
use tokio::sync::watch;
use uuid::Uuid;

/// Row cap per rendered report. Spec output is a single bounded page by
/// design; deliveries cap well above the interactive page size so a CSV
/// stays a CSV without becoming an export job.
pub const DELIVERY_ROW_CAP: i64 = 5_000;

pub use core_domain::cadence::{next_run_after, shape_of, Cadence, ScheduleShape};

// --- report execution -------------------------------------------------------------

/// Compiles the stored spec for one delivery slot: OrgWide scope (schedules
/// are admin-managed org-wide surface) with the run-time window applied on
/// the view's time anchor via the compiler's execution-lane seam — the
/// statement stays 100% compiler output, every value a bound parameter.
pub fn windowed_statement(
    spec: &ReportSpec,
    window_days: i32,
    slot: DateTime<Utc>,
) -> Result<(String, Vec<Bind>), core_reports::ReportsError> {
    let end = slot;
    let start = slot - Duration::days(i64::from(window_days));
    let compiled = spec.compile_windowed(
        Scope::OrgWide,
        DELIVERY_ROW_CAP,
        core_reports::RunWindow { start, end },
    )?;
    Ok((compiled.sql, compiled.binds))
}

/// Executes a windowed statement on the reporting pool inside an
/// org-bound transaction and renders the CSV. The reporting pool runs
/// the RLS-constrained `marula_app` role (same rewrite as core-api's
/// `AppState::reporting_pool`), so tenancy flows through the
/// security-invoker views plus the explicit `app.org_id` bind — never
/// through the worker's owner privileges.
async fn render_csv(
    reporting: &PgPool,
    sql: &str,
    binds: &[Bind],
    org_id: Uuid,
) -> Result<String, anyhow::Error> {
    let mut tx = reporting.begin().await?;
    core_db::set_org(&mut tx, org_id).await?;
    let mut q = sqlx::query(sql);
    for b in binds {
        q = match b {
            Bind::Uuid(u) => q.bind(*u),
            Bind::Text(s) => q.bind(s.as_str()),
            Bind::Date(d) => q.bind(*d),
            Bind::Timestamp(t) => q.bind(t),
            Bind::Int(n) => q.bind(*n),
            Bind::Bool(b) => q.bind(*b),
        };
    }
    // Compile carries the column layout; decode against the wrapper's
    // output (the subquery's aliases become the outer column names).
    let rows = q.fetch_all(&mut *tx).await?;
    let mut out = String::new();
    if let Some(first) = rows.first() {
        let header: Vec<String> = first.columns().iter().map(|c| csv_cell(c.name())).collect();
        out.push_str(&header.join(","));
        out.push('\n');
    }
    for row in &rows {
        let cells: Vec<String> = row
            .columns()
            .iter()
            .map(|col| {
                let name = col.name();
                let ty = col.type_info().to_string();
                csv_cell(&dynamic_cell(row, name, &ty))
            })
            .collect();
        out.push_str(&cells.join(","));
        out.push('\n');
    }
    // Read-only request: rollback is both correct and free.
    tx.rollback().await?;
    Ok(out)
}

/// Type-OID-driven cell rendering — the wrapper's output columns mirror
/// the compiler's declared types, but decoding dynamically off the
/// PG type name keeps this path correct even if the compiler's declared
/// OutType and the wire type ever drift.
fn dynamic_cell(row: &sqlx::postgres::PgRow, name: &str, ty: &str) -> String {
    let v: Option<String> = match ty {
        "UUID" => row
            .try_get::<Option<Uuid>, _>(name)
            .ok()
            .flatten()
            .map(|v| v.to_string()),
        "TEXT" | "NAME" | "VARCHAR" => row.try_get::<Option<String>, _>(name).ok().flatten(),
        "TIMESTAMPTZ" => row
            .try_get::<Option<DateTime<Utc>>, _>(name)
            .ok()
            .flatten()
            .map(|t| t.to_rfc3339()),
        "DATE" => row
            .try_get::<Option<chrono::NaiveDate>, _>(name)
            .ok()
            .flatten()
            .map(|d| d.to_string()),
        "INT4" => row
            .try_get::<Option<i32>, _>(name)
            .ok()
            .flatten()
            .map(|v| v.to_string()),
        "INT8" => row
            .try_get::<Option<i64>, _>(name)
            .ok()
            .flatten()
            .map(|v| v.to_string()),
        "FLOAT8" | "FLOAT4" | "NUMERIC" => row
            .try_get::<Option<f64>, _>(name)
            .ok()
            .flatten()
            .map(|v| v.to_string()),
        "BOOL" => row
            .try_get::<Option<bool>, _>(name)
            .ok()
            .flatten()
            .map(|v| v.to_string()),
        _ => Some(format!("undecodable:{ty}")),
    };
    v.unwrap_or_default()
}

/// CSV escaping with stable house semantics — identical quoting rules to
/// the interactive `?format=csv` lane (comma/quote/newline → quoted with
/// doubled inner quotes).
fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

// --- recipient fan-out --------------------------------------------------------------

/// One recipient's resolved fan-out plan: the user (for the email
/// address) plus the preference-chain decision for THIS slot.
pub struct RecipientPlan {
    pub user: User,
    pub decision: Decision,
}

/// Resolves one recipient through the FULL preference chain — user row →
/// org default → built-in — under the `report_delivery` kind, evaluating
/// quiet hours on the recipient's wall clock. Reports are never
/// urgency-class, so an active quiet window always defers the email leg
/// (exactly `decide_channel`'s non-critical semantics — reused verbatim).
pub async fn plan_recipient(
    pool: &PgPool,
    org_id: Uuid,
    user_id: Uuid,
    kind: NotificationKind,
    now: DateTime<Utc>,
) -> Option<RecipientPlan> {
    let user = core_db::users::get_user(pool, org_id, user_id).await.ok()?;
    let pref = core_db::notifications::effective_preference(pool, org_id, user_id, kind.as_str())
        .await
        .ok()?;
    let quiet = QuietHours::parse(
        pref.quiet_from.as_deref(),
        pref.quiet_until.as_deref(),
        &pref.timezone,
    )
    .ok()
    .flatten();
    Some(RecipientPlan {
        user,
        decision: decide_channel(
            DeliveryChannel::Email,
            kind,
            pref.email,
            quiet.as_ref(),
            now,
        ),
    })
}

// --- the delivery loop ---------------------------------------------------------------

/// Builds the RLS-constrained reporting pool the delivery path executes
/// on — the SAME credential rewrite core-api applies (`marula_app`
/// when the configured URL carries elevated creds).
fn reporting_target(database_url: &str) -> String {
    if database_url.contains("marula_app:") {
        return database_url.to_string();
    }
    let (scheme_rest, rest) = database_url
        .split_once("://")
        .unwrap_or(("postgres", database_url));
    match rest.split_once('@') {
        Some((_creds, host)) => format!("{scheme_rest}://marula_app:marula_app@{host}"),
        None => format!("{scheme_rest}://marula_app:marula_app@{rest}"),
    }
}

/// Runs the delivery loop until `shutdown` fires. The loop itself never
/// fails: poll errors are logged and retried next tick, per-schedule
/// failures are ledgered, and none-mode (transport unset) turns sends
/// into logged skips. Returns when told to stop.
pub async fn run_delivery_loop(
    pool: PgPool,
    database_url: String,
    transport: Arc<dyn core_mail::MailTransport>,
    transport_kind: TransportKind,
    config: DeliveryLoopConfig,
    mut shutdown: watch::Receiver<bool>,
) {
    // The reporting pool is built lazily: a deployment whose app role is
    // absent (minimal standalone installs) still gets claiming + skips.
    // The capacity travels with the pool (Wave 52 lane P9): this is the
    // worker's third pool, and the pooler-safe value is not inherited from
    // the app pool just because both live in one process.
    let reporting = match core_db::connect_options(
        &reporting_target(&database_url),
        config.statement_cache_capacity,
    ) {
        Ok(options) => sqlx::postgres::PgPoolOptions::new()
            .max_connections(config.pool_max)
            .acquire_timeout(StdDuration::from_secs(30))
            .connect_lazy_with(options),
        Err(error) => {
            tracing::error!(%error, "invalid reporting pool config; delivery loop exiting");
            return;
        }
    };
    tracing::info!(
        transport = transport_kind.as_str(),
        poll_secs = config.poll_secs,
        "scheduled report delivery loop started"
    );
    loop {
        if *shutdown.borrow_and_update() {
            return;
        }
        tokio::select! {
            _ = shutdown.changed() => return,
            _ = tokio::time::sleep(StdDuration::from_secs(config.poll_secs.max(1))) => {
                let now = Utc::now();
                match deliver_due(&pool, &reporting, transport.as_ref(), &transport_kind, &config, now).await {
                    Ok(n) if n > 0 => tracing::info!(schedules = n, "delivery cycle completed"),
                    Ok(_) => {}
                    Err(error) => tracing::error!(%error, "delivery cycle failed"),
                }
                // Retry of an earlier failed slot (Wave 52 lane R4). Same
                // cycle, same pooled reporting connection and same resolved
                // transport: handing this to the reconcile loop would mean a
                // second copy of both. A failure here never stops the claim
                // sweep above, so a broken retry cannot cost a new slot.
                match retry_due(&pool, &reporting, transport.as_ref(), &transport_kind, &config, now).await {
                    Ok(report) if report.retried > 0 || report.failed > 0 => tracing::info!(
                        pending = report.pending,
                        retried = report.retried,
                        exhausted = report.exhausted,
                        failed = report.failed,
                        "report delivery retry pass"
                    ),
                    Ok(_) => {}
                    Err(error) => tracing::error!(%error, "report delivery retry pass failed"),
                }
            }
        }
    }
}

/// One claim → render → fan-out → record pass over every due schedule.
/// Returns how many schedules were discharged. `now` is injected rather
/// than read from the host so the deterministic two-weeks harness can
/// drive this exact function over simulated time — production parity by
/// construction, not by reimplementation.
pub async fn deliver_due(
    pool: &PgPool,
    reporting: &PgPool,
    transport: &dyn core_mail::MailTransport,
    transport_kind: &TransportKind,
    config: &DeliveryLoopConfig,
    now: DateTime<Utc>,
) -> anyhow::Result<usize> {
    let claimed = {
        let mut tx = pool.begin().await?;
        let rows = core_db::report_schedules::claim_due(&mut tx, now, config.claim_batch).await?;
        for c in &rows {
            // A stored row that fails shape validation (impossible past
            // the migration's CHECKs, defensive regardless) still
            // ADVANCES — by a day-grace fallback — so one malformed row
            // can never pin the sweep on itself forever.
            let next = shape_of(
                &c.row.cadence,
                c.row.time_of_day,
                c.row.day_of_week,
                c.row.day_of_month,
                &c.row.timezone,
            )
            .and_then(|shape| next_run_after(&shape, c.due_at))
            .unwrap_or_else(|error| {
                tracing::error!(schedule = %c.row.name, %error, "schedule shape unusable; advancing by a day");
                c.due_at + Duration::hours(24)
            });
            core_db::report_schedules::advance_after_run(
                &mut tx,
                c.row.org_id,
                c.row.id,
                c.due_at,
                next,
            )
            .await?;
        }
        tx.commit().await?;
        rows
    };

    let discharged = claimed.len();
    for claimed in claimed {
        run_one(pool, reporting, transport, transport_kind, config, claimed).await;
    }
    Ok(discharged)
}

/// Delivers one claimed slot: none-mode skips ledger-only; everything
/// else renders once and fans out per recipient. NEVER returns an error
/// upward — a bad schedule is recorded, not propagated (the loop must
/// survive one broken row).
async fn run_one(
    pool: &PgPool,
    reporting: &PgPool,
    transport: &dyn core_mail::MailTransport,
    transport_kind: &TransportKind,
    config: &DeliveryLoopConfig,
    claimed: core_db::report_schedules::ClaimedSchedule,
) {
    let row = &claimed.row;
    let delivery_id = Uuid::now_v7();
    let org_id = row.org_id;
    let schedule_id = row.id;

    // None mode: log-skip, ledger, move on. The schedule ALREADY
    // advanced in the claim transaction — a missing provider costs one
    // logged slot, never a stuck loop.
    if matches!(transport_kind, TransportKind::None) {
        tracing::info!(
            schedule = %row.name,
            "email transport=none: report delivery skipped (schedule advanced)"
        );
        record(
            pool,
            core_db::report_schedules::DeliveryRecord {
                id: delivery_id,
                org_id,
                schedule_id,
                slot_at: claimed.due_at,
                status: "skipped".into(),
                detail: serde_json::json!({ "reason": "transport_none" }),
            },
        )
        .await;
        set_outcome(
            pool,
            org_id,
            schedule_id,
            claimed.due_at,
            Some("transport=none: skipped"),
        )
        .await;
        return;
    }

    // Render once per slot.
    let csv = render_slot(pool, reporting, &claimed.row, claimed.due_at).await;
    let csv = match csv {
        Ok(csv) => csv,
        Err(error) => {
            tracing::error!(schedule = %row.name, %error, "report render failed");
            record(
                pool,
                core_db::report_schedules::DeliveryRecord {
                    id: delivery_id,
                    org_id,
                    schedule_id,
                    slot_at: claimed.due_at,
                    status: "failed".into(),
                    detail: serde_json::json!({ "error": error.to_string() }),
                },
            )
            .await;
            set_outcome(
                pool,
                org_id,
                schedule_id,
                claimed.due_at,
                Some(&error.to_string()),
            )
            .await;
            return;
        }
    };

    // Fan out per recipient through the preference chain, evaluated at
    // the SLOT instant (not host time): the decision must be a function
    // of the claimed slot, so quiet hours and reruns stay deterministic.
    let mut delivered = 0_u32;
    let mut disabled = 0_u32;
    let mut quiet_deferred = 0_u32;
    let mut failures: Vec<serde_json::Value> = Vec::new();
    for user_id in &row.recipients {
        let Some(plan) = plan_recipient(
            pool,
            org_id,
            *user_id,
            NotificationKind::ReportDelivery,
            claimed.due_at,
        )
        .await
        else {
            failures.push(
                serde_json::json!({ "user_id": user_id, "error": "recipient not resolvable" }),
            );
            continue;
        };
        match plan.decision {
            Decision::Disabled => disabled += 1,
            Decision::QuietDeferred => quiet_deferred += 1,
            Decision::Deliver => {
                let message = OutboundEmail {
                    from_address: config.from_address.clone(),
                    from_name: config.from_name.clone(),
                    // Report mail replies go to the From address; board
                    // identities apply to ticket mail (Wave 34 lane 8).
                    reply_to: None,
                    to_addresses: vec![plan.user.email.clone()],
                    subject: format!("Scheduled report: {}", row.name),
                    body_text: csv.clone(),
                    // Stable threading anchor minted per (slot, recipient)
                    // pair — redeliveries of the same slot stay one thread.
                    rfc_message_id: format!("<report-{delivery_id}-{user_id}@marula.local>"),
                    in_reply_to: None,
                    references: vec![],
                };
                match transport.send(&message).await {
                    Ok(_) => delivered += 1,
                    // Every failure class is recorded the same way here: a
                    // report delivery is at-most-once by slot (R4 owns its
                    // retry), so the permanent/transient distinction changes
                    // which box an operator reads it in, not what happens next.
                    Err(error) => {
                        tracing::warn!(
                            schedule = %row.name,
                            user = %user_id,
                            permanent = error.permanent(),
                            %error,
                            "delivery transport refused message"
                        );
                        failures.push(
                            serde_json::json!({ "user_id": user_id, "error": error.to_string() }),
                        );
                    }
                }
            }
        }
    }

    let status = if failures.is_empty() {
        "delivered"
    } else if delivered > 0 {
        "partial"
    } else {
        "failed"
    };
    let detail = serde_json::json!({
        "delivered": delivered,
        "disabled": disabled,
        "quiet_deferred": quiet_deferred,
        "failed": failures,
    });
    let last_error = (!failures.is_empty()).then(|| {
        failures
            .first()
            .and_then(|f| f.get("error").and_then(|e| e.as_str()))
            .unwrap_or("delivery failed")
            .to_string()
    });
    record(
        pool,
        core_db::report_schedules::DeliveryRecord {
            id: delivery_id,
            org_id,
            schedule_id,
            slot_at: claimed.due_at,
            status: status.into(),
            detail,
        },
    )
    .await;
    set_outcome(
        pool,
        org_id,
        schedule_id,
        claimed.due_at,
        last_error.as_deref(),
    )
    .await;
    tracing::info!(
        schedule = %row.name,
        status,
        delivered,
        disabled,
        quiet_deferred,
        failed = failures.len(),
        "report slot discharged"
    );
}

// --- retry of a failed slot (Wave 52 lane R4) -------------------------------------
//
// Why this exists: claiming and advancing are one transaction, so a slot is
// claimed exactly once and a failed one is never claimed again. The ledger
// recorded the failure and nothing read it — at-most-once by construction, with
// the ledger row the only trace of a report a recipient never got.
//
// The repair appends rather than rewrites: `report_deliveries` is append-only
// and `record_delivery`'s own doc says a retry of the same slot appends a new
// row. That keeps the history honest (the failure and the recovery are both in
// it, which is what the delivery-history dialog shows) and removes the need for
// a status the ledger never had.
//
// The budget and the delay come from `Subsystem::Email`: a report goes out over
// the same provider as every other message, so the same policy answers "how long
// before the next try" and "when do we stop". Nothing here re-invents a backoff.

/// When the next attempt for a failed slot is due, or `None` when its budget is
/// spent.
///
/// `attempts` is how many ledger rows exist for the slot, so the first retry is
/// attempt 2 and takes the policy's delay for attempt 1.
#[must_use]
pub fn retry_due_at(
    policy: &core_domain::retry::RetryPolicy,
    attempts: i64,
    recorded_at: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let made = u32::try_from(attempts).unwrap_or(u32::MAX);
    if policy.exhausted(made) {
        return None;
    }
    let delay = policy.delay(made.max(1), 1.0);
    Some(recorded_at + Duration::from_std(delay).unwrap_or_else(|_| Duration::zero()))
}

/// What one retry pass did. The first two fields are the gauges' values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetryReport {
    /// Slots attempted this pass (`marula.reconcile.report_retry_pending`).
    pub pending: u64,
    /// Slots whose newest row is a failure with no attempts left
    /// (`marula.reconcile.report_retry_exhausted`).
    pub exhausted: u64,
    /// Slots whose retry delivered to every previously failed recipient.
    pub retried: u64,
    /// Slots whose retry did not finish (render or transport failure).
    pub failed: u64,
}

/// Re-attempts the failed report slots whose retry delay has elapsed.
///
/// Deliberately addressed: the retry mails only the recipients the failed row
/// names, so a `partial` slot never re-sends to the people who already have it.
/// The slot window is unchanged (`render_slot` takes the ledger's `slot_at`), so
/// a retry is the same report over the same period, re-read from the same
/// views — not a second report.
pub async fn retry_due(
    pool: &PgPool,
    reporting: &PgPool,
    transport: &dyn core_mail::MailTransport,
    transport_kind: &TransportKind,
    config: &DeliveryLoopConfig,
    now: DateTime<Utc>,
) -> anyhow::Result<RetryReport> {
    let policy = core_domain::retry::policy(core_domain::retry::Subsystem::Email);
    let base = Duration::from_std(policy.delay(1, 1.0)).unwrap_or_else(|_| Duration::zero());
    let base_secs = base.num_seconds().max(0) as f64;

    let mut report = RetryReport::default();
    // The exhausted count is a whole-set read, not a batch slice: the operator
    // number is "how many slots are dead", which does not depend on how many
    // this pass can attempt.
    report.exhausted = u64::try_from(
        core_db::reconcile::exhausted_report_retries(pool, i64::from(policy.max_attempts_i32(1)))
            .await?,
    )
    .unwrap_or(0);

    let due = config.claim_batch.max(1);
    let candidates = core_db::reconcile::due_report_retries(pool, base_secs, due).await?;
    for candidate in candidates {
        let Some(next_due) = retry_due_at(&policy, candidate.attempts, candidate.recorded_at)
        else {
            continue;
        };
        if next_due > now {
            // Inside its backoff: not due yet. Waiting here rather than in SQL
            // keeps one policy decision in one place.
            continue;
        }
        report.pending += 1;
        match retry_one(
            pool,
            reporting,
            transport,
            transport_kind,
            config,
            &candidate,
        )
        .await
        {
            RetryOutcome::Delivered => report.retried += 1,
            RetryOutcome::StillFailing => {}
            // `pending` is the work FOUND (it is recorded before the attempt),
            // so it is not discounted by an outcome: a slot the pass could not
            // retry is still a slot that needed one, and the gauge has to keep
            // saying so until a later pass fixes it.
            RetryOutcome::Skipped => {}
            RetryOutcome::Error => report.failed += 1,
        }
    }
    core_otel::metrics::reconcile_report_retry(report.pending, report.exhausted);
    Ok(report)
}

/// The three ways one retry attempt can end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryOutcome {
    Delivered,
    StillFailing,
    /// Nothing to do: the schedule or its definition vanished.
    Skipped,
    /// The attempt could not run (database or render failure).
    Error,
}

/// One slot's retry: re-render, mail only the previously failed recipients, and
/// append the outcome.
async fn retry_one(
    pool: &PgPool,
    reporting: &PgPool,
    transport: &dyn core_mail::MailTransport,
    transport_kind: &TransportKind,
    config: &DeliveryLoopConfig,
    slot: &core_db::reconcile::DueRetry,
) -> RetryOutcome {
    let mut conn = match pool.acquire().await {
        Ok(conn) => conn,
        Err(error) => {
            tracing::error!(%error, schedule_id = %slot.schedule_id, "retry: pool acquire failed");
            return RetryOutcome::Error;
        }
    };
    let row = match core_db::report_schedules::get(&mut *conn, slot.org_id, slot.schedule_id).await
    {
        Ok(row) => row,
        Err(core_db::DbError::NotFound) => return RetryOutcome::Skipped,
        Err(error) => {
            tracing::error!(%error, schedule_id = %slot.schedule_id, "retry: schedule read failed");
            return RetryOutcome::Error;
        }
    };
    drop(conn);

    let recipients = failed_recipients(&slot.detail);
    if recipients.is_empty() {
        // A `failed` row with no named recipient cannot exist past `run_one`
        // (that path records `delivered`), so this is a hand-edited row. Say so
        // and stop rather than guessing at the whole recipient list.
        tracing::warn!(
            schedule_id = %slot.schedule_id,
            slot = %slot.slot_at,
            "retry: failed slot names no recipient; not retrying"
        );
        return RetryOutcome::Skipped;
    }

    // A deployment that lost its mail provider records the attempt as skipped,
    // exactly like a first attempt in none mode: the slot stays visible, the
    // attempts budget is not burned on a transport that cannot send.
    if matches!(transport_kind, TransportKind::None) {
        record(
            pool,
            core_db::report_schedules::DeliveryRecord {
                id: Uuid::now_v7(),
                org_id: slot.org_id,
                schedule_id: slot.schedule_id,
                slot_at: slot.slot_at,
                status: "skipped".into(),
                detail: serde_json::json!({ "reason": "transport_none", "retry": true }),
            },
        )
        .await;
        return RetryOutcome::Skipped;
    }

    let csv = match render_slot(pool, reporting, &row, slot.slot_at).await {
        Ok(csv) => csv,
        Err(error) => {
            tracing::error!(schedule = %row.name, %error, "retry: render failed");
            record(
                pool,
                core_db::report_schedules::DeliveryRecord {
                    id: Uuid::now_v7(),
                    org_id: slot.org_id,
                    schedule_id: slot.schedule_id,
                    slot_at: slot.slot_at,
                    status: "failed".into(),
                    detail: serde_json::json!({
                        "retry": true,
                        "error": error.to_string(),
                        "failed": recipients
                            .iter()
                            .map(|id| serde_json::json!({ "user_id": id, "error": error.to_string() }))
                            .collect::<Vec<_>>(),
                    }),
                },
            )
            .await;
            return RetryOutcome::Error;
        }
    };

    let mut delivered = 0_u32;
    let mut disabled = 0_u32;
    let mut quiet_deferred = 0_u32;
    let mut failures: Vec<serde_json::Value> = Vec::new();
    for user_id in &recipients {
        let Some(plan) = plan_recipient(
            pool,
            slot.org_id,
            *user_id,
            NotificationKind::ReportDelivery,
            slot.slot_at,
        )
        .await
        else {
            failures.push(
                serde_json::json!({ "user_id": user_id, "error": "recipient not resolvable" }),
            );
            continue;
        };
        match plan.decision {
            Decision::Disabled => disabled += 1,
            Decision::QuietDeferred => quiet_deferred += 1,
            Decision::Deliver => {
                let message = OutboundEmail {
                    from_address: config.from_address.clone(),
                    from_name: config.from_name.clone(),
                    reply_to: None,
                    to_addresses: vec![plan.user.email.clone()],
                    subject: format!("Scheduled report: {}", row.name),
                    body_text: csv.clone(),
                    // The FAILED row's id, not a fresh one: a retry of one
                    // message carries one Message-ID, so a client that already
                    // holds a copy (the send succeeded and the report was lost)
                    // threads it instead of showing a duplicate.
                    rfc_message_id: format!("<report-{}-{user_id}@marula.local>", slot.row_id),
                    in_reply_to: None,
                    references: vec![],
                };
                match transport.send(&message).await {
                    Ok(_) => delivered += 1,
                    Err(error) => {
                        tracing::warn!(
                            schedule = %row.name,
                            user = %user_id,
                            %error,
                            "retry: transport refused message"
                        );
                        failures.push(
                            serde_json::json!({ "user_id": user_id, "error": error.to_string() }),
                        );
                    }
                }
            }
        }
    }

    let status = if failures.is_empty() {
        "delivered"
    } else if delivered > 0 {
        "partial"
    } else {
        "failed"
    };
    record(
        pool,
        core_db::report_schedules::DeliveryRecord {
            id: Uuid::now_v7(),
            org_id: slot.org_id,
            schedule_id: slot.schedule_id,
            slot_at: slot.slot_at,
            status: status.into(),
            detail: serde_json::json!({
                "retry": true,
                "delivered": delivered,
                "disabled": disabled,
                "quiet_deferred": quiet_deferred,
                "failed": failures,
            }),
        },
    )
    .await;
    // The schedule's visible error tracks the LATEST attempt: a retry that
    // delivered clears it, one that failed replaces it. `last_run_at` keeps the
    // instant the schedule itself last fired — moving it back to the retried
    // slot would rewrite when the schedule last ran.
    let last_error = (!failures.is_empty()).then(|| {
        failures
            .first()
            .and_then(|f| f.get("error").and_then(|e| e.as_str()))
            .unwrap_or("retry failed")
            .to_string()
    });
    // `set_retry_outcome`, not `set_outcome`: a retry is not a run, and moving
    // `last_run_at` onto the retried slot would claim the schedule fired then.
    set_retry_outcome(pool, slot.org_id, slot.schedule_id, last_error.as_deref()).await;

    if failures.is_empty() {
        tracing::info!(
            schedule = %row.name,
            slot = %slot.slot_at,
            recipients = delivered,
            "retry: report slot recovered"
        );
        RetryOutcome::Delivered
    } else {
        RetryOutcome::StillFailing
    }
}

/// The recipients a failed ledger row names, in the order they were recorded.
///
/// The detail shape is `run_one`'s: `failed: [{user_id, error}]`. Anything else
/// is a row this code did not write, and an unparseable element is skipped
/// rather than turned into a wildcard send.
fn failed_recipients(detail: &serde_json::Value) -> Vec<Uuid> {
    detail
        .get("failed")
        .and_then(|f| f.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("user_id").and_then(|id| id.as_str()))
                .filter_map(|id| Uuid::parse_str(id).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Renders one slot's CSV: load the referenced definition, revalidate the
/// stored spec through the registry (fail closed on drift), apply the
/// window, execute on the reporting pool.
async fn render_slot(
    pool: &PgPool,
    reporting: &PgPool,
    row: &core_db::report_schedules::ReportScheduleRow,
    slot: DateTime<Utc>,
) -> Result<String, anyhow::Error> {
    let def = core_db::report_defs::get(
        &mut *pool.acquire().await?,
        row.org_id,
        row.report_definition_id,
    )
    .await
    .map_err(|e| anyhow::anyhow!("definition load: {e}"))?;
    let spec = ReportSpec::from_parts(&def.kind, &def.spec, &def.viz)
        .map_err(|e| anyhow::anyhow!("stored definition failed registry: {e}"))?;
    let (sql, binds) = windowed_statement(&spec, row.window_days, slot)?;
    render_csv(reporting, &sql, &binds, row.org_id).await
}

/// Ledger append; failure is logged, never propagated (the schedule has
/// already advanced — an unwritable ledger must not double-fire runs).
async fn record(pool: &PgPool, rec: core_db::report_schedules::DeliveryRecord) {
    let result = async {
        let mut tx = pool.begin().await?;
        core_db::report_schedules::record_delivery(&mut tx, &rec).await?;
        tx.commit().await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if let Err(error) = result {
        tracing::error!(schedule_id = %rec.schedule_id, %error, "delivery ledger write failed");
    }
}

/// The retry's cosmetic write: the error moves, `last_run_at` does not.
/// Best effort, like every other ledger-adjacent write here.
async fn set_retry_outcome(pool: &PgPool, org_id: Uuid, schedule_id: Uuid, error: Option<&str>) {
    let result = async {
        let mut conn = pool.acquire().await?;
        core_db::report_schedules::set_retry_outcome(&mut conn, org_id, schedule_id, error).await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if let Err(e) = result {
        tracing::error!(%schedule_id, %e, "retry outcome write failed");
    }
}

/// Last-run cosmetics; same best-effort contract as `record`.
async fn set_outcome(
    pool: &PgPool,
    org_id: Uuid,
    schedule_id: Uuid,
    ran_at: DateTime<Utc>,
    error: Option<&str>,
) {
    let result = async {
        let mut tx = pool.begin().await?;
        core_db::report_schedules::set_run_outcome(&mut tx, org_id, schedule_id, ran_at, error)
            .await?;
        tx.commit().await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if let Err(e) = result {
        tracing::error!(schedule_id = %schedule_id, %e, "run outcome write failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_domain::retry::Subsystem;

    fn email_policy() -> core_domain::retry::RetryPolicy {
        core_domain::retry::policy(Subsystem::Email)
    }

    /// The deliberate failure at the heart of the retry: a slot inside its
    /// backoff must NOT be retried. Without this the pass would re-mail a
    /// provider that is rate limiting on every delivery tick, which is the
    /// storm the retry matrix exists to prevent.
    #[test]
    fn a_slot_inside_its_backoff_is_not_due_yet() {
        let recorded = Utc::now();
        let next = retry_due_at(&email_policy(), 1, recorded).expect("one retry left");
        assert!(
            next > recorded,
            "the first retry must wait for the policy's delay, not fire immediately"
        );
        assert!(
            next > Utc::now(),
            "a freshly failed slot is not due in the same tick"
        );
    }

    #[test]
    fn each_attempt_waits_longer_than_the_one_before_it() {
        let recorded = Utc::now();
        let first = retry_due_at(&email_policy(), 1, recorded).expect("due");
        let second = retry_due_at(&email_policy(), 2, recorded).expect("due");
        assert!(second > first, "the delay must grow with the attempt count");
    }

    /// The budget is the same one every other mail path uses; a slot that has
    /// spent it is reported as exhausted and never attempted again.
    #[test]
    fn a_slot_past_the_policy_budget_is_never_due_again() {
        let attempts = i64::from(email_policy().max_attempts_i32(1));
        assert_eq!(retry_due_at(&email_policy(), attempts, Utc::now()), None);
    }

    /// The addressee list is the whole safety property: a retry mails the
    /// recipients the failed row named, and a row that names none retries
    /// nobody rather than falling back to "everyone".
    #[test]
    fn only_the_named_failures_are_retried() {
        let detail = serde_json::json!({
            "delivered": 2,
            "failed": [
                { "user_id": "00000000-0000-0000-0000-000000000001", "error": "550" },
                { "user_id": "00000000-0000-0000-0000-000000000002", "error": "timeout" },
                { "user_id": "not-a-uuid", "error": "malformed" }
            ]
        });
        let recipients = failed_recipients(&detail);
        assert_eq!(
            recipients.len(),
            2,
            "the unparseable id is skipped, not a wildcard"
        );
        assert_eq!(
            recipients[0],
            Uuid::parse_str("00000000-0000-0000-0000-000000000001").expect("uuid")
        );
    }

    #[test]
    fn a_row_with_no_named_fallback_retries_nobody() {
        assert!(failed_recipients(&serde_json::json!({ "delivered": 0 })).is_empty());
        assert!(failed_recipients(&serde_json::json!({ "failed": [] })).is_empty());
        assert!(failed_recipients(&serde_json::json!(null)).is_empty());
    }
}
