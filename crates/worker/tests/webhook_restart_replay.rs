//! Live Postgres+Redis proof for Wave 52 lanes P1b (`p52-webhook-restart-replay`)
//! and P1c (`p52-webhook-consumer-name`).
//!
//! Purpose: prove the webhooks-out consumer recovers exactly what it is owed,
//! and that two consumers on one stream never both deliver the same event.
//!
//! P1b's bug: a startup read of `marula:events` from id `0` with no consumer
//! group, which means "every entry the stream still holds": each restart
//! enqueued a fresh `webhook.deliver` job for every resident event.
//!
//! P1c's bug: `CONSUMER_NAME` was the constant `"worker"`, so every replica
//! read the group's single pending list. A drain could take an entry a live
//! sibling had just read, both enqueued a job, and the receiver got two signed
//! POSTs because the job insert had no uniqueness guard.
//!
//! Three phases over a real group on the real stream:
//!
//! 1. **A restart does not replay settled history.** Three events are consumed
//!    and acked, the consumer stops and starts again under the same name while
//!    those entries are still in the stream, and the job count does not move.
//! 2. **A restart does redeliver an unacked entry under a name that survives.**
//!    An entry is handed to a named consumer with `XREADGROUP ... ">"` and
//!    never acked, and the next start under that name queues its delivery.
//! 3. **A different name does not see it, and an idle entry is claimed.**
//!    Another consumer name starts against the same pending entry while it is
//!    fresh and must not deliver it; once the entry is aged past the claim
//!    threshold it is taken over and delivered exactly once. A repeat enqueue
//!    for the same (webhook, event) is refused by the dedupe key.
//!
//! Phases run one at a time behind [`STREAM_LOCK`] because they share the
//! stream key and the group.
//!
//! Non-purpose: HTTP delivery (`signed_delivery_e2e`) and the settle matrix
//! (lib tests).
//!
//! Database selection follows the worker-test contract:
//! `MARULA_TEST_DATABASE_URL` or `DATABASE_URL`, schema reset once per process.
//! Redis is `REDIS_URL` or `redis://localhost:6379`.
//!
//! That Redis must be this binary's alone. A second consumer in `webhooks-out`
//! is exactly what the group is built for, and exactly what breaks these
//! phases: a live `worker` process on the same server reads with `>` between
//! the phase's publish and its own read, and the phase then finds nothing to
//! take. Point `REDIS_URL` at a Valkey of this run's own (`scripts/gate-db.sh
//! up`) and never at a running stack's.

use core_events::{Envelope, EVENT_STREAM};
use redis::streams::{StreamClaimOptions, StreamClaimReply, StreamReadOptions, StreamReadReply};
use redis::AsyncCommands;
use sqlx::PgPool;
use std::time::{Duration, Instant};
use uuid::Uuid;

const GROUP: &str = "webhooks-out";
/// Names standing in for distinct processes. The production name comes from
/// `worker::consumer::consumer_name("webhooks-out")`; naming them here is what
/// lets one test process model a restart that got a new name.
const CONSUMER_A: &str = "replica-a";
const CONSUMER_B: &str = "replica-b";
const CONSUMER_C: &str = "replica-c";
const EVENT_TYPE: &str = "ticket.created";

/// Serializes the phases: they share `EVENT_STREAM` and `GROUP`, and the test
/// binary runs its tests on separate threads.
static STREAM_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn test_pool() -> (PgPool, String) {
    static BOOTSTRAP: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();
    let url = BOOTSTRAP
        .get_or_init(|| async {
            let url = std::env::var("MARULA_TEST_DATABASE_URL")
                .or_else(|_| std::env::var("DATABASE_URL"))
                .expect(
                    "set MARULA_TEST_DATABASE_URL (or DATABASE_URL) to a privileged \
                     Postgres for the webhook restart replay test",
                );
            let admin = core_db::connect(&url).await.expect("connect");
            sqlx::query(
            "DO $$ BEGIN DROP SCHEMA IF EXISTS ops CASCADE; DROP SCHEMA IF EXISTS public CASCADE; END $$",
        )
                .execute(&admin)
                .await
                .expect("drop schema");
            sqlx::query("CREATE SCHEMA public")
                .execute(&admin)
                .await
                .expect("create schema");
            core_db::run_migrations(&admin).await.expect("migrations");
            url
        })
        .await
        .clone();
    (core_db::connect(&url).await.expect("pool"), url)
}

fn redis_url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string())
}

async fn connect_redis(url: &str) -> redis::aio::MultiplexedConnection {
    redis::Client::open(url)
        .expect("redis url")
        .get_multiplexed_async_connection()
        .await
        .expect("connect redis")
}

fn envelope(org: Uuid) -> Envelope {
    Envelope {
        id: Uuid::now_v7(),
        event_type: EVENT_TYPE.to_string(),
        org_id: org,
        entity: "ticket".to_string(),
        entity_id: None,
        occurred_at: chrono::Utc::now(),
        payload_schema_ref: "core-domain/v1".to_string(),
        payload: serde_json::json!({"replay_probe": true}),
    }
}

/// How many `webhook.deliver` jobs this subscription has queued, read inside
/// an org-scoped transaction so the answer is the same whether or not the test
/// role bypasses RLS.
async fn delivery_jobs(pool: &PgPool, org: Uuid, webhook: Uuid) -> i64 {
    let mut tx = pool.begin().await.expect("begin count");
    core_db::set_org(&mut tx, org).await.expect("set_org");
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM jobs WHERE kind = $1 AND payload->>'webhook_id' = $2",
    )
    .bind(worker::webhook_out::JOB_KIND)
    .bind(webhook.to_string())
    .fetch_one(&mut *tx)
    .await
    .expect("count webhook jobs")
}

async fn wait_for_jobs(pool: &PgPool, org: Uuid, webhook: Uuid, expected: i64) -> i64 {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let count = delivery_jobs(pool, org, webhook).await;
        if count >= expected || Instant::now() > deadline {
            return count;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Entry ids sitting unacked in one consumer's PEL. The read uses id `"0"`,
/// which returns that consumer's pending entries and changes nothing.
async fn pending_ids(conn: &mut redis::aio::MultiplexedConnection, consumer: &str) -> Vec<String> {
    let options = StreamReadOptions::default()
        .count(128)
        .group(GROUP, consumer);
    let reply: StreamReadReply = conn
        .xread_options(&[EVENT_STREAM], &["0"], &options)
        .await
        .expect("read pending");
    reply
        .keys
        .into_iter()
        .flat_map(|key| key.ids)
        .map(|entry| entry.id)
        .collect()
}

/// Starts one named consumer and returns its stop handle.
fn start_consumer(
    pool: &PgPool,
    url: &str,
    name: &str,
) -> (
    tokio::sync::watch::Sender<bool>,
    tokio::task::JoinHandle<()>,
) {
    let (stop, stop_rx) = tokio::sync::watch::channel(false);
    let handle = tokio::spawn(worker::webhook_out::run_webhook_out_as(
        pool.clone(),
        url.to_string(),
        name.to_string(),
        stop_rx,
    ));
    (stop, handle)
}

/// Stops a consumer and waits for it, so nothing outlives the phase.
async fn stop_consumer(
    stop: tokio::sync::watch::Sender<bool>,
    handle: tokio::task::JoinHandle<()>,
) {
    let _ = stop.send(true);
    handle.await.expect("webhook consumer task");
}

async fn stream_len(conn: &mut redis::aio::MultiplexedConnection) -> i64 {
    redis::cmd("XLEN")
        .arg(EVENT_STREAM)
        .query_async(conn)
        .await
        .expect("xlen")
}

/// The group's own account of itself, rendered for a `>` read that came back
/// empty.
///
/// `>` returns nothing for two reasons only. Either the group's cursor is
/// already at the tail — `lag` is 0 and `last-delivered-id` is the newest entry
/// the stream holds — or the entry was delivered to some consumer before this
/// read, and only a consumer moves that cursor. The consumer list is what names
/// the second case, which is the one worth printing: an idle consumer holding
/// the entry is a sibling this binary did not start.
async fn group_state(conn: &mut redis::aio::MultiplexedConnection) -> String {
    let entries = stream_len(conn).await;
    let groups: redis::streams::StreamInfoGroupsReply = match conn.xinfo_groups(EVENT_STREAM).await
    {
        Ok(reply) => reply,
        Err(error) => {
            return format!("stream holds {entries} entries; xinfo groups failed: {error}")
        }
    };
    let Some(group) = groups.groups.iter().find(|group| group.name == GROUP) else {
        return format!("stream holds {entries} entries and has no {GROUP} group");
    };
    let consumers: redis::streams::StreamInfoConsumersReply =
        match conn.xinfo_consumers(EVENT_STREAM, GROUP).await {
            Ok(reply) => reply,
            Err(error) => return format!("{GROUP} cursor: xinfo consumers failed: {error}"),
        };
    let owners: Vec<String> = consumers
        .consumers
        .iter()
        .map(|owner| {
            format!(
                "{} ({} pending, idle {} ms)",
                owner.name, owner.pending, owner.idle
            )
        })
        .collect();
    format!(
        "stream holds {entries} entries; {GROUP} is at last-delivered-id {} \
         with {} pending and lag {:?}; consumers: [{}]",
        group.last_delivered_id,
        group.pending,
        group.lag,
        owners.join(", ")
    )
}

/// One org plus one human-created subscription for [`EVENT_TYPE`].
async fn seed_subscription(pool: &PgPool, org: Uuid, webhook: Uuid) {
    let mut tx = pool.begin().await.expect("begin seed");
    core_db::set_org(&mut tx, org).await.expect("set_org");
    core_db::orgs::insert_org(
        &mut *tx,
        org,
        "Restart Replay",
        &format!("restart-replay-{org}"),
        "RRP",
    )
    .await
    .expect("insert org");
    let user = core_db::users::create_user(
        &mut *tx,
        Uuid::now_v7(),
        org,
        &format!("replay-{org}@marula.test"),
        "Replay",
        "not-a-password",
        "UTC",
    )
    .await
    .expect("insert user");
    core_db::webhooks::insert_webhook(
        &mut *tx,
        webhook,
        org,
        "https://receiver.test/marula",
        b"sealed-by-the-signing-path",
        &[EVENT_TYPE.to_string()],
        user.id,
        None,
    )
    .await
    .expect("insert webhook");
    tx.commit().await.expect("commit seed");
}

/// A clean stream with the group already at `0`: entries published from here
/// on are undelivered, so the `>` tail is what picks them up and a group-less
/// head read would be the only way to see them twice.
async fn reset_stream(conn: &mut redis::aio::MultiplexedConnection) {
    let _: i64 = conn.del(EVENT_STREAM).await.expect("del stream");
    let _: () = conn
        .xgroup_create_mkstream(EVENT_STREAM, GROUP, "0")
        .await
        .expect("create group");
}

/// The dedupe key the consumer writes for one (subscription, event) pair.
/// Duplicated from `webhook_out.rs` on purpose: an integration test that
/// imported the format could not notice the format changing.
fn dedupe_key(webhook: Uuid, event: Uuid) -> String {
    format!("webhook.deliver:{webhook}:{event}")
}

/// The dedupe key recorded on the job for one event, if there is one.
async fn job_dedupe_key(pool: &PgPool, org: Uuid, event: Uuid) -> Option<String> {
    let mut tx = pool.begin().await.expect("begin key read");
    core_db::set_org(&mut tx, org).await.expect("set_org");
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT dedupe_key FROM jobs WHERE payload->>'event_id' = $1 LIMIT 1",
    )
    .bind(event.to_string())
    .fetch_optional(&mut *tx)
    .await
    .expect("read dedupe key")
    .flatten()
}

#[tokio::test]
async fn a_restart_redelivers_only_the_unacked_entries() {
    let _serial = STREAM_LOCK.lock().await;
    let (pool, _database_url) = test_pool().await;
    let url = redis_url();
    let mut seed = connect_redis(&url).await;
    reset_stream(&mut seed).await;

    let org = Uuid::now_v7();
    let webhook = Uuid::now_v7();
    seed_subscription(&pool, org, webhook).await;

    // Three events, all matching the subscription.
    let published: Vec<Envelope> = (0..3).map(|_| envelope(org)).collect();
    {
        let mut publisher = core_events::Publisher::connect(&url)
            .await
            .expect("connect publisher");
        for event in &published {
            publisher.publish(event).await.expect("publish");
        }
    }
    assert_eq!(
        stream_len(&mut seed).await,
        3,
        "the stream must hold all three entries, or the regression cannot reproduce"
    );

    // --- phase 1: one run consumes and settles, the next must not replay ---
    let (stop, consumer) = start_consumer(&pool, &url, CONSUMER_A);
    let first = wait_for_jobs(&pool, org, webhook, 3).await;
    stop_consumer(stop, consumer).await;
    assert_eq!(
        first, 3,
        "the first run must queue one delivery per matching event"
    );
    assert!(
        pending_ids(&mut seed, CONSUMER_A).await.is_empty(),
        "a consumed and acked event must leave the PEL"
    );

    // Restart against the same stream. The entries are still resident — XACK
    // removes them from the PEL, not from the stream — so the old head read
    // would enqueue three more jobs here.
    assert_eq!(stream_len(&mut seed).await, 3);
    let (stop, consumer) = start_consumer(&pool, &url, CONSUMER_A);
    // Longer than one BLOCK_MS cycle, so a head replay would have landed.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let after_restart = delivery_jobs(&pool, org, webhook).await;
    stop_consumer(stop, consumer).await;
    assert_eq!(
        after_restart, 3,
        "a restart must not re-enqueue settled events still resident in the stream"
    );

    // --- phase 2: a delivered-but-unacked entry is recovered ---
    //
    // The entry is delivered to `CONSUMER_A` and the restart runs as the same
    // name, which is the case where recovery is the own-PEL drain. Phase 3
    // covers the other case: the crash that comes back under a new name.
    {
        let mut publisher = core_events::Publisher::connect(&url)
            .await
            .expect("connect publisher");
        publisher
            .publish(&envelope(org))
            .await
            .expect("publish crash-shape event");
    }
    // Take delivery without acking: the crash shape, one step past the read.
    let options = StreamReadOptions::default()
        .count(1)
        .group(GROUP, CONSUMER_A);
    let delivered: StreamReadReply = seed
        .xread_options(&[EVENT_STREAM], &[">"], &options)
        .await
        .expect("deliver to the group");
    let handed_over = delivered.keys.into_iter().flat_map(|key| key.ids).count();
    assert_eq!(
        handed_over,
        1,
        "the group must take the new entry; {}",
        group_state(&mut seed).await
    );
    assert_eq!(
        pending_ids(&mut seed, CONSUMER_A).await.len(),
        1,
        "the delivered entry must sit in the PEL, not be acked"
    );
    assert_eq!(
        delivery_jobs(&pool, org, webhook).await,
        3,
        "nothing queues a delivery until a consumer handles the pending entry"
    );

    let (stop, consumer) = start_consumer(&pool, &url, CONSUMER_A);
    let recovered = wait_for_jobs(&pool, org, webhook, 4).await;
    // Settle before the final count so a duplicate enqueue would show up.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let settled = delivery_jobs(&pool, org, webhook).await;
    stop_consumer(stop, consumer).await;
    assert_eq!(
        recovered, 4,
        "a delivered-but-unacked entry must be redelivered after a restart"
    );
    assert_eq!(
        settled, 4,
        "recovery must queue that entry once, not once per read"
    );
    assert!(
        pending_ids(&mut seed, CONSUMER_A).await.is_empty(),
        "the recovered entry must be acked once its delivery is queued"
    );
}

#[tokio::test]
async fn consumer_names_split_the_pending_list_and_idle_entries_are_claimed() {
    let _serial = STREAM_LOCK.lock().await;
    let (pool, _database_url) = test_pool().await;
    let url = redis_url();
    let mut seed = connect_redis(&url).await;
    reset_stream(&mut seed).await;

    let org = Uuid::now_v7();
    let webhook = Uuid::now_v7();
    seed_subscription(&pool, org, webhook).await;

    // Three events consumed and acked by `CONSUMER_A`: the settled baseline.
    {
        let mut publisher = core_events::Publisher::connect(&url)
            .await
            .expect("connect publisher");
        for _ in 0..3 {
            publisher
                .publish(&envelope(org))
                .await
                .expect("publish baseline event");
        }
    }
    let (stop, consumer) = start_consumer(&pool, &url, CONSUMER_A);
    assert_eq!(
        wait_for_jobs(&pool, org, webhook, 3).await,
        3,
        "the baseline consumer must queue three deliveries"
    );
    stop_consumer(stop, consumer).await;
    assert!(
        pending_ids(&mut seed, CONSUMER_A).await.is_empty(),
        "the baseline consumer settled everything it read"
    );

    // One more event, read by `CONSUMER_B` with `>` and left unacked: a live
    // replica mid-handling, or a replica that died right there.
    let in_flight = envelope(org);
    let in_flight_id = in_flight.id;
    {
        let mut publisher = core_events::Publisher::connect(&url)
            .await
            .expect("connect publisher");
        publisher
            .publish(&in_flight)
            .await
            .expect("publish in-flight event");
    }
    let options = StreamReadOptions::default()
        .count(1)
        .group(GROUP, CONSUMER_B);
    let delivered: StreamReadReply = seed
        .xread_options(&[EVENT_STREAM], &[">"], &options)
        .await
        .expect("deliver to CONSUMER_B");
    // A PEL holds STREAM entry ids, not the envelope's uuid: the id the entry
    // was given when it was appended. The two are different namespaces, and
    // the assertions below compare against the right one.
    let delivered_ids: Vec<String> = delivered
        .keys
        .into_iter()
        .flat_map(|key| key.ids)
        .map(|entry| entry.id)
        .collect();
    assert_eq!(
        delivered_ids.len(),
        1,
        "the group must hand the new entry to CONSUMER_B; {}",
        group_state(&mut seed).await
    );
    let delivered_id = delivered_ids[0].clone();
    assert_eq!(
        pending_ids(&mut seed, CONSUMER_B).await,
        vec![delivered_id.clone()],
        "the entry is CONSUMER_B's, not the group's"
    );

    // --- the property: a second name neither sees a settled entry nor takes a
    // fresh one from a sibling ---
    //
    // Before P1c every name read this same PEL, so `CONSUMER_C` would have
    // drained `CONSUMER_B`'s entry and enqueued a duplicate delivery.
    let (stop, consumer) = start_consumer(&pool, &url, CONSUMER_C);
    tokio::time::sleep(Duration::from_secs(3)).await;
    let during = delivery_jobs(&pool, org, webhook).await;
    stop_consumer(stop, consumer).await;
    assert_eq!(
        during, 3,
        "a fresh name must not deliver an entry another consumer owns, nor re-deliver the settled ones"
    );
    assert_eq!(
        pending_ids(&mut seed, CONSUMER_C).await,
        Vec::<String>::new(),
        "CONSUMER_C must own nothing: it neither took the entry nor kept a copy"
    );
    assert_eq!(
        pending_ids(&mut seed, CONSUMER_B).await,
        vec![delivered_id.clone()],
        "the entry must still belong to CONSUMER_B, unacked"
    );

    // --- the claim: once the same entry is idle past the threshold it belongs
    // to no one alive, and the next name takes it over ---
    //
    // XCLAIM sets the idle time directly, so the test exercises the real
    // threshold instead of waiting a minute for it. `CONSUMER_B` is the owner
    // in the PEL; from here it stands for a consumer that will never ack.
    let aged: StreamClaimReply = seed
        .xclaim_options(
            EVENT_STREAM,
            GROUP,
            CONSUMER_B,
            0_u64,
            // clippy 1.95 (cloned_ref_to_slice_refs): same one-element slice,
            // no clone. Pre-existing lint on `main`, fixed on the group
            // branch so the group gate can be green (see MERGE-NOTES.md).
            std::slice::from_ref(&delivered_id),
            StreamClaimOptions::default().idle(120_000).with_force(),
        )
        .await
        .expect("age the abandoned entry");
    assert_eq!(
        aged.ids.len(),
        1,
        "the entry must stay pending under the dead name, aged past the threshold"
    );

    let (stop, consumer) = start_consumer(&pool, &url, CONSUMER_C);
    let claimed = wait_for_jobs(&pool, org, webhook, 4).await;
    // Settle before the final count so a double enqueue would show up.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let settled = delivery_jobs(&pool, org, webhook).await;
    stop_consumer(stop, consumer).await;
    assert_eq!(
        claimed, 4,
        "an entry abandoned by a dead consumer must be claimed and delivered"
    );
    assert_eq!(
        settled, 4,
        "the claim must queue one delivery, not one per claim page"
    );
    assert!(
        pending_ids(&mut seed, CONSUMER_C).await.is_empty(),
        "the claimed entry must be acked by the consumer that took it over"
    );
    assert_eq!(
        job_dedupe_key(&pool, org, in_flight_id).await,
        Some(dedupe_key(webhook, in_flight_id)),
        "the claimed delivery must carry the (subscription, event) dedupe key"
    );

    // --- the guard: the same pair cannot be queued twice ---
    //
    // This is what a second consumer's enqueue for an entry it also read
    // becomes: not a second row, and so not a second signed POST.
    let repeat = core_db::queue::NewJob {
        org_id: org,
        kind: worker::webhook_out::JOB_KIND.to_string(),
        payload: serde_json::json!({
            "webhook_id": webhook,
            "event_id": in_flight_id,
            "envelope": serde_json::Value::Null,
        }),
        max_attempts: 6,
    };
    let again = core_db::queue::enqueue_deduped(&pool, &repeat, &dedupe_key(webhook, in_flight_id))
        .await
        .expect("dedupe insert");
    assert!(
        again.is_none(),
        "a second enqueue for a queued (webhook, event) must insert nothing"
    );
    assert_eq!(
        delivery_jobs(&pool, org, webhook).await,
        4,
        "the refused enqueue must leave the job count alone"
    );
}
