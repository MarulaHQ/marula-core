//! Live Postgres+Redis proof for Wave 52 lane P1b
//! (`p52-webhook-restart-replay`).
//!
//! Purpose: prove the webhooks-out consumer recovers exactly what it is owed.
//! The bug was a startup read of `marula:events` from id `0` with no consumer
//! group, which means "every entry the stream still holds": each restart
//! enqueued a fresh `webhook.deliver` job for every resident event.
//!
//! One test, two phases, both against a real group on the real stream:
//!
//! 1. **A restart does not replay settled history.** Three events are consumed
//!    and acked, the consumer stops and starts again while those entries are
//!    still in the stream, and the job count does not move.
//! 2. **A restart does redeliver an unacked entry.** An entry is handed to the
//!    consumer with `XREADGROUP ... ">"` and never acked — the crash shape —
//!    and the next start queues its delivery.
//!
//! Both phases run in one `#[tokio::test]` because they share the stream and
//! the group; two tests in this binary would race over the same keys.
//!
//! Non-purpose: HTTP delivery (`signed_delivery_e2e`), the settle matrix
//! (lib tests), and the shared-PEL behaviour of several replicas at once,
//! which needs two processes and is described in `REVIEW.md`.
//!
//! Database selection follows the worker-test contract:
//! `MARULA_TEST_DATABASE_URL` or `DATABASE_URL`, schema reset once per process.
//! Redis is `REDIS_URL` or `redis://localhost:6379`.

use core_events::{Envelope, EVENT_STREAM};
use redis::streams::{StreamReadOptions, StreamReadReply};
use redis::AsyncCommands;
use sqlx::PgPool;
use std::time::{Duration, Instant};
use uuid::Uuid;

const GROUP: &str = "webhooks-out";
const CONSUMER: &str = "worker";
const EVENT_TYPE: &str = "ticket.created";

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
            sqlx::query("DROP SCHEMA public CASCADE")
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

/// Entry ids sitting unacked in this consumer's PEL. The read uses id `"0"`,
/// which returns pending entries and changes nothing.
async fn pending_ids(conn: &mut redis::aio::MultiplexedConnection) -> Vec<String> {
    let options = StreamReadOptions::default()
        .count(128)
        .group(GROUP, CONSUMER);
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

async fn stream_len(conn: &mut redis::aio::MultiplexedConnection) -> i64 {
    redis::cmd("XLEN")
        .arg(EVENT_STREAM)
        .query_async(conn)
        .await
        .expect("xlen")
}

#[tokio::test]
async fn a_restart_redelivers_only_the_unacked_entries() {
    let (pool, _database_url) = test_pool().await;
    let url = redis_url();
    let mut seed = connect_redis(&url).await;

    // A clean stream with the group already at `0`: entries published from
    // here on are undelivered, so the `>` tail is what picks them up and a
    // group-less head read would be the only way to see them twice.
    let _: i64 = seed.del(EVENT_STREAM).await.expect("del stream");
    let _: () = seed
        .xgroup_create_mkstream(EVENT_STREAM, GROUP, "0")
        .await
        .expect("create group");

    // One org, one human-created subscription for EVENT_TYPE.
    let org = Uuid::now_v7();
    let webhook = Uuid::now_v7();
    {
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
    let (stop, stop_rx) = tokio::sync::watch::channel(false);
    let consumer = tokio::spawn(worker::webhook_out::run_webhook_out(
        pool.clone(),
        url.clone(),
        stop_rx,
    ));
    let first = wait_for_jobs(&pool, org, webhook, 3).await;
    let _ = stop.send(true);
    consumer.await.expect("webhook consumer task");
    assert_eq!(
        first, 3,
        "the first run must queue one delivery per matching event"
    );
    assert!(
        pending_ids(&mut seed).await.is_empty(),
        "a consumed and acked event must leave the PEL"
    );

    // Restart against the same stream. The entries are still resident — XACK
    // removes them from the PEL, not from the stream — so the old head read
    // would enqueue three more jobs here.
    assert_eq!(stream_len(&mut seed).await, 3);
    let (stop, stop_rx) = tokio::sync::watch::channel(false);
    let consumer = tokio::spawn(worker::webhook_out::run_webhook_out(
        pool.clone(),
        url.clone(),
        stop_rx,
    ));
    // Longer than one BLOCK_MS cycle, so a head replay would have landed.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let after_restart = delivery_jobs(&pool, org, webhook).await;
    let _ = stop.send(true);
    consumer.await.expect("webhook consumer task");
    assert_eq!(
        after_restart, 3,
        "a restart must not re-enqueue settled events still resident in the stream"
    );

    // --- phase 2: a delivered-but-unacked entry is recovered ---
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
    let options = StreamReadOptions::default().count(1).group(GROUP, CONSUMER);
    let delivered: StreamReadReply = seed
        .xread_options(&[EVENT_STREAM], &[">"], &options)
        .await
        .expect("deliver to the group");
    let handed_over = delivered.keys.into_iter().flat_map(|key| key.ids).count();
    assert_eq!(handed_over, 1, "the group must take the new entry");
    assert_eq!(
        pending_ids(&mut seed).await.len(),
        1,
        "the delivered entry must sit in the PEL, not be acked"
    );
    assert_eq!(
        delivery_jobs(&pool, org, webhook).await,
        3,
        "nothing queues a delivery until a consumer handles the pending entry"
    );

    let (stop, stop_rx) = tokio::sync::watch::channel(false);
    let consumer = tokio::spawn(worker::webhook_out::run_webhook_out(
        pool.clone(),
        url.clone(),
        stop_rx,
    ));
    let recovered = wait_for_jobs(&pool, org, webhook, 4).await;
    // Settle before the final count so a duplicate enqueue would show up.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let settled = delivery_jobs(&pool, org, webhook).await;
    let _ = stop.send(true);
    consumer.await.expect("webhook consumer task");
    assert_eq!(
        recovered, 4,
        "a delivered-but-unacked entry must be redelivered after a restart"
    );
    assert_eq!(
        settled, 4,
        "recovery must queue that entry once, not once per read"
    );
    assert!(
        pending_ids(&mut seed).await.is_empty(),
        "the recovered entry must be acked once its delivery is queued"
    );
}
