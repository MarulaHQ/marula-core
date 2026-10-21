//! Live-Postgres proof for the stream-consumer delivery ledger.
//!
//! Purpose: prove the SQL behind the consumer harness — the statements that
//! count per-entry attempts, defer a retry, settle it, and dead-letter an
//! entry that exhausted its budget. These tables are new, so their statements
//! are runtime queries with no `.sqlx` entry; this test is what proves they
//! parse and bind.
//!
//! It also pins the two properties the harness relies on:
//!
//! * a deferred attempt is not "due" until its delay passes, so the loop does
//!   not burn the whole budget in one pass;
//! * dead-lettering is idempotent per `(group, entry)`, so a redelivery after
//!   a lost ack cannot produce a second record of the same abandonment.
//!
//! Non-purpose: Redis, the harness loop itself (unit-tested in
//! `crates/worker/src/consumer.rs`), retention.
//!
//! Database selection follows the contract in `tests/common/mod.rs`.

mod common;

use common::test_db;
use core_db::consumer::{self, Attempt, DeadEventSpec};
use uuid::Uuid;

#[tokio::test]
async fn attempts_defer_settle_and_dead_letter_one_stream_entry() {
    let db = test_db().await;
    let pool = &db.admin;
    let group = "test-group";
    let entry = "1700000000000-0";
    let event_id = Uuid::now_v7();
    let org_id = Uuid::now_v7();
    // The delivery ledger carries a cascading FK to `orgs` since 0146, so the
    // org a failing entry names has to exist.
    sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, $2, $3)")
        .bind(org_id)
        .bind("delivery test")
        .bind(format!("delivery-{org_id}"))
        .execute(pool)
        .await
        .expect("seed org");

    // 1. An entry nobody has failed on has no row and no dead record.
    let states = consumer::delivery_states(pool, group, &[entry.to_string()])
        .await
        .expect("states");
    assert!(states.is_empty(), "a fresh entry has no delivery row");

    // 2. First failure: the row appears, due immediately, with the identity.
    let attempts = consumer::record_attempt(
        pool,
        &Attempt {
            group,
            consumer: "replica-a",
            entry_id: entry,
            org_id: Some(org_id),
            event_id: Some(event_id),
            event_type: Some("ticket.created"),
            error: "first failure",
            retry_in_secs: 0.0,
        },
    )
    .await
    .expect("record first attempt");
    assert_eq!(attempts, 1);

    let state = only_state(pool, group, entry).await;
    assert!(state.due, "a zero delay is due now");
    assert!(!state.dead);
    assert_eq!(state.attempts, 1);

    // 3. Second failure deferred by an hour: recorded, but not due yet, which
    //    is what keeps the harness from spending the whole budget in one pass.
    let attempts = consumer::record_attempt(
        pool,
        &Attempt {
            group,
            consumer: "replica-b",
            entry_id: entry,
            org_id: Some(org_id),
            event_id: Some(event_id),
            event_type: Some("ticket.created"),
            error: "second failure",
            retry_in_secs: 3_600.0,
        },
    )
    .await
    .expect("record second attempt");
    assert_eq!(attempts, 2);
    let state = only_state(pool, group, entry).await;
    assert!(!state.due, "a deferred entry is not due");
    assert_eq!(state.attempts, 2);

    // 4. Dead-lettering records the abandonment and marks the row dead.
    let spec = DeadEventSpec {
        group,
        consumer: "replica-b",
        entry_id: entry,
        org_id: Some(org_id),
        event_id: Some(event_id),
        event_type: Some("ticket.created"),
        attempts,
        last_error: "budget exhausted",
        payload: Some(r#"{"broken":true"#),
    };
    consumer::dead_letter(pool, &spec)
        .await
        .expect("dead-letter");
    let dead = consumer::dead_events(pool, group, 10).await.expect("list");
    assert_eq!(dead.len(), 1);
    assert_eq!(dead[0].entry_id, entry);
    assert_eq!(dead[0].consumer, "replica-b");
    assert_eq!(dead[0].event_id, Some(event_id));
    assert_eq!(dead[0].attempts, 2);
    assert_eq!(dead[0].last_error, "budget exhausted");
    assert_eq!(
        dead[0].payload.as_deref(),
        Some(r#"{"broken":true"#),
        "a payload that is not valid JSON is still kept"
    );

    let state = only_state(pool, group, entry).await;
    assert!(state.dead, "the delivery row carries the dead mark");

    // 5. A redelivered entry dead-letters once, not twice.
    consumer::dead_letter(pool, &spec)
        .await
        .expect("second dead-letter");
    let dead = consumer::dead_events(pool, group, 10).await.expect("list");
    assert_eq!(dead.len(), 1, "one abandonment per entry and group");

    // 6. Settling a previously failed entry clears it from the live set.
    consumer::settle(pool, group, entry).await.expect("settle");
    assert_eq!(
        consumer::live_failure_count(pool, group)
            .await
            .expect("count"),
        0,
        "a settled entry is no longer a live failure"
    );

    // 7. Groups are independent: the same entry id under another group has no
    //    state of its own.
    let other = consumer::delivery_states(pool, "other-group", &[entry.to_string()])
        .await
        .expect("states");
    assert!(other.is_empty(), "one group's failure is not another's");
}

async fn only_state(pool: &sqlx::PgPool, group: &str, entry: &str) -> consumer::DeliveryState {
    let mut states = consumer::delivery_states(pool, group, &[entry.to_string()])
        .await
        .expect("states");
    assert_eq!(states.len(), 1, "expected one delivery row for {entry}");
    states.remove(0)
}
