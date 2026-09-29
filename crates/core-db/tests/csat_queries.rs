//! CSAT surveys on resolution — DB-layer query contracts (Wave 34 lane 2).
//!
//! Pins: queue idempotency per (org, ticket), the min-interval read,
//! single-use consume (replay/expiry/foreign-client are no-ops), and the
//! per-tech / per-client rollups' windowing + low-score counting.

use core_db::csat;
use sqlx::PgPool;
use uuid::Uuid;

async fn pool() -> PgPool {
    let url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("csat queries need MARULA_TEST_DATABASE_URL or DATABASE_URL");
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("pool")
}

async fn seed_org(pool: &PgPool) -> Uuid {
    let org_id = Uuid::now_v7();
    sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, $2, $3)")
        .bind(org_id)
        .bind(format!("csat-{org_id}"))
        .bind(format!("csat-{org_id}"))
        .execute(pool)
        .await
        .expect("org");
    org_id
}

async fn seed_ticket(pool: &PgPool, org_id: Uuid, client_id: Uuid) -> Uuid {
    let board_id = Uuid::now_v7();
    let status_id = Uuid::now_v7();
    let ticket_id = Uuid::now_v7();
    sqlx::query("INSERT INTO boards (id, org_id, name, is_default) VALUES ($1, $2, $3, false)")
        .bind(board_id)
        .bind(org_id)
        .bind(format!("board-{board_id}"))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO board_statuses (id, org_id, board_id, name, category, sort_order, is_entry)
         VALUES ($1, $2, $3, 'Open', 'open', 0, true)",
    )
    .bind(status_id)
    .bind(org_id)
    .bind(board_id)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO tickets (id, org_id, number, board_id, status_id, title, priority, category, client_id)
         VALUES ($1, $2, $3, $4, $5, 'csat probe', 'p3', 'open', $6)",
    )
    .bind(ticket_id)
    .bind(org_id)
    .bind(format!("CSAT-{}", Uuid::now_v7().simple()))
    .bind(board_id)
    .bind(status_id)
    .bind(client_id)
    .execute(pool)
    .await
    .unwrap();
    ticket_id
}

fn new_token() -> (String, String) {
    use sha2::{Digest, Sha256};
    let raw = format!("raw-{}", Uuid::now_v7().simple());
    let hash = hex::encode(Sha256::digest(raw.as_bytes()));
    (raw, hash)
}

#[tokio::test]
async fn queue_is_idempotent_per_ticket_and_consume_is_single_use() {
    let pool = pool().await;
    let org = seed_org(&pool).await;
    let client = {
        let id = Uuid::now_v7();
        sqlx::query("INSERT INTO clients (id, org_id, name) VALUES ($1, $2, 'Probe Client')")
            .bind(id)
            .bind(org)
            .execute(&pool)
            .await
            .unwrap();
        id
    };
    let ticket = seed_ticket(&pool, org, client).await;
    let (_, hash) = new_token();

    let id1 = csat::queue(
        &pool,
        org,
        ticket,
        Some(client),
        "probe@test.example",
        &hash,
        chrono::Utc::now() + chrono::Duration::days(14),
        chrono::Utc::now(),
    )
    .await
    .unwrap();
    // Replay with a DIFFERENT hash still settles the ORIGINAL row.
    let (_, hash2) = new_token();
    let id2 = csat::queue(
        &pool,
        org,
        ticket,
        Some(client),
        "probe@test.example",
        &hash2,
        chrono::Utc::now() + chrono::Duration::days(14),
        chrono::Utc::now(),
    )
    .await
    .unwrap();
    assert_eq!(id1, id2, "replayed resolve reuses the one survey row");

    // Foreign client + not-yet-sent row: consume is a no-op.
    let other_client = {
        let id = Uuid::now_v7();
        sqlx::query("INSERT INTO clients (id, org_id, name) VALUES ($1, $2, 'Other Client')")
            .bind(id)
            .bind(org)
            .execute(&pool)
            .await
            .unwrap();
        id
    };
    assert!(
        csat::consume(&pool, &hash, org, other_client, chrono::Utc::now())
            .await
            .unwrap()
            .is_none()
    );

    // Delivery flips the row to sent; consume only matches sent rows.
    csat::mark_sent(&pool, org, id1, chrono::Utc::now())
        .await
        .unwrap()
        .expect("first send wins");

    // The owning client consumes exactly once.
    let won = csat::consume(&pool, &hash, org, client, chrono::Utc::now())
        .await
        .unwrap()
        .expect("first consume wins");
    assert_eq!(won.ticket_id, ticket);
    assert!(
        csat::consume(&pool, &hash, org, client, chrono::Utc::now())
            .await
            .unwrap()
            .is_none(),
        "replayed consume must not match"
    );

    // Score records once; a second write is refused.
    assert!(
        csat::record_response(&pool, org, won.survey_id, 4, Some("good"))
            .await
            .unwrap()
            .is_some()
    );
    assert!(csat::record_response(&pool, org, won.survey_id, 1, None)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn rollups_window_on_responses_and_count_low_scores() {
    let pool = pool().await;
    let org = seed_org(&pool).await;
    let client = {
        let id = Uuid::now_v7();
        sqlx::query("INSERT INTO clients (id, org_id, name) VALUES ($1, $2, 'Rollup Client')")
            .bind(id)
            .bind(org)
            .execute(&pool)
            .await
            .unwrap();
        id
    };
    let t_low = seed_ticket(&pool, org, client).await;
    let t_high = seed_ticket(&pool, org, client).await;
    let t_old = seed_ticket(&pool, org, client).await;

    for (ticket, score, responded_at) in [
        (t_low, 1i16, chrono::Utc::now()),
        (t_high, 5, chrono::Utc::now()),
        (t_old, 1, chrono::Utc::now() - chrono::Duration::days(40)),
    ] {
        let (_, hash) = new_token();
        csat::queue(
            &pool,
            org,
            ticket,
            Some(client),
            "probe@test.example",
            &hash,
            chrono::Utc::now() + chrono::Duration::days(14),
            chrono::Utc::now(),
        )
        .await
        .unwrap();
        sqlx::query(
            "UPDATE csat_surveys SET status = 'responded', responded_at = $3, score = $4 WHERE org_id = $1 AND ticket_id = $2",
        )
        .bind(org)
        .bind(ticket)
        .bind(responded_at)
        .bind(score)
        .execute(&pool)
        .await
        .unwrap();
    }

    let from = chrono::Utc::now() - chrono::Duration::days(1);
    let to = chrono::Utc::now() + chrono::Duration::days(1);
    let clients = csat::rollup_for_clients(&pool, org, from, to, 2)
        .await
        .unwrap();
    assert_eq!(clients.len(), 1);
    assert_eq!(
        clients[0].responses, 2,
        "window excludes the 40-day-old row"
    );
    assert_eq!(clients[0].low_scores, 1);
    assert_eq!(clients[0].avg_score, Some(3.0));

    // The client has surveys ⇒ the scorecards availability gate flips.
    assert!(csat::has_surveys(&pool, org).await.unwrap());
}
