//! Payment application decided by the database, not by a prior read (Wave 54
//! lane S4a).
//!
//! `apply_payment_total_expecting` takes the invoice total the caller priced
//! the receipt on and puts it in the UPDATE's predicate, next to the payable
//! status and the outstanding balance. The property worth pinning is the one
//! the old statement could not hold: a payment validated against a total that
//! moved before the write lands nowhere, says why, and leaves the invoice
//! exactly as it was. Silent success here means money recorded against a total
//! that no longer describes the invoice.
//!
//! Non-purpose: the HTTP routes that call this (core-api tests own those), and
//! the FX snapshot and provider seam, which run before the statement under
//! test.

mod common;

use common::{org_tx, test_db};
use core_db::invoices;
use core_db::DbError;
use sqlx::PgPool;
use uuid::Uuid;

struct Seeded {
    org: Uuid,
    invoice: Uuid,
}

/// One org with one issued invoice for 100.00, nothing paid.
async fn seed_issued_invoice(db: &common::TestDb) -> Seeded {
    let org = Uuid::now_v7();
    let client = Uuid::now_v7();
    let contract = Uuid::now_v7();
    let invoice = Uuid::now_v7();
    sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, $2, $3)")
        .bind(org)
        .bind("Ledger Co")
        .bind(format!("ledger-{org}"))
        .execute(&db.admin)
        .await
        .expect("org");
    sqlx::query("INSERT INTO clients (id, org_id, name) VALUES ($1, $2, 'Ledger Client')")
        .bind(client)
        .bind(org)
        .execute(&db.admin)
        .await
        .expect("client");
    sqlx::query(
        "INSERT INTO contracts (id, org_id, client_id, number, name, contract_type, status, \
                                currency, hourly_rate_minor, effective_from) \
         VALUES ($1, $2, $3, $4, 'Ledger T&M', 'time_and_materials', 'active', 'USD', 15000, \
                 CURRENT_DATE)",
    )
    .bind(contract)
    .bind(org)
    .bind(client)
    .bind(format!("C-{}", Uuid::now_v7().simple()))
    .execute(&db.admin)
    .await
    .expect("contract");
    sqlx::query(
        "INSERT INTO invoices (id, org_id, client_id, contract_id, number, status, currency, \
                               issued_at, due_on, total_minor, total_paid_minor) \
         VALUES ($1, $2, $3, $4, $5, 'issued', 'USD', CURRENT_DATE, CURRENT_DATE + 30, \
                 10000, 0)",
    )
    .bind(invoice)
    .bind(org)
    .bind(client)
    .bind(contract)
    .bind(format!("INV-{}", Uuid::now_v7().simple()))
    .execute(&db.admin)
    .await
    .expect("invoice");
    Seeded { org, invoice }
}

async fn paid_and_status(pool: &PgPool, invoice: Uuid) -> (i64, String) {
    sqlx::query_as("SELECT total_paid_minor, status FROM invoices WHERE id = $1")
        .bind(invoice)
        .fetch_one(pool)
        .await
        .expect("invoice row")
}

#[tokio::test]
async fn a_payment_priced_on_a_moved_total_is_refused_and_writes_nothing() {
    let db = test_db().await;
    let seeded = seed_issued_invoice(&db).await;

    // The payer's transaction, exactly as the route runs it: the invoice is
    // read, the amount is validated against that read, then the receipt is
    // written.
    let mut payer = org_tx(&db.app, seeded.org).await;
    let priced_on = invoices::get_invoice(&mut *payer, seeded.org, seeded.invoice)
        .await
        .expect("invoice read")
        .total_minor;
    assert_eq!(priced_on, 10_000, "the payer prices against 100.00");
    sqlx::query(
        "INSERT INTO invoice_payments (id, org_id, invoice_id, amount_minor, currency, provider) \
         VALUES ($1, $2, $3, 2500, 'USD', 'none')",
    )
    .bind(Uuid::now_v7())
    .bind(seeded.org)
    .bind(seeded.invoice)
    .execute(&mut *payer)
    .await
    .expect("receipt row");

    // Meanwhile the invoice is amended on another connection (a change order
    // billed late, a credit, an operator edit): the total the payer priced on
    // no longer describes the row.
    sqlx::query("UPDATE invoices SET total_minor = 12_500, updated_at = now() WHERE id = $1")
        .bind(seeded.invoice)
        .execute(&db.admin)
        .await
        .expect("total edit");

    let refused = invoices::apply_payment_total_expecting(
        &mut *payer,
        seeded.org,
        seeded.invoice,
        2_500,
        priced_on,
    )
    .await;
    match refused {
        Err(DbError::Conflict(message)) => assert!(
            message.contains("total changed"),
            "the refusal names the moved total: {message}"
        ),
        other => panic!("a stale total must be a typed conflict, got {other:?}"),
    }

    // The loser's transaction rolls back whole: receipt row and paid total.
    payer.rollback().await.expect("rollback");
    let (paid, status) = paid_and_status(&db.admin, seeded.invoice).await;
    assert_eq!(paid, 0, "no payment was applied against the stale total");
    assert_eq!(status, "issued", "the invoice keeps its payable status");
    let receipts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM invoice_payments WHERE invoice_id = $1")
            .bind(seeded.invoice)
            .fetch_one(&db.admin)
            .await
            .expect("receipt count");
    assert_eq!(
        receipts, 0,
        "the rolled-back receipt row left nothing behind"
    );
}

/// The same shape for the balance: two receipts priced on one outstanding
/// amount cannot both land, so the paid total never passes the total.
#[tokio::test]
async fn a_second_receipt_beyond_the_balance_is_refused() {
    let db = test_db().await;
    let seeded = seed_issued_invoice(&db).await;

    let mut first = org_tx(&db.app, seeded.org).await;
    let total = invoices::get_invoice(&mut *first, seeded.org, seeded.invoice)
        .await
        .expect("invoice read")
        .total_minor;
    invoices::apply_payment_total_expecting(&mut *first, seeded.org, seeded.invoice, 6_000, total)
        .await
        .expect("the first receipt fits the balance");
    first.commit().await.expect("commit the first receipt");

    // A second payer still believes 100.00 is outstanding and tries 60.00.
    let mut second = org_tx(&db.app, seeded.org).await;
    let refused = invoices::apply_payment_total_expecting(
        &mut *second,
        seeded.org,
        seeded.invoice,
        6_000,
        total,
    )
    .await;
    match refused {
        Err(DbError::Conflict(message)) => assert!(
            message.contains("outstanding balance"),
            "the refusal names the balance: {message}"
        ),
        other => panic!("an over-balance receipt must be a typed conflict, got {other:?}"),
    }
    second.rollback().await.expect("rollback");

    let (paid, status) = paid_and_status(&db.admin, seeded.invoice).await;
    assert_eq!(paid, 6_000, "only the receipt that fit was applied");
    assert_eq!(status, "issued", "60.00 of 100.00 leaves the invoice open");
}
