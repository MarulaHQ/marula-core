//! Batched purchase-order line reads — DB-layer query contracts (Wave 41 lane 9).
//!
//! `list_po_lines_for_orders` exists so the order LIST can price every row from
//! one query instead of one per order. Two properties are worth pinning because
//! a bug in either is silent rather than loud:
//!
//! 1. **Org scoping.** A batch read must never return another org's lines, or a
//!    ledger's money column would silently show a stranger's order value. The
//!    batch must be exactly as narrow as the single-order read it replaces.
//! 2. **Grouping + order.** Every line must come back, attributed to its own
//!    order, in the same `position` order the single-order read gives, because
//!    the caller folds the lines into a per-order rollup in memory. A dropped
//!    or misattributed line is a wrong total, not an error.

use core_db::purchasing::{list_po_lines, list_po_lines_for_orders};
use sqlx::PgPool;
use uuid::Uuid;

async fn pool() -> PgPool {
    let url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("purchasing queries need MARULA_TEST_DATABASE_URL or DATABASE_URL");
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
        .bind(format!("po-{org_id}"))
        .bind(format!("po-{org_id}"))
        .execute(pool)
        .await
        .expect("org");
    org_id
}

async fn seed_vendor(pool: &PgPool, org_id: Uuid) -> Uuid {
    let vendor_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO vendors (id, org_id, name, default_currency, active)
         VALUES ($1, $2, $3, 'USD', true)",
    )
    .bind(vendor_id)
    .bind(org_id)
    .bind(format!("vendor-{vendor_id}"))
    .execute(pool)
    .await
    .expect("vendor");
    vendor_id
}

async fn seed_order(pool: &PgPool, org_id: Uuid, vendor_id: Uuid) -> Uuid {
    let order_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO purchase_orders (id, org_id, vendor_id, number, status, currency)
         VALUES ($1, $2, $3, $4, 'ordered', 'USD')",
    )
    .bind(order_id)
    .bind(org_id)
    .bind(vendor_id)
    .bind(format!("PO-{}", Uuid::now_v7().simple()))
    .execute(pool)
    .await
    .expect("order");
    order_id
}

/// Two lines on one order, inserted out of position order so a query that
/// forgot `ORDER BY position` would visibly differ.
async fn seed_line(
    pool: &PgPool,
    org_id: Uuid,
    order_id: Uuid,
    position: i32,
    qty_ordered_milli: i64,
    unit_cost_minor: i64,
) {
    sqlx::query(
        "INSERT INTO purchase_order_lines
           (id, org_id, purchase_order_id, description, position, qty_ordered_milli,
            qty_received_milli, unit_cost_minor, receipt_state)
         VALUES ($1, $2, $3, $4, $5, $6, 0, $7, 'none')",
    )
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(order_id)
    .bind(format!("line at position {position}"))
    .bind(position)
    .bind(qty_ordered_milli)
    .bind(unit_cost_minor)
    .execute(pool)
    .await
    .expect("line");
}

#[tokio::test]
async fn batch_read_returns_every_line_attributed_to_its_own_order() {
    let pool = pool().await;
    let org_id = seed_org(&pool).await;
    let vendor_id = seed_vendor(&pool, org_id).await;

    let first = seed_order(&pool, org_id, vendor_id).await;
    let second = seed_order(&pool, org_id, vendor_id).await;

    // Inserted high-position-first so the position ordering is actually tested.
    seed_line(&pool, org_id, first, 2, 3_000, 250).await;
    seed_line(&pool, org_id, first, 1, 1_000, 100).await;
    seed_line(&pool, org_id, second, 1, 5_000, 700).await;

    let mut conn = pool.acquire().await.expect("conn");
    let batched = list_po_lines_for_orders(&mut conn, org_id, &[first, second])
        .await
        .expect("batched read");

    assert_eq!(
        batched.len(),
        3,
        "every line of every requested order comes back: {batched:#?}"
    );

    // Same lines, same order, as the single-order read the list replaced.
    for order_id in [first, second] {
        let single = list_po_lines(&mut conn, org_id, order_id)
            .await
            .expect("single read");
        let from_batch: Vec<_> = batched
            .iter()
            .filter(|line| line.purchase_order_id == order_id)
            .collect();
        assert_eq!(
            from_batch.len(),
            single.len(),
            "a batched order's line count equals its single read"
        );
        for (batched_line, single_line) in from_batch.iter().zip(single.iter()) {
            assert_eq!(batched_line.id, single_line.id, "same line, same order");
            assert_eq!(
                batched_line.position, single_line.position,
                "position order matches the single read"
            );
            assert_eq!(
                batched_line.unit_cost_minor, single_line.unit_cost_minor,
                "money is carried through unchanged"
            );
        }
    }

    // And the rollup the list folds this into agrees with the single-order path.
    let first_single = list_po_lines(&mut conn, org_id, first).await.expect("first");
    let first_batched: Vec<_> = batched
        .iter()
        .filter(|line| line.purchase_order_id == first)
        .cloned()
        .collect();
    assert_eq!(
        core_domain::entities::po_rollup(&first_single).expect("single rollup"),
        core_domain::entities::po_rollup(&first_batched).expect("batched rollup"),
        "a row's value from the batch equals the order's own total"
    );
}

#[tokio::test]
async fn batch_read_never_reaches_another_org() {
    let pool = pool().await;
    let mine_org = seed_org(&pool).await;
    let other_org = seed_org(&pool).await;

    let mine_vendor = seed_vendor(&pool, mine_org).await;
    let other_vendor = seed_vendor(&pool, other_org).await;
    let mine_order = seed_order(&pool, mine_org, mine_vendor).await;
    let other_order = seed_order(&pool, other_org, other_vendor).await;
    seed_line(&pool, mine_org, mine_order, 1, 1_000, 100).await;
    seed_line(&pool, other_org, other_order, 1, 9_000, 900).await;

    let mut conn = pool.acquire().await.expect("conn");
    // Ask for both ids while scoped to my org: the other org's line must not
    // appear, because the read is org-scoped, not id-scoped.
    let batched = list_po_lines_for_orders(&mut conn, mine_org, &[mine_order, other_order])
        .await
        .expect("batched read");

    assert_eq!(batched.len(), 1, "only my org's line: {batched:#?}");
    assert_eq!(batched[0].purchase_order_id, mine_order);
    assert_ne!(
        batched[0].org_id, other_org,
        "a foreign org's line never leaks into the batch"
    );
}

#[tokio::test]
async fn an_empty_batch_is_a_no_op() {
    let pool = pool().await;
    let org_id = seed_org(&pool).await;
    let mut conn = pool.acquire().await.expect("conn");
    let batched = list_po_lines_for_orders(&mut conn, org_id, &[])
        .await
        .expect("empty batch");
    assert!(
        batched.is_empty(),
        "an empty id list is not a query that returns everything"
    );
}
