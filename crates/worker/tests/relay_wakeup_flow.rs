//! Live-database tests for the notification-driven relay (Wave 52 lane P4).
//!
//! Purpose: prove that a COMMITTED outbox insert wakes the relay through the
//! `marula_events` notification, not through the safety poll. The pacing is
//! set to a 30-second poll, so a row published inside a few seconds can only
//! have been woken by the trigger.
//!
//! Non-purpose: the envelope and stream contract (`core-events` unit tests)
//! and the SSE surface (`core-api`'s realtime suite).

use sqlx::PgPool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Reset+migrate ONCE per process, the same contract as the other worker
/// suites: tests share one database, and a pool is created per call because
/// each `#[tokio::test]` owns its runtime.
async fn test_pool() -> (PgPool, String) {
    static BOOTSTRAP: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();
    let url = BOOTSTRAP
        .get_or_init(|| async {
            let url = std::env::var("MARULA_TEST_DATABASE_URL")
                .or_else(|_| std::env::var("DATABASE_URL"))
                .expect(
                    "set MARULA_TEST_DATABASE_URL (or DATABASE_URL) to a privileged \
                     Postgres for the relay wakeup test",
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

async fn published_at(pool: &PgPool, org: Uuid, id: Uuid) -> Option<chrono::DateTime<chrono::Utc>> {
    let mut tx = pool.begin().await.expect("begin probe");
    core_db::set_org(&mut tx, org).await.expect("set_org");
    sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
        "SELECT published_at FROM outbox_events WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .expect("read published_at")
    .flatten()
}

/// A row committed after the relay has taken its first empty pass must be
/// published well inside the 30-second safety poll. The gap between the two
/// numbers is the wakeup.
#[tokio::test]
async fn a_committed_outbox_insert_wakes_the_relay() {
    let (pool, database_url) = test_pool().await;
    let org = Uuid::from_u128(0x6D65_7269_6469_616E_u128 << 64 | 0x52_01);

    // `outbox_events.org_id` is a foreign key, so the org has to exist.
    {
        let mut tx = pool.begin().await.expect("begin seed");
        core_db::set_org(&mut tx, org).await.expect("set_org");
        core_db::orgs::insert_org(&mut *tx, org, "Relay Test", "relay-test", "RLY")
            .await
            .expect("insert org");
        tx.commit().await.expect("commit seed");
    }

    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string());
    let publisher = core_events::Publisher::connect(&redis_url)
        .await
        .expect("connect redis for the relay test");

    let wakeup = Arc::new(tokio::sync::Notify::new());
    let relay = tokio::spawn(core_events::run_relay(
        pool.clone(),
        publisher,
        100,
        core_events::RelayPacing::notified(Duration::from_secs(30), Arc::clone(&wakeup)),
    ));
    let listener = tokio::spawn(core_events::run_wakeup_listener(
        database_url,
        Arc::clone(&wakeup),
    ));

    // Give the listener time to issue LISTEN and the relay time to take its
    // first (empty) pass. A notification fired before LISTEN is active is
    // lost by design — the safety poll covers it — and this test would then
    // be racing the 30-second poll instead of the wakeup.
    tokio::time::sleep(Duration::from_millis(1_500)).await;

    let id = Uuid::now_v7();
    let started = Instant::now();
    {
        let mut tx = pool.begin().await.expect("begin insert");
        core_db::set_org(&mut tx, org).await.expect("set_org");
        core_db::outbox::insert(
            &mut tx,
            id,
            org,
            "ticket.created",
            "ticket",
            None,
            &serde_json::json!({"wakeup_probe": true}),
        )
        .await
        .expect("insert outbox row");
        tx.commit().await.expect("commit outbox row");
    }

    let deadline = Duration::from_secs(5);
    let mut seen = None;
    while started.elapsed() < deadline {
        if let Some(at) = published_at(&pool, org, id).await {
            seen = Some(at);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let elapsed = started.elapsed();

    relay.abort();
    listener.abort();

    assert!(
        seen.is_some(),
        "the relay did not publish within {deadline:?} (elapsed {elapsed:?}); the safety poll \
         is 30s, so the notification did not wake it"
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "published? yes, but after {elapsed:?}, which is not a wakeup"
    );
}
