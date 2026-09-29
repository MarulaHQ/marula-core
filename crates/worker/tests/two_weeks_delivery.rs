//! THE delivery acceptance harness (TASKLIST R4): "scheduled delivery
//! runs two simulated weeks with zero lost slots."
//!
//! Purpose: a deterministic, in-process simulation of 14 simulated days
//! of schedule ticks driven through the REAL delivery pipeline — the
//! same `worker::delivery::deliver_due` claim/advance/render/fan-out
//! path production runs, with only the clock and the transport injected
//! (the mail-lane harness pattern from `core-api/tests/two_weeks.rs`).
//!
//! Proven over the window:
//! 1. Daily/weekly/monthly cadences fire EXACTLY once per slot — no
//!    lost slots, no duplicates (overlapping sweeps in the same tick are
//!    absorbed by the claim CAS).
//! 2. Recipient fan-out honors the preference chain: email-disabled
//!    recipients are suppressed, quiet-hours recipients are deferred,
//!    opted-in recipients get exactly one message per slot.
//! 3. None mode (transport unset) LEDGERS every slot as `skipped` and
//!    never sends — and the schedule still advances.
//!
//! Non-purpose: no wall-clock sleeping, no real network (recording
//! transport), no HTTP.

use chrono::{DateTime, Duration, TimeZone, Utc};
use core_domain::entities::{NotificationKind, NotificationPreference};
use core_domain::notifications::QuietHours;
use core_mail::{OutboundEmail, SendReceipt, TransportError};
use core_reports::{MeasureSpec, ReportSpec, VizHint};
use sqlx::PgPool;
use std::sync::{Arc, Mutex};
use uuid::Uuid;
use worker::config::DeliveryLoopConfig;
use worker::delivery::deliver_due;

/// Sim window: 2026-01-01 (a Thursday) 00:00 → 2026-01-15 00:00.
const SIM_START: DateTime<Utc> = match chrono::DateTime::from_timestamp(1_767_225_600, 0) {
    Some(t) => t,
    None => unreachable!(),
};
const SIM_DAYS: i64 = 14;
const TICK: Duration = Duration::minutes(30);

fn cfg() -> DeliveryLoopConfig {
    DeliveryLoopConfig {
        from_address: "reports@marula.test".into(),
        from_name: Some("Marula Reports".into()),
        claim_batch: 50,
        poll_secs: 60,
    }
}

/// Records every send — the harness's zero-loss oracle.
#[derive(Default)]
struct RecordingTransport {
    sent: Mutex<Vec<OutboundEmail>>,
}

#[async_trait::async_trait]
impl core_mail::MailTransport for RecordingTransport {
    fn name(&self) -> &'static str {
        "recording"
    }
    async fn send(&self, message: &OutboundEmail) -> Result<SendReceipt, TransportError> {
        self.sent.lock().expect("mutex").push(message.clone());
        Ok(SendReceipt {
            provider_message_id: Some(format!("rec-{}", message.rfc_message_id)),
        })
    }
}

impl RecordingTransport {
    fn messages(&self) -> Vec<OutboundEmail> {
        self.sent.lock().expect("mutex").clone()
    }
}

/// The logging-only stand-in passed alongside `TransportKind::None` —
/// none-mode must never reach the transport at all, so anything here
/// would be a failure.
struct MustNotSend;

#[async_trait::async_trait]
impl core_mail::MailTransport for MustNotSend {
    fn name(&self) -> &'static str {
        "must-not-send"
    }
    async fn send(&self, _message: &OutboundEmail) -> Result<SendReceipt, TransportError> {
        Err(TransportError::Unavailable(
            "none mode must not send".into(),
        ))
    }
}

async fn admin_pool() -> PgPool {
    let url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("delivery harness needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    // The lane DB is shared across runs: stale schedules from earlier
    // harness processes would re-claim inside this run's sim window
    // (their next_run_at lags in the same simulated past). Clearing them
    // makes every run deterministic; deliveries cascade.
    sqlx::query("DELETE FROM report_schedules")
        .execute(&pool)
        .await
        .expect("clear stale schedules");
    pool
}

/// The RLS-constrained pool the render path executes on (same role
/// rewrite core-api applies for its reporting pool).
fn reporting_pool(admin_url: &str) -> PgPool {
    let target = if admin_url.contains("marula_app:") {
        admin_url.to_string()
    } else {
        let (scheme_rest, rest) = admin_url
            .split_once("://")
            .unwrap_or(("postgres", admin_url));
        match rest.split_once('@') {
            Some((_creds, host)) => format!("{scheme_rest}://marula_app:marula_app@{host}"),
            None => format!("{scheme_rest}://marula_app:marula_app@{rest}"),
        }
    };
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(3)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect_lazy(&target)
        .expect("reporting pool")
}

struct Org {
    id: Uuid,
    u_optin: Uuid,
    u_disabled: Uuid,
    u_quiet: Uuid,
    definition: Uuid,
}

/// Seeds one org with three staff recipients (opted-in / email-disabled /
/// quiet-hours) and one stored grand-total definition.
async fn seed_org(pool: &PgPool) -> Org {
    let suffix = Uuid::now_v7().simple().to_string();
    let org = core_db::orgs::insert_org(
        pool,
        Uuid::now_v7(),
        &format!("Delivery Co {suffix}"),
        &format!("delivery-{suffix}"),
        "DC",
    )
    .await
    .expect("org");
    let u_optin = core_db::users::create_user(
        pool,
        Uuid::now_v7(),
        org.id,
        &format!("optin-{suffix}@marula.test"),
        "Optin",
        "x",
        "UTC",
    )
    .await
    .expect("user");
    let u_disabled = core_db::users::create_user(
        pool,
        Uuid::now_v7(),
        org.id,
        &format!("disabled-{suffix}@marula.test"),
        "Disabled",
        "x",
        "UTC",
    )
    .await
    .expect("user");
    let u_quiet = core_db::users::create_user(
        pool,
        Uuid::now_v7(),
        org.id,
        &format!("quiet-{suffix}@marula.test"),
        "Quiet",
        "x",
        "UTC",
    )
    .await
    .expect("user");

    // Preference chain: opt-in delivers; disabled suppresses; quiet
    // delivers only outside 22:00–06:00 UTC.
    for (user, email, quiet) in [
        (&u_optin, true, None),
        (&u_disabled, false, None),
        (&u_quiet, true, Some(("22:00", "06:00"))),
    ] {
        let (from, until) = quiet.map_or((None, None), |(f, u)| {
            (Some(f.to_string()), Some(u.to_string()))
        });
        core_db::notifications::upsert_preference(
            pool,
            &NotificationPreference {
                user_id: user.id,
                kind: NotificationKind::ReportDelivery,
                in_app: true,
                email,
                quiet_from: from,
                quiet_until: until,
                timezone: "UTC".into(),
            },
        )
        .await
        .expect("preference");
    }

    // One stored definition: grand-total ticket count (registry-valid,
    // yields one CSV row even over an empty view).
    let spec = ReportSpec {
        view: core_reports::ViewId::Tickets,
        filters: vec![],
        group_by: vec![],
        measures: vec![MeasureSpec {
            agg: core_reports::Aggregate::Count,
            column: None,
        }],
        viz: VizHint::Table,
    };
    let (kind, body, viz) = spec.to_parts();
    let mut tx = pool.begin().await.expect("tx");
    core_db::report_defs::insert(
        &mut tx,
        Uuid::now_v7(),
        org.id,
        u_optin.id,
        &core_db::report_defs::DefinitionFields {
            name: "Total tickets".into(),
            description: None,
            kind: kind.to_string(),
            spec: serde_json::to_value(&body).unwrap(),
            viz: viz.to_string(),
            scope: "org".into(),
        },
    )
    .await
    .expect("definition");
    tx.commit().await.expect("commit");

    Org {
        id: org.id,
        u_optin: u_optin.id,
        u_disabled: u_disabled.id,
        u_quiet: u_quiet.id,
        definition: {
            let row = core_db::report_defs::list_all(pool, org.id)
                .await
                .expect("defs");
            row[0].id
        },
    }
}

/// Inserts a schedule whose first slot is the first cadence occurrence
/// strictly after SIM_START (the API path computes the same value).
#[allow(clippy::too_many_arguments)]
async fn seed_schedule(
    pool: &PgPool,
    org: &Org,
    name: &str,
    cadence: &str,
    time: &str,
    dow: Option<i16>,
    dom: Option<i16>,
    recipients: &[Uuid],
) -> Uuid {
    let time_of_day = chrono::NaiveTime::parse_from_str(time, "%H:%M").unwrap();
    let shape =
        core_domain::cadence::shape_of(cadence, time_of_day, dow, dom, "UTC").expect("shape");
    let first = core_domain::cadence::next_run_after(&shape, SIM_START).expect("first slot");
    let id = Uuid::now_v7();
    let mut tx = pool.begin().await.expect("tx");
    core_db::report_schedules::insert(
        &mut tx,
        id,
        org.id,
        org.u_optin,
        &core_db::report_schedules::ScheduleFields {
            report_definition_id: org.definition,
            name: format!("{name} {}", Uuid::now_v7().simple()),
            recipients: recipients.to_vec(),
            cadence: cadence.to_string(),
            time_of_day,
            day_of_week: dow,
            day_of_month: dom,
            timezone: "UTC".into(),
            window_days: 7,
            enabled: true,
            next_run_at: first,
        },
    )
    .await
    .expect("schedule");
    tx.commit().await.expect("commit");
    id
}

/// Drives the REAL deliver_due over the sim window in TICK steps.
async fn run_sim(
    pool: &PgPool,
    reporting: &PgPool,
    transport: &dyn core_mail::MailTransport,
    kind: &core_mail::TransportKind,
) {
    let config = cfg();
    let mut t = SIM_START;
    let end = SIM_START + Duration::days(SIM_DAYS);
    while t < end {
        deliver_due(pool, reporting, transport, kind, &config, t)
            .await
            .expect("delivery cycle");
        t += TICK;
    }
}

/// One serialized harness: the sweep is GLOBAL (org-blind by design —
/// a worker claims every org's due schedules), so the two-weeks phase
/// and the none-mode phase share one #[tokio::test] to keep their sim
/// windows from claiming each other's schedules.
#[tokio::test]
async fn two_simulated_weeks_with_zero_lost_slots_then_none_mode() {
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("test database url");
    let pool = admin_pool().await;
    let reporting = reporting_pool(&admin_url);
    let org = seed_org(&pool).await;

    // The scenario: one daily digest to the opted-in user, one weekly
    // Monday fan-out to optin+disabled, one monthly on the 1st, one
    // daily evening report to the quiet-hours user.
    let s_daily = seed_schedule(
        &pool,
        &org,
        "daily",
        "daily",
        "08:00",
        None,
        None,
        &[org.u_optin],
    )
    .await;
    let s_weekly = seed_schedule(
        &pool,
        &org,
        "weekly",
        "weekly",
        "09:00",
        Some(0),
        None,
        &[org.u_optin, org.u_disabled],
    )
    .await;
    let s_monthly = seed_schedule(
        &pool,
        &org,
        "monthly",
        "monthly",
        "06:00",
        None,
        Some(1),
        &[org.u_optin],
    )
    .await;
    let s_quiet = seed_schedule(
        &pool,
        &org,
        "quiet",
        "daily",
        "23:30",
        None,
        None,
        &[org.u_quiet],
    )
    .await;

    let transport = Arc::new(RecordingTransport::default());
    let kind = core_mail::TransportKind::Smtp {
        host: "127.0.0.1".into(),
        port: 2525,
        username: None,
        password: None,
    };
    run_sim(&pool, &reporting, &transport, &kind).await;

    // --- zero-loss assertions ---------------------------------------------

    // Expected slots: daily 14 (Jan 1..14), weekly Mondays Jan 5 + Jan 12,
    // monthly Jan 1. Quiet slots fire but defer every send.
    const DAILY_SLOTS: usize = 14;
    const WEEKLY_SLOTS: usize = 2;
    const MONTHLY_SLOTS: usize = 1;

    let messages = transport.messages();
    let expected_mails = DAILY_SLOTS + WEEKLY_SLOTS + MONTHLY_SLOTS;
    assert_eq!(
        messages.len(),
        expected_mails,
        "every opted-in slot delivered exactly once, disabled recipient never mailed"
    );
    // One message per (slot, recipient): RFC ids are minted per slot.
    let mut rfc_ids: Vec<&str> = messages.iter().map(|m| m.rfc_message_id.as_str()).collect();
    rfc_ids.sort_unstable();
    let n = rfc_ids.len();
    rfc_ids.dedup();
    assert_eq!(
        n,
        rfc_ids.len(),
        "no duplicate RFC message ids across the window"
    );
    for m in &messages {
        assert_eq!(m.from_address, "reports@marula.test");
        assert!(m.to_addresses[0].ends_with("@marula.test"));
        assert!(
            m.body_text.starts_with("count"),
            "CSV header first: {}",
            m.body_text
        );
    }

    // Ledger: one row per claimed slot, per-recipient outcomes honest.
    let ledger = |schedule: Uuid| {
        let pool = pool.clone();
        async move {
            core_db::report_schedules::list_deliveries(&pool, org.id, schedule, 1000)
                .await
                .expect("ledger")
        }
    };
    let daily_ledger = ledger(s_daily).await;
    assert_eq!(
        daily_ledger.len(),
        DAILY_SLOTS,
        "one ledger row per daily slot"
    );
    assert!(daily_ledger.iter().all(|(_, s, _)| s == "delivered"));

    let weekly_ledger = ledger(s_weekly).await;
    assert_eq!(weekly_ledger.len(), WEEKLY_SLOTS);
    for (_, status, detail) in &weekly_ledger {
        assert_eq!(status, "delivered");
        assert_eq!(detail["delivered"], 1, "opted-in recipient");
        assert_eq!(detail["disabled"], 1, "email-disabled recipient suppressed");
    }

    assert_eq!(ledger(s_monthly).await.len(), MONTHLY_SLOTS);

    let quiet_ledger = ledger(s_quiet).await;
    assert_eq!(quiet_ledger.len(), DAILY_SLOTS, "quiet slots still RUN");
    for (_, status, detail) in &quiet_ledger {
        assert_eq!(
            status, "delivered",
            "suppression is an outcome, not a failure"
        );
        assert_eq!(
            detail["quiet_deferred"], 1,
            "quiet hours defer the email leg"
        );
        assert_eq!(detail["delivered"], 0);
    }

    // Schedules advanced cleanly: the daily's next slot is the first one
    // AFTER the sim window (Jan 15 08:00), none left in the past.
    let daily_row = core_db::report_schedules::get(&pool, org.id, s_daily)
        .await
        .expect("schedule");
    assert_eq!(
        daily_row.next_run_at,
        Utc.with_ymd_and_hms(2026, 1, 15, 8, 0, 0).unwrap()
    );
    assert_eq!(
        daily_row.last_run_at,
        Some(Utc.with_ymd_and_hms(2026, 1, 14, 8, 0, 0).unwrap())
    );
    assert!(daily_row.last_error.is_none(), "clean run: {daily_row:?}");

    // Overlap inertness: one more sweep at the sim's end claims nothing —
    // no new mails, no new ledger rows.
    let before = transport.messages().len();
    deliver_due(
        &pool,
        &reporting,
        &transport,
        &kind,
        &cfg(),
        SIM_START + Duration::days(SIM_DAYS),
    )
    .await
    .expect("idempotent tail sweep");
    assert_eq!(
        transport.messages().len(),
        before,
        "overlapping sweep is inert"
    );

    // Phase 2: none mode over two further daily slots — the window is
    // strictly AFTER phase 1's sim, so the global sweep here only sees
    // the phase-2 schedule.
    none_mode_phase(&pool, &reporting).await;
}

async fn none_mode_phase(pool: &PgPool, reporting: &PgPool) {
    let org = seed_org(pool).await;

    let s = seed_schedule(
        pool,
        &org,
        "noon",
        "daily",
        "12:00",
        None,
        None,
        &[org.u_optin],
    )
    .await;

    let transport = Arc::new(MustNotSend);
    let config = cfg();
    // Two sweeps run well past the schedule's first slots (Jan 1/2 at
    // 12:00) — exercising the catch-up contract: a sweep discharges the
    // OLDEST pending slot and advances one cadence step, so a schedule
    // left un-run catches up one slot per sweep instead of bursting.
    deliver_due(
        pool,
        reporting,
        &transport,
        &core_mail::TransportKind::None,
        &config,
        Utc.with_ymd_and_hms(2026, 1, 15, 12, 0, 0).unwrap(),
    )
    .await
    .expect("cycle 1");
    deliver_due(
        pool,
        reporting,
        &transport,
        &core_mail::TransportKind::None,
        &config,
        Utc.with_ymd_and_hms(2026, 1, 16, 12, 0, 0).unwrap(),
    )
    .await
    .expect("cycle 2");

    let mut ledger = core_db::report_schedules::list_deliveries(pool, org.id, s, 100)
        .await
        .expect("ledger");
    ledger.sort_by_key(|(slot, _, _)| *slot);
    assert_eq!(ledger.len(), 2, "both pending slots ledgered");
    // The discharged slots are the ORIGINAL Jan 1/2 ones (oldest-first
    // catch-up), ledgered as skipped with the none reason.
    assert_eq!(
        ledger[0].0,
        Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap()
    );
    assert_eq!(
        ledger[1].0,
        Utc.with_ymd_and_hms(2026, 1, 2, 12, 0, 0).unwrap()
    );
    for (_, status, detail) in &ledger {
        assert_eq!(status, "skipped");
        assert_eq!(detail["reason"], "transport_none");
    }
    let row = core_db::report_schedules::get(pool, org.id, s)
        .await
        .expect("schedule");
    // The schedule STILL advanced: a missing provider never wedges the
    // sweep — the next pending slot is Jan 3 at 12:00.
    assert_eq!(
        row.next_run_at,
        Utc.with_ymd_and_hms(2026, 1, 3, 12, 0, 0).unwrap()
    );
    // And the quiet-hours helper is unaffected (guard: window parse ok).
    assert!(QuietHours::parse(Some("22:00"), Some("06:00"), "UTC").is_ok());
}
