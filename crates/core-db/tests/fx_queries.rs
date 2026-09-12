//! Currency & FX persistence tests (ADR 0002, Wave 12 lane 10; migrations
//! 0065/0066/0067).
//!
//! Covers the promises the API layer leans on: the append-only rate book
//! (insert-only, corrections add rows), the effective-rate resolver
//! (directed + inverse coverage, newest capture on-or-before the date),
//! the immutability triggers (rate rows and conversion snapshots never
//! rewrite), the governed base-currency change guard, and the
//! reconciliation read model over the five lock moments.

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::{costs, fx, orgs, tickets};
use sqlx::PgPool;
use uuid::Uuid;

macro_rules! rtx {
    ($app:expr, $org:expr, $body:expr) => {{
        let mut tx = org_tx($app, $org).await;
        let out = $body(&mut *tx).await;
        tx.commit().await.expect("commit read");
        out
    }};
}

async fn seed_org(app: &PgPool, n: u64) -> Uuid {
    let org = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    orgs::insert_org(&mut *tx, org, "Acme MSP", &format!("fx-{n}"), "MER")
        .await
        .expect("org insert");
    orgs::seed_default_board(&mut tx, org)
        .await
        .expect("board seed");
    orgs::seed_default_sla_policy(&mut *tx, org)
        .await
        .expect("sla seed");
    tx.commit().await.expect("commit seed");
    org
}

async fn seed_ticket(app: &PgPool, org: Uuid, n: u64) -> Uuid {
    let ticket_id = fixed_uuid(7_000 + n);
    let mut tx = org_tx(app, org).await;
    let board = sqlx::query_scalar!(r#"SELECT id FROM boards WHERE org_id = $1 LIMIT 1"#, org)
        .fetch_one(&mut *tx)
        .await
        .expect("board");
    let statuses = sqlx::query_scalar!(
        r#"SELECT id FROM board_statuses WHERE board_id = $1 ORDER BY sort_order LIMIT 1"#,
        board
    )
    .fetch_one(&mut *tx)
    .await
    .expect("statuses");
    let (prefix, seq) = orgs::next_ticket_number(&mut *tx, org)
        .await
        .expect("number alloc");
    let number = format!("{prefix}-{seq}");
    tickets::insert_ticket(
        &mut tx,
        tickets::NewTicket {
            id: ticket_id,
            org_id: org,
            number: &number,
            board_id: board,
            status_id: statuses,
            title: "FX ticket",
            body: None,
            priority: core_domain::status::Priority::P3,
            client_id: None,
            site_id: None,
            contact_id: None,
            assignee_id: None,
            requester_id: None,
            due_at: None,
        },
    )
    .await
    .expect("ticket insert");
    tx.commit().await.expect("commit");
    ticket_id
}

const DAY1: chrono::NaiveDate = match chrono::NaiveDate::from_ymd_opt(2026, 9, 1) {
    Some(d) => d,
    None => unreachable!(),
};
const DAY5: chrono::NaiveDate = match chrono::NaiveDate::from_ymd_opt(2026, 9, 5) {
    Some(d) => d,
    None => unreachable!(),
};
const DAY10: chrono::NaiveDate = match chrono::NaiveDate::from_ymd_opt(2026, 9, 10) {
    Some(d) => d,
    None => unreachable!(),
};

#[tokio::test]
async fn rate_book_is_append_only_and_resolver_is_deterministic() {
    let db = test_db().await;
    let org = seed_org(&db.admin, 401).await;

    // Directed row: 1 USD = 17.85 ZAR on DAY1 (manual).
    let first = fx::insert_rate(
        &db.admin,
        org,
        "USD",
        "ZAR",
        DAY1,
        "17.850000000000",
        "manual",
        None,
    )
    .await
    .expect("rate insert");

    // Directed resolution on/before the date: the stored row wins.
    let coverage = rtx!(&db.admin, org, |c| fx::resolve_rate(
        c, org, "USD", "ZAR", DAY5
    ))
    .expect("resolve")
    .expect("directed coverage");
    assert_eq!(coverage.id, first.id);

    // Corrections ADD rows; the resolver picks the newest capture whose
    // as_of_date is on-or-before the requested date.
    let correction = fx::insert_rate(
        &db.admin,
        org,
        "USD",
        "ZAR",
        DAY5,
        "17.900000000000",
        "manual",
        None,
    )
    .await
    .expect("correction row");
    assert_ne!(correction.id, first.id);
    let effective = rtx!(&db.admin, org, |c| fx::resolve_rate(
        c, org, "USD", "ZAR", DAY10
    ))
    .expect("resolve")
    .expect("coverage");
    assert_eq!(effective.id, correction.id, "newest capture ≤ date wins");
    assert_eq!(effective.rate, "17.900000000000");

    // Inverse coverage: resolving ZAR → USD finds the stored (USD, ZAR)
    // row; the API layer inverts it through the typed operation.
    let inverse = rtx!(&db.admin, org, |c| fx::resolve_rate(
        c, org, "ZAR", "USD", DAY5
    ))
    .expect("resolve")
    .expect("inverse coverage");
    assert_eq!(
        (
            inverse.base_currency.as_str(),
            inverse.quote_currency.as_str()
        ),
        ("USD", "ZAR")
    );

    // No coverage for an unrecorded pair ⇒ None (fail-closed at the API).
    let missing = rtx!(&db.admin, org, |c| fx::resolve_rate(
        c, org, "JPY", "USD", DAY5
    ))
    .expect("resolve");
    assert!(missing.is_none());

    // Append-only enforcement: UPDATE and DELETE are refused by triggers.
    let mut tx = org_tx(&db.admin, org).await;
    let rewrite = sqlx::query("UPDATE fx_rates SET rate = 1 WHERE id = $1")
        .bind(first.id)
        .execute(&mut *tx)
        .await;
    assert!(rewrite.is_err(), "fx_rates UPDATE must be refused");
    let erase = sqlx::query("DELETE FROM fx_rates WHERE id = $1")
        .bind(first.id)
        .execute(&mut *tx)
        .await;
    assert!(erase.is_err(), "fx_rates DELETE must be refused");
    tx.rollback().await.expect("rollback");
}

#[tokio::test]
async fn conversion_snapshots_survive_patches_and_refuse_rewrites() {
    let db = test_db().await;
    let org = seed_org(&db.admin, 402).await;
    let ticket = seed_ticket(&db.admin, org, 402).await;

    // An expense entered in ZAR (org base USD) carrying an explicit
    // snapshot: 178.50 ZAR at 0.056022408964 = 10.00 USD.
    let expense = core_domain::entities::TicketExpense {
        id: Uuid::now_v7(),
        org_id: org,
        ticket_id: ticket,
        description: "ZAR courier".into(),
        amount_minor: 178_500,
        currency: "ZAR".into(),
        billable: false,
        fx_rate_used: Some("0.056022408964".into()),
        fx_rate_date: Some(DAY1),
        fx_source: Some("manual".into()),
        base_amount_minor: Some(10_000),
        invoice_line_id: None,
        incurred_by: None,
        incurred_at: chrono::Utc::now(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    costs::insert_expense(&db.admin, &expense)
        .await
        .expect("expense insert");

    // A normal field patch leaves the snapshot untouched.
    let mut tx = org_tx(&db.admin, org).await;
    costs::update_expense(&mut tx, org, expense.id, Some("renamed"), None, None)
        .await
        .expect("patch");
    let rows = costs::list_expenses(&mut *tx, org, ticket)
        .await
        .expect("list");
    tx.commit().await.expect("commit");
    let found = rows
        .into_iter()
        .find(|e| e.id == expense.id)
        .expect("expense row");
    assert_eq!(found.fx_rate_used.as_deref(), Some("0.056022408964"));
    assert_eq!(found.fx_source.as_deref(), Some("manual"));
    assert_eq!(found.base_amount_minor, Some(10_000));

    // A direct rewrite of any snapshot column is refused by the trigger.
    let mut tx = org_tx(&db.admin, org).await;
    let rewrite = sqlx::query(
        "UPDATE ticket_expenses SET base_amount_minor = 1 WHERE org_id = $1 AND id = $2",
    )
    .bind(org)
    .bind(expense.id)
    .execute(&mut *tx)
    .await;
    assert!(rewrite.is_err(), "snapshot rewrite must be refused");
    tx.rollback().await.expect("rollback");
}

#[tokio::test]
async fn base_currency_change_is_governed_by_snapshot_presence() {
    let db = test_db().await;
    let org = seed_org(&db.admin, 403).await;

    // No snapshots yet: the change lands.
    let changed = {
        let mut tx = org_tx(&db.admin, org).await;
        let o = orgs::change_base_currency(&mut tx, org, "EUR")
            .await
            .expect("change");
        tx.commit().await.expect("commit");
        o
    };
    assert_eq!(changed.base_currency, "EUR");

    // Once a snapshot exists, the change is a typed conflict: snapshots
    // are locked to the base they were taken against.
    let ticket = seed_ticket(&db.admin, org, 403).await;
    let mut tx = org_tx(&db.admin, org).await;
    sqlx::query(
        "INSERT INTO ticket_expenses (id, org_id, ticket_id, description, amount_minor, currency, fx_rate_used, fx_rate_date, fx_source, base_amount_minor)
         VALUES ($1, $2, $3, 'gap', 100, 'USD', 1.000000000000, $4, 'manual', 90)",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(ticket)
    .bind(DAY1)
    .execute(&mut *tx)
    .await
    .expect("seed snapshot row");
    tx.commit().await.expect("commit");
    let refused = {
        let mut tx = org_tx(&db.admin, org).await;
        orgs::change_base_currency(&mut tx, org, "GBP").await
    };
    assert!(matches!(refused, Err(core_db::DbError::Conflict(_))));
}

#[tokio::test]
async fn reconciliation_read_model_lists_cross_currency_facts() {
    let db = test_db().await;
    let org = seed_org(&db.admin, 404).await;
    let ticket = seed_ticket(&db.admin, org, 404).await;

    let mut tx = org_tx(&db.admin, org).await;
    sqlx::query(
        "INSERT INTO ticket_expenses (id, org_id, ticket_id, description, amount_minor, currency, fx_rate_used, fx_rate_date, fx_source, base_amount_minor, incurred_at)
         VALUES ($1, $2, $3, 'ZAR courier', 178500, 'ZAR', 0.056022408964, $4, 'manual', 10000, now())",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(ticket)
    .bind(DAY1)
    .execute(&mut *tx)
    .await
    .expect("expense");
    tx.commit().await.expect("commit");

    let rows = fx::reconciliation(&db.admin, org, 100)
        .await
        .expect("reconciliation");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, "expense");
    assert_eq!(rows[0].original_currency.as_deref(), Some("ZAR"));
    assert_eq!(rows[0].original_amount_minor, Some(178_500));
    assert_eq!(rows[0].base_amount_minor, Some(10_000));
    assert_eq!(rows[0].sync_target, "none");
}
