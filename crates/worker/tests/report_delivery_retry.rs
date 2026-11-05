//! The report-delivery retry pass against a real Postgres (Wave 52 lane R4,
//! `p52-reconciliation-jobs`).
//!
//! Purpose: the retry pass is the one place in this lane where a mistake mails a
//! human twice. The unit tests pin the delay arithmetic and the addressee list;
//! this file drives the REAL pass through the REAL render path and a recording
//! transport, and asserts the two properties that matter:
//!
//! 1. A retry mails ONLY the recipients the failed row named — a `partial` slot
//!    never re-sends to the people who already received it.
//! 2. Once the retry lands, the slot leaves the retry set, so the next pass sends
//!    nothing. The ledger keeps both rows (append-only), which is what makes the
//!    history show the failure and the recovery.
//!
//! Needs `MARULA_TEST_DATABASE_URL` or `DATABASE_URL` (the lane's own container;
//! this file does not start one, same contract as `two_weeks_delivery.rs`).

use chrono::{DateTime, Duration, Utc};
use core_domain::entities::{NotificationKind, NotificationPreference};
use core_mail::{OutboundEmail, SendReceipt, TransportError, TransportKind};
use core_reports::{MeasureSpec, ReportSpec, VizHint};
use sqlx::PgPool;
use std::sync::Mutex;
use uuid::Uuid;
use worker::config::DeliveryLoopConfig;
use worker::delivery::retry_due;

const SLOT: DateTime<Utc> = match chrono::DateTime::from_timestamp(1_767_225_600, 0) {
    Some(t) => t,
    None => unreachable!(),
};

fn cfg() -> DeliveryLoopConfig {
    DeliveryLoopConfig {
        from_address: "reports@marula.test".into(),
        from_name: Some("Marula Reports".into()),
        claim_batch: 50,
        poll_secs: 60,
        pool_max: 5,
        statement_cache_capacity: 0,
    }
}

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
    fn recipients(&self) -> Vec<String> {
        self.sent
            .lock()
            .expect("mutex")
            .iter()
            .map(|m| m.to_addresses.join(","))
            .collect()
    }
}

async fn admin_pool() -> PgPool {
    let url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("retry harness needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    // The lane DB is reused across runs, and `retry_due` is org-blind: a failed
    // slot left by an earlier harness process is a candidate in this run and
    // would be counted as this pass's work. Clearing the schedules cascades the
    // deliveries, which is the same reset `two_weeks_delivery.rs` performs for
    // the same reason.
    sqlx::query("DELETE FROM report_schedules")
        .execute(&pool)
        .await
        .expect("clear stale schedules");
    pool
}

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

struct Seeded {
    org: Uuid,
    /// This run's unique suffix, so assertions can scope to its own recipients
    /// in a database other fixtures also write to.
    suffix: String,
    /// The recipient whose copy failed — the only one a retry may mail.
    failed: Uuid,
    /// The recipient whose copy succeeded — a retry that mails them again is the
    /// defect this test exists for.
    delivered: Uuid,
    schedule: Uuid,
}

async fn seed(pool: &PgPool) -> Seeded {
    let suffix = Uuid::now_v7().simple().to_string();
    let org = core_db::orgs::insert_org(
        pool,
        Uuid::now_v7(),
        &format!("Retry Co {suffix}"),
        &format!("retry-{suffix}"),
        "RC",
    )
    .await
    .expect("org");
    let failed = core_db::users::create_user(
        pool,
        Uuid::now_v7(),
        org.id,
        &format!("failed-{suffix}@marula.test"),
        "Failed",
        "x",
        "UTC",
    )
    .await
    .expect("user");
    let delivered = core_db::users::create_user(
        pool,
        Uuid::now_v7(),
        org.id,
        &format!("delivered-{suffix}@marula.test"),
        "Delivered",
        "x",
        "UTC",
    )
    .await
    .expect("user");

    // Both opted in: the preference chain must not be the thing that decides
    // this test, since the failed row already says who failed.
    for user in [&failed, &delivered] {
        core_db::notifications::upsert_preference(
            pool,
            &NotificationPreference {
                user_id: user.id,
                kind: NotificationKind::ReportDelivery,
                in_app: true,
                email: true,
                quiet_from: None,
                quiet_until: None,
                timezone: "UTC".into(),
            },
        )
        .await
        .expect("preference");
    }

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
    let definition = Uuid::now_v7();
    let mut tx = pool.begin().await.expect("tx");
    core_db::report_defs::insert(
        &mut tx,
        definition,
        org.id,
        failed.id,
        &core_db::report_defs::DefinitionFields {
            name: format!("Total tickets {suffix}"),
            description: None,
            kind: kind.to_string(),
            spec: serde_json::to_value(&body).expect("spec"),
            card: None,
            viz: viz.to_string(),
            scope: "org".into(),
        },
    )
    .await
    .expect("definition");

    let schedule = Uuid::now_v7();
    core_db::report_schedules::insert(
        &mut tx,
        schedule,
        org.id,
        failed.id,
        &core_db::report_schedules::ScheduleFields {
            report_definition_id: definition,
            name: format!("Weekly {suffix}"),
            recipients: vec![failed.id, delivered.id],
            cadence: "weekly".into(),
            time_of_day: chrono::NaiveTime::from_hms_opt(7, 0, 0).expect("time"),
            day_of_week: Some(0),
            day_of_month: None,
            timezone: "UTC".into(),
            window_days: 7,
            enabled: true,
            // Far from the retry window: the claim sweep must not touch this
            // schedule, so the test exercises the retry pass alone.
            next_run_at: Utc::now() + Duration::days(30),
        },
    )
    .await
    .expect("schedule");
    tx.commit().await.expect("commit");

    // The failure being repaired: one slot, delivered to one recipient, failed
    // for the other. `created_at` is inside the policy's first delay window?
    // No — it is old enough that the first retry is due.
    sqlx::query(
        "INSERT INTO report_deliveries (id, org_id, schedule_id, slot_at, status, detail, created_at)
         VALUES ($1, $2, $3, $4, 'partial', $5, now() - interval '30 minutes')",
    )
    .bind(Uuid::now_v7())
    .bind(org.id)
    .bind(schedule)
    .bind(SLOT)
    .bind(serde_json::json!({
        "delivered": 1,
        "disabled": 0,
        "quiet_deferred": 0,
        "failed": [{ "user_id": failed.id, "error": "550 mailbox unavailable" }],
    }))
    .execute(pool)
    .await
    .expect("seed a partial delivery");

    Seeded {
        org: org.id,
        suffix,
        failed: failed.id,
        delivered: delivered.id,
        schedule,
    }
}

#[tokio::test]
async fn a_retry_mails_only_the_failed_recipient_and_never_repeats() {
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("retry harness needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = admin_pool().await;
    let reporting = reporting_pool(&admin_url);
    let seeded = seed(&pool).await;
    let transport = RecordingTransport::default();
    // Any non-None kind: the pass never builds a transport, it is handed one.
    let kind = TransportKind::Smtp {
        host: "127.0.0.1".into(),
        port: 25,
        username: None,
        password: None,
    };

    let first = retry_due(&pool, &reporting, &transport, &kind, &cfg(), Utc::now())
        .await
        .expect("retry pass");
    assert_eq!(first.pending, 1, "the one partial slot is due");
    assert_eq!(first.retried, 1, "the retry delivered");
    assert_eq!(first.failed, 0);

    // Scoped to this test's org: `retry_due` is org-blind by design (the worker
    // owns every tenant), so anything another fixture left behind can be in the
    // same pass.
    let recipients: Vec<String> = transport
        .recipients()
        .into_iter()
        .filter(|to| to.contains(&seeded.suffix))
        .collect();
    assert_eq!(
        recipients.len(),
        1,
        "a partial slot must not re-send to the recipient who already has it"
    );
    assert!(
        recipients[0].contains("failed-"),
        "the retry addresses the failed recipient, got {:?}",
        recipients[0]
    );
    assert!(
        !recipients[0].contains("delivered-"),
        "the recipient who already received this slot is never mailed again"
    );

    // The ledger is append-only: the failure row stays, and the recovery is a
    // second row for the same slot.
    let rows: Vec<(String, Option<serde_json::Value>)> = sqlx::query_as(
        "SELECT status, detail FROM report_deliveries
          WHERE schedule_id = $1 AND slot_at = $2
          ORDER BY created_at",
    )
    .bind(seeded.schedule)
    .bind(SLOT)
    .fetch_all(&pool)
    .await
    .expect("ledger");
    assert_eq!(
        rows.len(),
        2,
        "the failure and the recovery are both recorded"
    );
    assert_eq!(rows[0].0, "partial");
    assert_eq!(rows[1].0, "delivered");
    assert_eq!(
        rows[1].1.as_ref().and_then(|d| d.get("retry")),
        Some(&serde_json::json!(true)),
        "the recovery row says it was a retry"
    );
    assert_eq!(
        rows[0]
            .1
            .as_ref()
            .and_then(|d| d.get("failed"))
            .and_then(|f| f.as_array())
            .and_then(|f| f.first())
            .and_then(|f| f.get("user_id"))
            .and_then(|id| id.as_str()),
        Some(seeded.failed.to_string().as_str()),
        "the failure row still names the recipient the retry addressed"
    );

    // The deliberate failure this guards against: a second pass re-mailing the
    // recovered slot. Nothing is due, so nothing is sent.
    let second = retry_due(&pool, &reporting, &transport, &kind, &cfg(), Utc::now())
        .await
        .expect("second retry pass");
    assert_eq!(second.pending, 0, "a recovered slot is not due again");
    assert_eq!(
        transport
            .recipients()
            .into_iter()
            .filter(|to| to.contains(&seeded.suffix))
            .count(),
        1,
        "the second pass sent nothing"
    );

    // The schedule's visible error is cleared by the recovery, and its
    // `last_run_at` is untouched by a retry.
    let (last_error, last_run_at): (Option<String>, Option<DateTime<Utc>>) =
        sqlx::query_as("SELECT last_error, last_run_at FROM report_schedules WHERE id = $1")
            .bind(seeded.schedule)
            .fetch_one(&pool)
            .await
            .expect("schedule");
    assert!(
        last_run_at.is_none(),
        "a retry does not make the schedule look like it ran"
    );
    assert!(
        last_error.is_none(),
        "the recovery clears the schedule's error"
    );

    // Cleanup: the harness DB is shared, and a schedule left behind fires in the
    // next run. Deliveries cascade.
    sqlx::query("DELETE FROM orgs WHERE id = $1")
        .bind(seeded.org)
        .execute(&pool)
        .await
        .expect("cleanup");

    let _ = seeded.delivered;
}
