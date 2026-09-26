//! worker/delivery — scheduled report delivery (TASKLIST R4).
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
use core_mail::{OutboundEmail, TransportError, TransportKind};
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
/// the RLS-constrained `meridian_app` role (same rewrite as core-api's
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
/// on — the SAME credential rewrite core-api applies (`meridian_app`
/// when the configured URL carries elevated creds).
fn reporting_target(database_url: &str) -> String {
    if database_url.contains("meridian_app:") {
        return database_url.to_string();
    }
    let (scheme_rest, rest) = database_url
        .split_once("://")
        .unwrap_or(("postgres", database_url));
    match rest.split_once('@') {
        Some((_creds, host)) => format!("{scheme_rest}://meridian_app:meridian_app@{host}"),
        None => format!("{scheme_rest}://meridian_app:meridian_app@{rest}"),
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
    let reporting = match sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(StdDuration::from_secs(30))
        .connect_lazy(&reporting_target(&database_url))
    {
        Ok(p) => p,
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
                    rfc_message_id: format!("<report-{delivery_id}-{user_id}@meridian.local>"),
                    in_reply_to: None,
                    references: vec![],
                };
                match transport.send(&message).await {
                    Ok(_) => delivered += 1,
                    Err(TransportError::Rejected(reason) | TransportError::Unavailable(reason)) => {
                        tracing::warn!(schedule = %row.name, user = %user_id, %reason, "delivery transport refused message");
                        failures.push(serde_json::json!({ "user_id": user_id, "error": reason }));
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
