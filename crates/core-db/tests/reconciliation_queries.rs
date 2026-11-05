//! The reconciliation queries against a real Postgres (Wave 52 lane R4,
//! `p52-reconciliation-jobs`).
//!
//! Purpose: these four statements decide what the reconciler repairs, and they
//! are runtime queries (`sqlx::query_as`/`query_scalar`, not the `query!` macro —
//! they have no `.sqlx` entry yet), so the compiler never sees their SQL. This
//! file is what stops a typo, a wrong join or a wrong status vocabulary from
//! reaching production, and it is also the only place migration `0150` is
//! actually APPLIED: `scripts/check-migrations.sh` lints SQL, it does not run
//! it, and a migration that has never been applied is not verified.
//!
//! Each test builds the FAILURE the query exists to find, not a happy path:
//! an unindexed source row, a dead entry that must not be replayed, a parked run
//! whose wake job is gone, a report slot that failed once and was never claimed
//! again.
//!
//! Database selection follows `tests/common/mod.rs`: `MARULA_TEST_DATABASE_URL`,
//! else `DATABASE_URL`, else a local testcontainer. The schema is reset and
//! migrated from scratch per test binary, which is what makes "0150 applied to a
//! virgin schema" a claim this file proves.

mod common;

use chrono::{Duration, Utc};
use common::{db_now, fixed_uuid, org_tx, test_db};
use core_db::{clients, orgs, queue, reconcile, report_defs, report_schedules, search, users};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

async fn seed_org(app: &PgPool, n: u64) -> Uuid {
    let org = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    orgs::insert_org(
        &mut *tx,
        org,
        "Acme MSP",
        &format!("acme-reconcile-{n}"),
        "REC",
    )
    .await
    .expect("org insert");
    tx.commit().await.expect("commit seed");
    org
}

async fn seed_user(app: &PgPool, org: Uuid, n: u64) -> Uuid {
    let id = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    users::create_user(
        &mut *tx,
        id,
        org,
        &format!("reconcile{n}@acme.test"),
        &format!("Reconcile {n}"),
        "argon2hash",
        "UTC",
    )
    .await
    .expect("create user");
    tx.commit().await.expect("commit user");
    id
}

/// One dead stream entry, written the way the consumer harness writes it.
#[allow(clippy::too_many_arguments)]
async fn dead_event(
    app: &PgPool,
    org: Uuid,
    id: Uuid,
    group: &str,
    event_id: Option<Uuid>,
    payload: Option<&str>,
    replayable_at: Option<chrono::DateTime<Utc>>,
) {
    sqlx::query(
        "INSERT INTO dead_events
           (id, consumer_group, entry_id, consumer, org_id, event_id, event_type,
            attempts, last_error, payload, replayed_at)
         VALUES ($1, $2, $3, 'replica-a', $4, $5, 'ticket.updated', 6, 'boom', $6, $7)",
    )
    .bind(id)
    .bind(group)
    .bind(format!("{}-0", id.as_u128() % 1_000_000))
    .bind(org)
    .bind(event_id)
    .bind(payload)
    .bind(replayable_at)
    .execute(app)
    .await
    .expect("seed dead event");
}

// ---------------------------------------------------------------------------
// The search-index backfill
// ---------------------------------------------------------------------------

/// The failure: an entity that exists and has no index row, because its event
/// was published before the consumer group existed or dead-lettered since.
#[tokio::test]
async fn a_source_row_with_no_index_row_is_found_and_an_indexed_one_is_not() {
    let db = test_db().await;
    let org = seed_org(&db.admin, 92_000).await;
    let indexed = fixed_uuid(92_001);
    let missing = fixed_uuid(92_002);

    let mut tx = org_tx(&db.app, org).await;
    clients::create_client(&mut *tx, org, indexed, "Indexed Ltd", None)
        .await
        .expect("client");
    clients::create_client(&mut *tx, org, missing, "Missing Ltd", None)
        .await
        .expect("client");
    search::upsert_row(
        &mut *tx,
        org,
        search::SearchKind::Client,
        indexed,
        None,
        "Indexed Ltd",
        "",
        None,
    )
    .await
    .expect("index row");
    tx.commit().await.expect("commit");

    // Scoped to this test's org: the schema is shared by the whole binary, so
    // every other test's unindexed rows are in the same answer.
    let found: Vec<_> = reconcile::missing_search_rows(&db.admin, 500)
        .await
        .expect("backfill scan")
        .into_iter()
        .filter(|row| row.org_id == org)
        .collect();
    assert_eq!(found.len(), 1, "only the unindexed client is missing");
    assert_eq!(found[0].entity_id, missing);
    assert_eq!(found[0].entity_kind, "client");
}

/// The pass must be self-advancing: once the missing row is indexed, the same
/// query returns nothing. A query that returned the same row forever would make
/// the backlog gauge a constant.
#[tokio::test]
async fn the_backlog_returns_to_zero_once_the_row_is_indexed() {
    let db = test_db().await;
    let org = seed_org(&db.admin, 92_010).await;
    let client = fixed_uuid(92_011);
    let mut tx = org_tx(&db.app, org).await;
    clients::create_client(&mut *tx, org, client, "Late Ltd", None)
        .await
        .expect("client");
    tx.commit().await.expect("commit");

    let before = reconcile::missing_search_rows(&db.admin, 500)
        .await
        .expect("scan")
        .into_iter()
        .filter(|row| row.org_id == org)
        .count();
    assert_eq!(before, 1, "the unindexed client is the org's whole backlog");

    let mut tx = org_tx(&db.app, org).await;
    search::upsert_row(
        &mut *tx,
        org,
        search::SearchKind::Client,
        client,
        None,
        "Late Ltd",
        "",
        None,
    )
    .await
    .expect("index");
    tx.commit().await.expect("commit");

    let after = reconcile::missing_search_rows(&db.admin, 500)
        .await
        .expect("scan")
        .into_iter()
        .filter(|row| row.org_id == org)
        .count();
    assert_eq!(
        after, 0,
        "the gauge's number must fall back to zero, not stay at one"
    );
}

// ---------------------------------------------------------------------------
// Dead-entry replay
// ---------------------------------------------------------------------------

/// The failure: four dead entries, three of which must NOT be replayed — one
/// whose body never parsed, one an operator cancelled, one already replayed.
#[tokio::test]
async fn only_a_parsed_uncancelled_unreplayed_entry_is_eligible() {
    let db = test_db().await;
    let org = seed_org(&db.admin, 92_020).await;
    let eligible = fixed_uuid(92_021);
    let unparsed = fixed_uuid(92_022);
    let cancelled = fixed_uuid(92_023);
    let done = fixed_uuid(92_024);

    dead_event(
        &db.admin,
        org,
        eligible,
        "notifier-a",
        Some(fixed_uuid(92_031)),
        Some("{}"),
        None,
    )
    .await;
    dead_event(
        &db.admin,
        org,
        unparsed,
        "notifier-a",
        None,
        Some("not json"),
        None,
    )
    .await;
    dead_event(
        &db.admin,
        org,
        cancelled,
        "notifier-a",
        Some(fixed_uuid(92_032)),
        Some("{}"),
        None,
    )
    .await;
    dead_event(
        &db.admin,
        org,
        done,
        "notifier-a",
        Some(fixed_uuid(92_033)),
        Some("{}"),
        Some(Utc::now()),
    )
    .await;

    sqlx::query(
        "INSERT INTO dead_work_cancellations (source, id, org_id, reason)
         VALUES ('event', $1, $2, 'operator decided: poison payload')",
    )
    .bind(cancelled)
    .bind(org)
    .execute(&db.admin)
    .await
    .expect("cancel");

    let rows: Vec<_> = reconcile::replayable_dead_events(&db.admin, 500, 3)
        .await
        .expect("replay scan")
        .into_iter()
        .filter(|row| row.consumer_group == "notifier-a")
        .collect();
    assert_eq!(
        rows.len(),
        1,
        "only one entry of this group may be replayed"
    );
    assert_eq!(rows[0].id, eligible);
    assert_eq!(rows[0].replay_count, 0);
    assert_eq!(rows[0].replay_count, 0);

    let backlog = reconcile::dead_event_backlog(&db.admin, 3)
        .await
        .expect("backlog");
    let notifier = backlog
        .iter()
        .find(|g| g.consumer_group == "notifier-a")
        .expect("notifier group in the backlog");
    assert_eq!(notifier.replayable, 1);
    assert_eq!(
        notifier.unrepairable, 1,
        "the unparsed body is the one that needs a person; the cancelled and \
         already-replayed rows are decisions, not outstanding work"
    );
}

/// The bound that ends a ping-pong: an event that has already died more times
/// than the budget stops being replayable, so a poison envelope cannot loop
/// through publish → fail → dead-letter → publish forever.
#[tokio::test]
async fn an_event_past_its_death_budget_is_unrepairable_and_not_replayed() {
    let db = test_db().await;
    let org = seed_org(&db.admin, 92_040).await;
    let event_id = fixed_uuid(92_041);
    for n in 0..4 {
        dead_event(
            &db.admin,
            org,
            fixed_uuid(92_045 + n),
            "search-indexer-b",
            Some(event_id),
            Some("{}"),
            None,
        )
        .await;
    }

    let group_rows: Vec<_> = reconcile::replayable_dead_events(&db.admin, 500, 3)
        .await
        .expect("scan")
        .into_iter()
        .filter(|row| row.consumer_group == "search-indexer-b")
        .collect();
    assert!(
        group_rows.is_empty(),
        "four deaths against a budget of three means replay has given up"
    );
    let backlog = reconcile::dead_event_backlog(&db.admin, 3)
        .await
        .expect("backlog");
    let group = backlog
        .iter()
        .find(|g| g.consumer_group == "search-indexer-b")
        .expect("group");
    assert_eq!(group.unrepairable, 4);
    assert_eq!(group.replayable, 0);
}

/// Marking is a compare-and-set: the second reconcile replica (or the next pass
/// on the same replica) must not replay the same entry again.
#[tokio::test]
async fn marking_a_replay_is_idempotent() {
    let db = test_db().await;
    let org = seed_org(&db.admin, 92_050).await;
    let id = fixed_uuid(92_051);
    dead_event(
        &db.admin,
        org,
        id,
        "notifier-c",
        Some(fixed_uuid(92_052)),
        Some("{}"),
        None,
    )
    .await;

    assert_eq!(
        reconcile::mark_replayed(&db.admin, &[id])
            .await
            .expect("first mark"),
        1
    );
    assert_eq!(
        reconcile::mark_replayed(&db.admin, &[id])
            .await
            .expect("second mark"),
        0,
        "an already-replayed entry is not replayed again"
    );

    let count: i32 = sqlx::query_scalar("SELECT replay_count FROM dead_events WHERE id = $1")
        .bind(id)
        .fetch_one(&db.admin)
        .await
        .expect("count");
    assert_eq!(count, 1, "the counter moved once, not twice");
}

// ---------------------------------------------------------------------------
// Parked-run reaping
// ---------------------------------------------------------------------------

async fn parked_run(app: &PgPool, org: Uuid, id: Uuid, wake_at: Option<chrono::DateTime<Utc>>) {
    sqlx::query(
        "INSERT INTO workflow_runs
           (id, org_id, rule_name, rule_definition, catalogue_version, status, trigger_kind,
            resume_cursor, wake_at, wake_on)
         VALUES ($1, $2, 'Escalate overdue tickets', '{}'::jsonb, 'test', 'waiting', 'event',
                 '{\"node\": \"flow.delay\"}'::jsonb, $3, 'flow.delay')",
    )
    .bind(id)
    .bind(org)
    .bind(wake_at)
    .execute(app)
    .await
    .expect("seed a parked run");
}

/// The three-way discriminator, which is what keeps this from being a sweep that
/// abandons runs that are merely slow or legitimately waiting on an event:
///
/// * deadline past, wake job dead → unwakeable;
/// * deadline past, wake job still pending → slow, left alone;
/// * deadline future → not due, left alone;
/// * event wait with no deadline → parked by design, never selected.
#[tokio::test]
async fn only_a_run_whose_wake_job_is_gone_is_unwakeable() {
    let db = test_db().await;
    let org = seed_org(&db.admin, 92_060).await;
    let lost = fixed_uuid(92_061);
    let slow = fixed_uuid(92_062);
    let future = fixed_uuid(92_063);
    let event_wait = fixed_uuid(92_064);
    let past = Utc::now() - Duration::seconds(7_200);

    parked_run(&db.admin, org, lost, Some(past)).await;
    parked_run(&db.admin, org, slow, Some(past)).await;
    parked_run(
        &db.admin,
        org,
        future,
        Some(Utc::now() + Duration::hours(1)),
    )
    .await;

    sqlx::query(
        "INSERT INTO workflow_runs
           (id, org_id, rule_name, rule_definition, catalogue_version, status, trigger_kind,
            resume_cursor, wake_on)
         VALUES ($1, $2, 'Wait for closure', '{}'::jsonb, 'test', 'waiting', 'event',
                 '{\"node\": \"flow.waitFor\"}'::jsonb, 'ticket.closed')",
    )
    .bind(event_wait)
    .bind(org)
    .execute(&db.admin)
    .await
    .expect("seed an event park");

    // The slow run still has its wake job, due but unclaimed.
    let mut tx = org_tx(&db.app, org).await;
    queue::enqueue_after(
        &mut *tx,
        &queue::NewJob {
            org_id: org,
            kind: core_db::workflow_runs::WORKFLOW_RESUME_JOB.to_string(),
            payload: json!({ "op": "resume", "run_id": slow }),
            max_attempts: 5,
        },
        past,
    )
    .await
    .expect("enqueue the wake");
    tx.commit().await.expect("commit");

    let found = reconcile::unwakeable_parked_runs(&db.admin, 60.0, 50)
        .await
        .expect("reap scan");
    assert_eq!(found.len(), 1, "exactly one run has no wake path left");
    assert_eq!(found[0].run_id, lost);

    let reaped = reconcile::abandon_unwakeable_runs(
        &db.admin,
        &found,
        "wake_lost",
        "the resume job that would wake this run is dead and past its retry window",
    )
    .await
    .expect("reap");
    assert_eq!(reaped, 1);

    // The status shape CHECK is the real proof: an abandoned row carries
    // `finished_at` and no cursor, wake-at or wake-on.
    // status, error_code, resume_cursor, wake_at, finished_at — the order the
    // query selects them in.
    type AbandonedRun = (
        String,
        Option<String>,
        Option<serde_json::Value>,
        Option<chrono::DateTime<Utc>>,
        Option<chrono::DateTime<Utc>>,
    );
    let (status, error_code, cursor, wake_at, finished): AbandonedRun = sqlx::query_as(
        "SELECT status, error_code, resume_cursor, wake_at, finished_at
           FROM workflow_runs WHERE id = $1",
    )
    .bind(lost)
    .fetch_one(&db.admin)
    .await
    .expect("reaped run");
    assert_eq!(status, "abandoned");
    assert_eq!(error_code.as_deref(), Some("wake_lost"));
    assert!(cursor.is_none() && wake_at.is_none() && finished.is_some());

    // The other three are untouched — including the event wait, which is the
    // case a naive "old waiting run" reaper would have broken.
    let (still_waiting,): (i64,) =
        sqlx::query_as("SELECT count(*)::bigint FROM workflow_runs WHERE status = 'waiting'")
            .fetch_one(&db.admin)
            .await
            .expect("count");
    assert_eq!(still_waiting, 3);
}

// ---------------------------------------------------------------------------
// Report-delivery retry
// ---------------------------------------------------------------------------

async fn seed_schedule(app: &PgPool, org: Uuid, n: u64) -> (Uuid, Uuid) {
    let user = seed_user(app, org, n + 1).await;
    let definition = fixed_uuid(n + 2);
    let schedule = fixed_uuid(n + 3);
    let mut tx = org_tx(app, org).await;
    report_defs::insert(
        &mut tx,
        definition,
        org,
        user,
        &report_defs::DefinitionFields {
            name: "Weekly tickets".into(),
            description: None,
            kind: "tickets".into(),
            spec: json!({ "filters": [], "group_by": [], "measures": [] }),
            card: None,
            viz: "table".into(),
            scope: "org".into(),
        },
    )
    .await
    .expect("definition");
    report_schedules::insert(
        &mut tx,
        schedule,
        org,
        user,
        &report_schedules::ScheduleFields {
            report_definition_id: definition,
            name: "Weekly tickets".into(),
            recipients: vec![user],
            cadence: "weekly".into(),
            time_of_day: chrono::NaiveTime::from_hms_opt(7, 0, 0).expect("time"),
            day_of_week: Some(0),
            day_of_month: None,
            timezone: "UTC".into(),
            window_days: 7,
            enabled: true,
            next_run_at: Utc::now() + Duration::days(7),
        },
    )
    .await
    .expect("schedule");
    tx.commit().await.expect("commit");
    (user, schedule)
}

async fn delivery_row(
    app: &PgPool,
    org: Uuid,
    schedule: Uuid,
    id: Uuid,
    slot: chrono::DateTime<Utc>,
    status: &str,
    recorded: chrono::DateTime<Utc>,
) {
    sqlx::query(
        "INSERT INTO report_deliveries (id, org_id, schedule_id, slot_at, status, detail, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(org)
    .bind(schedule)
    .bind(slot)
    .bind(status)
    .bind(json!({ "delivered": 0, "failed": [{ "user_id": fixed_uuid(92_999), "error": "550" }] }))
    .bind(recorded)
    .execute(app)
    .await
    .expect("seed a delivery row");
}

/// The failure: a slot that failed once and was never claimed again. The newest
/// row per slot is what decides, so a slot that later delivered drops out of the
/// retry set on its own — no flag, no status rewrite.
#[tokio::test]
async fn a_failed_slot_is_due_and_a_recovered_slot_is_not() {
    let db = test_db().await;
    let org = seed_org(&db.admin, 92_070).await;
    let (_user, schedule) = seed_schedule(&db.admin, org, 92_070).await;
    // The slot is compared to the value read back, so it must carry the
    // precision the column stores — see `common::db_now`.
    let slot = db_now() - Duration::minutes(30);
    let failed_slot = slot;
    let recovered_slot = slot - Duration::days(7);

    delivery_row(
        &db.admin,
        org,
        schedule,
        fixed_uuid(92_080),
        failed_slot,
        "failed",
        slot,
    )
    .await;
    // A slot that failed and then recovered: the delivered row is the newest.
    delivery_row(
        &db.admin,
        org,
        schedule,
        fixed_uuid(92_081),
        recovered_slot,
        "partial",
        recovered_slot,
    )
    .await;
    delivery_row(
        &db.admin,
        org,
        schedule,
        fixed_uuid(92_082),
        recovered_slot,
        "delivered",
        recovered_slot + Duration::minutes(2),
    )
    .await;

    let due = reconcile::due_report_retries(&db.admin, 0.0, 50)
        .await
        .expect("retry scan");
    assert_eq!(due.len(), 1, "only the slot whose newest row failed is due");
    assert_eq!(due[0].schedule_id, schedule);
    assert_eq!(due[0].slot_at, failed_slot);
    assert_eq!(due[0].attempts, 1);
    assert_eq!(
        due[0]
            .detail
            .get("failed")
            .and_then(|f| f.as_array())
            .map(Vec::len),
        Some(1),
        "the retry addressee list rides the ledger row"
    );

    // A second attempt for the same slot: the slot stays due (still failing)
    // and its attempt count is what the budget is measured against.
    delivery_row(
        &db.admin,
        org,
        schedule,
        fixed_uuid(92_083),
        failed_slot,
        "failed",
        slot + Duration::minutes(1),
    )
    .await;
    let again = reconcile::due_report_retries(&db.admin, 0.0, 50)
        .await
        .expect("retry scan");
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].attempts, 2);
    assert_eq!(
        reconcile::exhausted_report_retries(&db.admin, 2)
            .await
            .expect("exhausted count"),
        1,
        "two attempts against a budget of two means a person is needed"
    );
    assert_eq!(
        reconcile::exhausted_report_retries(&db.admin, 3)
            .await
            .expect("exhausted count"),
        0,
        "with one attempt left the slot is still the reconciler's work"
    );

    // A failed slot left behind would be picked up by the worker's retry pass in
    // the next suite against this same database, and mail a recipient that no
    // longer exists. Deliveries and schedules cascade from the org.
    sqlx::query("DELETE FROM orgs WHERE id = $1")
        .bind(org)
        .execute(&db.admin)
        .await
        .expect("cleanup");
}
