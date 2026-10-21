//! Live-Postgres proof for `ops.dead_work` and the two verbs over it (Wave 52
//! lane R3, `p52-ledger-redrive`).
//!
//! Purpose: the union view, the redrive and the cancel statements are all new
//! and all runtime queries with no `.sqlx` entry, so this suite is what proves
//! they parse, bind, and mean what the module says they mean. Five properties:
//!
//! 1. The view returns one row shape over four ledgers, and each row names the
//!    thing an operator would call it.
//! 2. A redrive requeues a job in place with a fresh attempt budget, and a
//!    second redrive is a no-op (`NotDead`) rather than a second requeue.
//! 3. A cancel records the decision once and moves a job out of the dead set,
//!    so the depth an alert reads actually falls.
//! 4. A dead stream event is listed but not replayable, and the verb refuses it
//!    rather than pretending.
//! 5. A paused destination stops matching events, and a resume brings it back.
//!
//! Non-purpose: the HTTP surface (unit-tested for validation in core-api), the
//! worker's delivery loop, retention.
//!
//! Database selection follows the contract in `tests/common/mod.rs`.

mod common;

use common::{test_db, TestDb};
use core_db::dead_work::{self, CancelOutcome, DeadWorkFilter, DeadWorkSource, RedriveOutcome};
use core_db::webhooks::{self, DestinationOutcome};
use sqlx::PgPool;
use uuid::Uuid;

async fn seed_org(pool: &PgPool) -> Uuid {
    let org_id = Uuid::now_v7();
    sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, $2, $3)")
        .bind(org_id)
        .bind(format!("dead-work-{org_id}"))
        .bind(format!("dead-work-{org_id}"))
        .execute(pool)
        .await
        .expect("org");
    org_id
}

/// Inserts a job already past its budget, which is the state the queue leaves
/// behind and the only state the view reads.
async fn seed_dead_job(
    pool: &PgPool,
    org_id: Uuid,
    kind: &str,
    payload: serde_json::Value,
) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO jobs (id, org_id, kind, payload, status, attempts, max_attempts, last_error)
           VALUES ($1, $2, $3, $4, 'dead', 6, 6, 'handler said no')"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(kind)
    .bind(payload)
    .execute(pool)
    .await
    .expect("dead job");
    id
}

async fn seed_dead_email(pool: &PgPool, org_id: Uuid) -> Uuid {
    let message_id = Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO email_messages (id, org_id, direction, rfc_message_id, subject,
                                       from_address, to_addresses, scan_status, transport)
           VALUES ($1, $2, 'outbound', $3, 'Your invoice', 'support@acme.test',
                   ARRAY['client@example.test'], 'skipped', 'smtp')"#,
    )
    .bind(message_id)
    .bind(org_id)
    .bind(format!("<{message_id}@acme.test>"))
    .execute(pool)
    .await
    .expect("email message");

    let id = Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO email_outbound (id, org_id, email_message_id, status, attempts,
                                       max_attempts, last_error)
           VALUES ($1, $2, $3, 'dead', 8, 8, 'transport rejected message: no such user')"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(message_id)
    .execute(pool)
    .await
    .expect("dead email");
    id
}

async fn seed_webhook(pool: &PgPool, org_id: Uuid, url: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO webhooks (id, org_id, url, secret_encrypted)
           VALUES ($1, $2, $3, '\x00'::bytea)"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(url)
    .execute(pool)
    .await
    .expect("webhook");
    id
}

async fn seed_dead_event(pool: &PgPool, org_id: Uuid, entry: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO dead_events (id, consumer_group, entry_id, consumer, org_id,
                                    event_type, attempts, last_error)
           VALUES ($1, 'webhooks-out', $2, 'replica-a', $3, 'ticket.created', 6,
                   'handler kept failing')"#,
    )
    .bind(id)
    .bind(entry)
    .bind(org_id)
    .execute(pool)
    .await
    .expect("dead event");
    id
}

/// Every dead-work row for one org, no filtering.
async fn all_rows(db: &TestDb, org_id: Uuid) -> Vec<dead_work::DeadWorkRow> {
    let mut tx = db.app.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    let rows = dead_work::list(
        &mut tx,
        org_id,
        DeadWorkFilter {
            source: None,
            include_cancelled: false,
            limit: 100,
            offset: 0,
        },
    )
    .await
    .expect("list");
    tx.rollback().await.expect("rollback (reads only)");
    rows
}

async fn job_status(db: &TestDb, id: Uuid) -> (String, i32) {
    sqlx::query_as::<_, (String, i32)>("SELECT status, attempts FROM jobs WHERE id = $1")
        .bind(id)
        .fetch_one(&db.admin)
        .await
        .expect("job status")
}

async fn email_status(db: &TestDb, id: Uuid) -> (String, i32) {
    sqlx::query_as::<_, (String, i32)>("SELECT status, attempts FROM email_outbound WHERE id = $1")
        .bind(id)
        .fetch_one(&db.admin)
        .await
        .expect("email status")
}

#[tokio::test]
async fn the_union_names_every_ledger_and_its_subject() {
    let db = test_db().await;
    let org_id = seed_org(&db.admin).await;

    let job_id = seed_dead_job(
        &db.admin,
        org_id,
        "workflow.resume",
        serde_json::json!({ "ticket_id": Uuid::now_v7() }),
    )
    .await;
    let email_id = seed_dead_email(&db.admin, org_id).await;
    let webhook = seed_webhook(&db.admin, org_id, "https://receiver.example.test/hook").await;
    let event_id = Uuid::now_v7();
    let webhook_job = seed_dead_job(
        &db.admin,
        org_id,
        webhook_out_kind(),
        serde_json::json!({
            "webhook_id": webhook,
            "event_id": event_id,
            "envelope": {
                "id": event_id,
                "type": "ticket.created",
                "org_id": org_id,
                "entity": "ticket",
                "occurred_at": "2026-01-01T00:00:00Z",
                "payload_schema_ref": "core-domain/v1",
                "payload": {},
            },
        }),
    )
    .await;
    let dead_event = seed_dead_event(&db.admin, org_id, "1700000000000-0").await;

    let rows = all_rows(&db, org_id).await;
    let by_source = |source: DeadWorkSource| -> Vec<&dead_work::DeadWorkRow> {
        rows.iter().filter(|r| r.source == source).collect()
    };

    // A plain job: named by its kind, replayable.
    let jobs: Vec<_> = by_source(DeadWorkSource::Job);
    assert_eq!(jobs.len(), 1, "one plain job, got {rows:#?}");
    assert_eq!(jobs[0].id, job_id);
    assert!(
        jobs[0].subject.starts_with("workflow.resume"),
        "{:?}",
        jobs[0].subject
    );
    assert!(jobs[0].replayable);
    assert_eq!(jobs[0].attempts, 6);
    assert_eq!(jobs[0].last_error.as_deref(), Some("handler said no"));

    // The webhook delivery is its own source, because the operator's question
    // about it is different, and its subject names the URL and the event.
    let webhooks: Vec<_> = by_source(DeadWorkSource::Webhook);
    assert_eq!(webhooks.len(), 1, "one webhook delivery, got {rows:#?}");
    assert_eq!(webhooks[0].id, webhook_job);
    assert!(
        webhooks[0]
            .subject
            .contains("https://receiver.example.test/hook"),
        "{:?}",
        webhooks[0].subject
    );
    assert!(
        webhooks[0].subject.contains("ticket.created"),
        "{:?}",
        webhooks[0].subject
    );

    // An email: subject and recipient, which is what an operator recognises.
    let emails: Vec<_> = by_source(DeadWorkSource::Email);
    assert_eq!(emails.len(), 1, "one email, got {rows:#?}");
    assert_eq!(emails[0].id, email_id);
    assert_eq!(
        emails[0].subject, "Your invoice -> client@example.test",
        "{:?}",
        emails[0].subject
    );

    // A dead stream entry: listed, and NOT replayable from this surface.
    let events: Vec<_> = by_source(DeadWorkSource::Event);
    assert_eq!(events.len(), 1, "one dead event, got {rows:#?}");
    assert_eq!(events[0].id, dead_event);
    assert!(!events[0].replayable);
    assert!(
        events[0].subject.contains("webhooks-out"),
        "{:?}",
        events[0].subject
    );
}

/// The worker's job kind, spelled here rather than imported from the worker
/// crate: core-db's tests must not depend on it, and the string is a queue
/// contract the view reads through.
fn webhook_out_kind() -> &'static str {
    "webhook.deliver"
}

#[tokio::test]
async fn a_redrive_requeues_in_place_and_a_second_one_changes_nothing() {
    let db = test_db().await;
    let org_id = seed_org(&db.admin).await;
    let job_id = seed_dead_job(&db.admin, org_id, "search.index", serde_json::json!({})).await;
    let email_id = seed_dead_email(&db.admin, org_id).await;

    let mut tx = db.app.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");

    let first = dead_work::redrive(&mut tx, org_id, DeadWorkSource::Job, job_id)
        .await
        .expect("redrive job");
    assert_eq!(first, RedriveOutcome::Requeued);

    let second = dead_work::redrive(&mut tx, org_id, DeadWorkSource::Job, job_id)
        .await
        .expect("redrive job twice");
    assert_eq!(
        second,
        RedriveOutcome::NotDead,
        "a second redrive must not requeue again"
    );

    let email = dead_work::redrive(&mut tx, org_id, DeadWorkSource::Email, email_id)
        .await
        .expect("redrive email");
    assert_eq!(email, RedriveOutcome::Requeued);
    tx.commit().await.expect("commit");

    // The attempt budget restarts. Without that the row would re-dead on its
    // first failure, which is a verb that only pretends.
    let (status, attempts) = job_status(&db, job_id).await;
    assert_eq!(status, "pending");
    assert_eq!(attempts, 0);
    assert_eq!(email_status(&db, email_id).await, ("queued".to_string(), 0));

    // A dead event is refused, because this surface has no stream writer.
    let event_id = seed_dead_event(&db.admin, org_id, "1700000000001-0").await;
    let mut tx = db.app.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    assert_eq!(
        dead_work::redrive(&mut tx, org_id, DeadWorkSource::Event, event_id)
            .await
            .expect("redrive event"),
        RedriveOutcome::NotReplayable
    );
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn a_cancel_is_recorded_once_and_moves_a_job_out_of_the_dead_set() {
    let db = test_db().await;
    let org_id = seed_org(&db.admin).await;
    let job_id = seed_dead_job(&db.admin, org_id, "report.deliver", serde_json::json!({})).await;

    let mut tx = db.app.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    let first = dead_work::cancel(
        &mut tx,
        org_id,
        DeadWorkSource::Job,
        job_id,
        "the schedule was deleted",
        None,
    )
    .await
    .expect("cancel");
    assert_eq!(first, CancelOutcome::Cancelled);
    let second = dead_work::cancel(
        &mut tx,
        org_id,
        DeadWorkSource::Job,
        job_id,
        "a second operator clicking the same button",
        None,
    )
    .await
    .expect("cancel twice");
    assert_eq!(
        second,
        CancelOutcome::AlreadyCancelled,
        "the first decision stands; a second must not rewrite it"
    );
    tx.commit().await.expect("commit");

    // The dead set actually shrank: `failed` is a status the queue never
    // writes, so a cancelled job stops feeding the depth an alert reads.
    let (status, _) = job_status(&db, job_id).await;
    assert_eq!(status, "failed");
    assert!(
        all_rows(&db, org_id).await.is_empty(),
        "the list is empty again"
    );

    // The decision survives, and asking for cancelled rows shows the row with
    // the timestamp rather than hiding the history.
    let mut tx = db.app.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    let with_cancelled = dead_work::list(
        &mut tx,
        org_id,
        DeadWorkFilter {
            source: None,
            include_cancelled: true,
            limit: 100,
            offset: 0,
        },
    )
    .await
    .expect("list cancelled");
    tx.commit().await.expect("commit");
    let row = with_cancelled
        .iter()
        .find(|r| r.id == job_id && r.source == DeadWorkSource::Job);
    // The cancelled job has left the dead set, so the default list no longer
    // shows it — but asking for cancelled rows still shows what was abandoned,
    // when, and that it is no longer replayable.
    let row = row.expect("a cancelled row is visible when cancelled rows are asked for");
    assert!(
        row.cancelled_at.is_some(),
        "the decision time is on the row"
    );
    assert!(
        !row.replayable,
        "a cancelled row must not offer a redrive the verb would refuse"
    );
    let kept: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM dead_work_cancellations WHERE source = 'job' AND id = $1",
    )
    .bind(job_id)
    .fetch_one(&db.admin)
    .await
    .expect("cancellation count");
    assert_eq!(kept, 1, "one decision, not one per click");

    // Cancel is terminal: the redrive verb refuses rather than making the
    // decision a suggestion.
    let mut tx = db.app.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    assert_eq!(
        dead_work::redrive(&mut tx, org_id, DeadWorkSource::Job, job_id)
            .await
            .expect("redrive cancelled job"),
        RedriveOutcome::Cancelled
    );
    tx.commit().await.expect("commit");
    assert_eq!(job_status(&db, job_id).await.0, "failed");
}

#[tokio::test]
async fn repeated_failures_pause_a_destination_and_a_resume_clears_it() {
    let db = test_db().await;
    let org_id = seed_org(&db.admin).await;
    let webhook_id = seed_webhook(&db.admin, org_id, "https://flaky.example.test/hook").await;

    let threshold = core_domain::webhooks::pause_after_failures();

    // Below the threshold: counting, not paused, and still matched.
    for _ in 0..threshold - 1 {
        let mut tx = db.app.begin().await.expect("tx");
        core_db::set_org(&mut tx, org_id).await.expect("set_org");
        let paused = webhooks::record_destination_outcome(
            &mut tx,
            org_id,
            webhook_id,
            DestinationOutcome::Failed,
            threshold,
        )
        .await
        .expect("record failure");
        assert!(paused.is_none(), "must not pause before the threshold");
        tx.commit().await.expect("commit");
    }
    assert_eq!(
        webhooks::find_enabled_for_event(&db.admin, org_id, "ticket.created")
            .await
            .expect("match")
            .len(),
        1,
        "a destination below the threshold still receives events"
    );

    // The threshold attempt pauses it, writes the reason, and stops matching.
    let mut tx = db.app.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    let paused = webhooks::record_destination_outcome(
        &mut tx,
        org_id,
        webhook_id,
        DestinationOutcome::Failed,
        threshold,
    )
    .await
    .expect("record threshold failure");
    assert!(paused.is_some(), "the threshold attempt must pause");
    tx.commit().await.expect("commit");

    let row = webhooks::get_webhook(&db.admin, org_id, webhook_id)
        .await
        .expect("webhook");
    assert!(row.paused_at.is_some());
    assert_eq!(row.consecutive_failures, threshold);
    assert!(row
        .paused_reason
        .as_deref()
        .unwrap_or_default()
        .contains("consecutive delivery failures"));
    assert!(
        webhooks::find_enabled_for_event(&db.admin, org_id, "ticket.created")
            .await
            .expect("match")
            .is_empty(),
        "a paused destination must not be matched for new events"
    );

    // A single success while paused resets the counter but does not lift the
    // pause: the operator decided that, not the next delivery.
    let mut tx = db.app.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    webhooks::record_destination_outcome(
        &mut tx,
        org_id,
        webhook_id,
        DestinationOutcome::Accepted,
        threshold,
    )
    .await
    .expect("record success");
    tx.commit().await.expect("commit");
    let row = webhooks::get_webhook(&db.admin, org_id, webhook_id)
        .await
        .expect("webhook");
    assert_eq!(row.consecutive_failures, 0);
    assert!(row.paused_at.is_some(), "a success does not resume a pause");

    // Resume clears both, and the destination receives events again.
    let mut tx = db.app.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    assert!(webhooks::resume_destination(&mut *tx, org_id, webhook_id)
        .await
        .expect("resume"));
    tx.commit().await.expect("commit");
    let row = webhooks::get_webhook(&db.admin, org_id, webhook_id)
        .await
        .expect("webhook");
    assert!(row.paused_at.is_none());
    assert!(row.paused_reason.is_none());
    assert_eq!(row.consecutive_failures, 0);
    assert_eq!(
        webhooks::find_enabled_for_event(&db.admin, org_id, "ticket.created")
            .await
            .expect("match")
            .len(),
        1
    );
}

#[tokio::test]
async fn a_refusal_on_the_merits_does_not_count_against_the_destination() {
    let db = test_db().await;
    let org_id = seed_org(&db.admin).await;
    let webhook_id = seed_webhook(&db.admin, org_id, "https://picky.example.test/hook").await;
    let threshold = core_domain::webhooks::pause_after_failures();

    // Twice the threshold in refusals: still not paused, because the endpoint
    // keeps answering and it is the requests it will not take.
    for _ in 0..threshold * 2 {
        let mut tx = db.app.begin().await.expect("tx");
        core_db::set_org(&mut tx, org_id).await.expect("set_org");
        let paused = webhooks::record_destination_outcome(
            &mut tx,
            org_id,
            webhook_id,
            DestinationOutcome::RefusedOnMerits,
            threshold,
        )
        .await
        .expect("record refusal");
        assert!(paused.is_none());
        tx.commit().await.expect("commit");
    }
    let row = webhooks::get_webhook(&db.admin, org_id, webhook_id)
        .await
        .expect("webhook");
    assert!(row.paused_at.is_none());
    assert_eq!(row.consecutive_failures, 0);
}

#[tokio::test]
async fn another_org_cannot_see_or_touch_this_dead_work() {
    let db = test_db().await;
    let mine = seed_org(&db.admin).await;
    let theirs = seed_org(&db.admin).await;
    let job_id = seed_dead_job(&db.admin, mine, "search.index", serde_json::json!({})).await;

    // The view is security_invoker, so the other org's scoped session reads
    // nothing even though the row exists.
    let rows = all_rows(&db, theirs).await;
    assert!(rows.is_empty(), "{rows:#?}");

    // And the verbs are scoped too: a redrive under the wrong org reports "no
    // such row" instead of moving someone else's work.
    let mut tx = db.app.begin().await.expect("tx");
    core_db::set_org(&mut tx, theirs).await.expect("set_org");
    assert_eq!(
        dead_work::redrive(&mut tx, theirs, DeadWorkSource::Job, job_id)
            .await
            .expect("redrive"),
        RedriveOutcome::NotFound
    );
    assert_eq!(
        dead_work::cancel(
            &mut tx,
            theirs,
            DeadWorkSource::Job,
            job_id,
            "not mine",
            None
        )
        .await
        .expect("cancel"),
        CancelOutcome::NotFound
    );
    tx.commit().await.expect("commit");

    // The victim row is untouched.
    assert_eq!(job_status(&db, job_id).await.0, "dead");
}
