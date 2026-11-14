//! Client-margin read model (migration 0188) and the `client_margin` report.
//!
//! The acceptance sentence this suite exists for: *a client with no invoiced
//! revenue in the period shows a negative margin, not a blank*. Every test
//! therefore drives the REAL compiled statement from `core_reports` against a
//! migrated database — not a hand-written approximation of it — so the view,
//! the compiler and the arithmetic are proved together.
//!
//! Seeding uses the admin pool (table owner, RLS bypassed); every assertion
//! reads through `org_tx`, which binds `app.org_id` exactly like a request.

mod common;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use common::{fixed_uuid, org_tx, test_db};
use core_reports::{
    compile, resolve_window, Bind, KindParams, MarginParams, ReportKind, Scope, WindowParams,
};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

/// One report row as the API decodes it (same column names, same types).
#[derive(Debug)]
struct MarginRow {
    client_id: Uuid,
    revenue_minor: i64,
    cost_minor: i64,
    costed_minutes: i64,
    uncosted_minutes: i64,
    unconverted_credit_minor: i64,
    margin_minor: Option<i64>,
}

/// Ids are global primary keys, and tests share one database, so every row id
/// is composed from its org tag, a per-table kind and a serial: distinct
/// (org, table) pairs never collide, and a test can recompute an id it needs
/// without reading it back.
fn uid(org: Uuid, kind: u128, n: u64) -> Uuid {
    let org_tag = org.as_u128() & 0xFFFF_FFFF_FFFF_FFFF;
    Uuid::from_u128((org_tag << 64) | ((kind & 0xFF) << 32) | u128::from(n & 0xFFFF_FFFF))
}

const USER: u128 = 1;
const CLIENT: u128 = 2;
const BOARD: u128 = 3;
const STATUS: u128 = 4;
const TICKET: u128 = 5;
const CONTRACT: u128 = 6;
const ENTRY: u128 = 7;
const INVOICE: u128 = 8;
const CREDIT: u128 = 9;

/// Fixed anchors: every fixture lives in June 2026.
fn day(n: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 6, n).expect("valid June day")
}

/// The day after the June window closes — the exclusive edge.
fn july_first() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 7, 1).expect("valid July day")
}

fn at(day: NaiveDate) -> DateTime<Utc> {
    day.and_hms_opt(9, 0, 0).expect("valid time").and_utc()
}

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_781_000_000, 0).expect("valid instant")
}

// --- seeding -----------------------------------------------------------------

async fn seed_org(admin: &PgPool, n: u64, default_cost_rate_minor: Option<i64>) -> Uuid {
    let org = fixed_uuid(n);
    sqlx::query(
        "INSERT INTO orgs (id, name, slug, default_cost_rate_minor) VALUES ($1, $2, $3, $4)",
    )
    .bind(org)
    .bind(format!("margin org {n}"))
    .bind(format!("margin-org-{n}"))
    .bind(default_cost_rate_minor)
    .execute(admin)
    .await
    .expect("seed org");
    org
}

async fn seed_user(admin: &PgPool, org: Uuid, n: u64, rate: Option<i64>) -> Uuid {
    let user = uid(org, USER, n);
    sqlx::query(
        "INSERT INTO users (id, org_id, email, display_name, password_hash, cost_rate_minor)
         VALUES ($1, $2, $3, $4, 'x', $5)",
    )
    .bind(user)
    .bind(org)
    .bind(format!("margin{n}@example.test"))
    .bind(format!("Margin User {n}"))
    .bind(rate)
    .execute(admin)
    .await
    .expect("seed user");
    user
}

async fn seed_client(admin: &PgPool, org: Uuid, n: u64) -> Uuid {
    let client = uid(org, CLIENT, n);
    sqlx::query("INSERT INTO clients (id, org_id, name) VALUES ($1, $2, $3)")
        .bind(client)
        .bind(org)
        .bind(format!("Client {n}"))
        .execute(admin)
        .await
        .expect("seed client");
    client
}

/// Board + one open status + one ticket for `client`, plus a contract the
/// invoices can reference.
async fn seed_ticket_and_contract(admin: &PgPool, org: Uuid, client: Uuid, n: u64) -> (Uuid, Uuid) {
    let board = uid(org, BOARD, n);
    let status = uid(org, STATUS, n);
    let ticket = uid(org, TICKET, n);
    let contract = uid(org, CONTRACT, n);
    sqlx::query("INSERT INTO boards (id, org_id, name, is_default) VALUES ($1, $2, $3, true)")
        .bind(board)
        .bind(org)
        .bind(format!("Board {n}"))
        .execute(admin)
        .await
        .expect("seed board");
    sqlx::query(
        "INSERT INTO board_statuses (id, board_id, org_id, name, category, sort_order, is_entry)
         VALUES ($1, $2, $3, 'Open', 'open', 0, true)",
    )
    .bind(status)
    .bind(board)
    .bind(org)
    .execute(admin)
    .await
    .expect("seed status");
    sqlx::query(
        "INSERT INTO tickets (id, org_id, number, board_id, status_id, title, category, client_id)
         VALUES ($1, $2, $3, $4, $5, 'Margin ticket', 'open', $6)",
    )
    .bind(ticket)
    .bind(org)
    .bind(format!("M-{n}"))
    .bind(board)
    .bind(status)
    .bind(client)
    .execute(admin)
    .await
    .expect("seed ticket");
    sqlx::query(
        "INSERT INTO contracts (id, org_id, client_id, number, name, contract_type, currency, effective_from)
         VALUES ($1, $2, $3, $4, 'Margin contract', 'time_and_materials', 'USD', $5)",
    )
    .bind(contract)
    .bind(org)
    .bind(client)
    .bind(format!("C-{n}"))
    .bind(day(1))
    .execute(admin)
    .await
    .expect("seed contract");
    (ticket, contract)
}

#[allow(clippy::too_many_arguments)]
async fn add_time(
    admin: &PgPool,
    org: Uuid,
    ticket: Uuid,
    user: Uuid,
    n: u64,
    minutes: i32,
    on: NaiveDate,
) {
    let start = at(on);
    sqlx::query(
        "INSERT INTO time_entries (id, org_id, ticket_id, user_id, started_at, ended_at, minutes, billable)
         VALUES ($1, $2, $3, $4, $5, $6, $7, true)",
    )
    .bind(uid(org, ENTRY, n))
    .bind(org)
    .bind(ticket)
    .bind(user)
    .bind(start)
    .bind(start + Duration::minutes(i64::from(minutes)))
    .bind(minutes)
    .execute(admin)
    .await
    .expect("seed time entry");
}

#[allow(clippy::too_many_arguments)]
async fn add_invoice(
    admin: &PgPool,
    org: Uuid,
    client: Uuid,
    contract: Uuid,
    n: u64,
    status: &str,
    issued: NaiveDate,
    total_minor: i64,
    currency: &str,
    base_total_minor: Option<i64>,
) -> Uuid {
    let invoice = uid(org, INVOICE, n);
    // A base-currency snapshot is all-or-nothing (0065's atomic group).
    let (fx_rate_used, fx_rate_date, fx_source) = match base_total_minor {
        Some(_) => (Some(1.10_f64), Some(issued), Some("test")),
        None => (None, None, None),
    };
    sqlx::query(
        "INSERT INTO invoices (id, org_id, client_id, contract_id, number, status, currency,
                               issued_at, due_on, total_minor, base_total_minor,
                               fx_rate_used, fx_rate_date, fx_source)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
    )
    .bind(invoice)
    .bind(org)
    .bind(client)
    .bind(contract)
    .bind(format!("INV-{n}"))
    .bind(status)
    .bind(currency)
    .bind(issued)
    .bind(issued + Duration::days(30))
    .bind(total_minor)
    .bind(base_total_minor)
    .bind(fx_rate_used)
    .bind(fx_rate_date)
    .bind(fx_source)
    .execute(admin)
    .await
    .expect("seed invoice");
    invoice
}

#[allow(clippy::too_many_arguments)]
async fn add_credit(
    admin: &PgPool,
    org: Uuid,
    client: Uuid,
    against: Option<Uuid>,
    n: u64,
    total_minor: i64,
    currency: &str,
    created: NaiveDate,
) {
    sqlx::query(
        "INSERT INTO credit_notes (id, org_id, number, client_id, issued_against_invoice_id,
                                   currency, total_minor, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(uid(org, CREDIT, n))
    .bind(org)
    .bind(format!("CN-{n}"))
    .bind(client)
    .bind(against)
    .bind(currency)
    .bind(total_minor)
    .bind(at(created))
    .execute(admin)
    .await
    .expect("seed credit note");
}

// --- report execution ---------------------------------------------------------

fn bind_all<'q>(
    mut query: sqlx::query::Query<'q, Postgres, sqlx::postgres::PgArguments>,
    binds: &'q [Bind],
) -> sqlx::query::Query<'q, Postgres, sqlx::postgres::PgArguments> {
    for b in binds {
        query = match b {
            Bind::Uuid(u) => query.bind(*u),
            Bind::Text(s) => query.bind(s.as_str()),
            Bind::Date(d) => query.bind(*d),
            Bind::Timestamp(t) => query.bind(*t),
            Bind::Int(n) => query.bind(*n),
            Bind::Bool(b) => query.bind(*b),
        };
    }
    query
}

fn margin_params(from: NaiveDate, to: NaiveDate) -> KindParams {
    KindParams::ClientMargin(MarginParams {
        window: WindowParams {
            from: Some(from),
            to: Some(to),
        },
    })
}

/// Compiles the real `client_margin` report for `[from, to]` and runs it.
async fn run_margin(
    tx: &mut Transaction<'_, Postgres>,
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<MarginRow> {
    let win = resolve_window(Some(from), Some(to), now()).expect("window");
    let compiled = compile(
        ReportKind::ClientMargin,
        &margin_params(from, to),
        Scope::OrgWide,
        None,
        500,
        win,
    )
    .expect("margin compiles");

    let rows = bind_all(sqlx::query(&compiled.sql), &compiled.binds)
        .fetch_all(&mut **tx)
        .await
        .expect("run margin report");
    rows.into_iter()
        .map(|r| MarginRow {
            client_id: r.try_get("client_id").expect("client_id"),
            revenue_minor: r.try_get("revenue_minor").expect("revenue_minor"),
            cost_minor: r.try_get("cost_minor").expect("cost_minor"),
            costed_minutes: r.try_get("costed_minutes").expect("costed_minutes"),
            uncosted_minutes: r.try_get("uncosted_minutes").expect("uncosted_minutes"),
            unconverted_credit_minor: r
                .try_get("unconverted_credit_minor")
                .expect("unconverted_credit_minor"),
            margin_minor: r.try_get("margin_minor").expect("margin_minor"),
        })
        .collect()
}

async fn run_summary(
    tx: &mut Transaction<'_, Postgres>,
    from: NaiveDate,
    to: NaiveDate,
) -> sqlx::postgres::PgRow {
    let win = resolve_window(Some(from), Some(to), now()).expect("window");
    let compiled = compile(
        ReportKind::ClientMargin,
        &margin_params(from, to),
        Scope::OrgWide,
        None,
        500,
        win,
    )
    .expect("margin compiles");
    bind_all(sqlx::query(&compiled.summary_sql), &compiled.summary_binds)
        .fetch_one(&mut **tx)
        .await
        .expect("run margin summary")
}

fn row_for(rows: &[MarginRow], client: Uuid) -> &MarginRow {
    rows.iter()
        .find(|r| r.client_id == client)
        .unwrap_or_else(|| panic!("client {client} missing from the report"))
}

// --- the arithmetic -----------------------------------------------------------

/// Revenue and cost on one row, a cost-only client going negative, a
/// revenue-only client keeping the full amount, and a dormant client at zero.
#[tokio::test]
async fn margin_arithmetic_including_the_cost_only_client() {
    let db = test_db().await;
    let org = seed_org(&db.admin, 1, None).await;
    // 60.00/hour.
    let user = seed_user(&db.admin, org, 1, Some(6_000)).await;

    let a = seed_client(&db.admin, org, 1).await;
    let b = seed_client(&db.admin, org, 2).await;
    let c = seed_client(&db.admin, org, 3).await;
    let d = seed_client(&db.admin, org, 4).await;

    // A: 200.00 invoiced, 2h of work = 120.00 cost -> 80.00 margin.
    let (ticket_a, contract_a) = seed_ticket_and_contract(&db.admin, org, a, 1).await;
    add_invoice(
        &db.admin,
        org,
        a,
        contract_a,
        1,
        "issued",
        day(10),
        20_000,
        "USD",
        None,
    )
    .await;
    add_time(&db.admin, org, ticket_a, user, 1, 120, day(11)).await;

    // B: cost and NO revenue — the acceptance case.
    let (ticket_b, _) = seed_ticket_and_contract(&db.admin, org, b, 2).await;
    add_time(&db.admin, org, ticket_b, user, 2, 60, day(12)).await;

    // C: revenue and no time — the whole amount is margin.
    let (_, contract_c) = seed_ticket_and_contract(&db.admin, org, c, 3).await;
    add_invoice(
        &db.admin,
        org,
        c,
        contract_c,
        2,
        "paid",
        day(13),
        50_000,
        "USD",
        None,
    )
    .await;

    // D: dormant — no events at all.

    let mut tx = org_tx(&db.app, org).await;
    let rows = run_margin(&mut tx, day(1), day(30)).await;

    let ra = row_for(&rows, a);
    assert_eq!((ra.revenue_minor, ra.cost_minor), (20_000, 12_000));
    assert_eq!(ra.costed_minutes, 120);
    assert_eq!(ra.margin_minor, Some(8_000));

    let rb = row_for(&rows, b);
    assert_eq!(rb.revenue_minor, 0, "a cost-only client invoices nothing");
    assert_eq!(rb.cost_minor, 6_000);
    assert_eq!(
        rb.margin_minor,
        Some(-6_000),
        "cost with no revenue must read as a negative margin, never a blank"
    );

    let rc = row_for(&rows, c);
    assert_eq!((rc.revenue_minor, rc.cost_minor), (50_000, 0));
    assert_eq!(rc.margin_minor, Some(50_000));

    let rd = row_for(&rows, d);
    assert_eq!((rd.revenue_minor, rd.cost_minor), (0, 0));
    assert_eq!(
        rd.margin_minor,
        Some(0),
        "a client with no activity is zero"
    );

    // The summary is the sum of the rows, with no second rounding step.
    let summary = run_summary(&mut tx, day(1), day(30)).await;
    let total_revenue: i64 = summary.try_get("total_revenue_minor").expect("revenue");
    let total_cost: i64 = summary.try_get("total_cost_minor").expect("cost");
    let total_margin: Option<i64> = summary.try_get("total_margin_minor").expect("margin");
    assert_eq!(total_revenue, 70_000);
    assert_eq!(total_cost, 18_000);
    assert_eq!(total_margin, Some(52_000));
    let with_activity: i64 = summary
        .try_get("clients_with_activity")
        .expect("activity count");
    assert_eq!(with_activity, 3);
}

/// Rounding happens once per entry, half up, matching core-billing's
/// `prorate`: (minutes * rate + 30) / 60.
#[tokio::test]
async fn per_entry_rounding_is_half_up_to_the_cent() {
    let db = test_db().await;
    let org = seed_org(&db.admin, 2, None).await;
    // 33.30/hour: one minute is 55.5 cents -> 56, not 55.
    let user = seed_user(&db.admin, org, 2, Some(3_330)).await;
    let client = seed_client(&db.admin, org, 10).await;
    let (ticket, _) = seed_ticket_and_contract(&db.admin, org, client, 10).await;
    add_time(&db.admin, org, ticket, user, 10, 1, day(5)).await;

    let mut tx = org_tx(&db.app, org).await;
    let rows = run_margin(&mut tx, day(1), day(30)).await;
    let r = row_for(&rows, client);
    assert_eq!(r.cost_minor, 56, "half a cent rounds up, once");
    assert_eq!(r.margin_minor, Some(-56));
}

// --- credit notes -------------------------------------------------------------

/// A credit note is a negative revenue event; ignoring it would overstate
/// margin in the customer's favour. A non-base credit with no invoice to
/// borrow a rate from is reported as unconverted, never dropped.
#[tokio::test]
async fn credit_notes_reduce_revenue_and_unconvertible_ones_are_reported() {
    let db = test_db().await;
    let org = seed_org(&db.admin, 3, None).await;
    let client = seed_client(&db.admin, org, 20).await;
    let (_, contract) = seed_ticket_and_contract(&db.admin, org, client, 20).await;

    let invoice = add_invoice(
        &db.admin,
        org,
        client,
        contract,
        20,
        "issued",
        day(5),
        10_000,
        "USD",
        None,
    )
    .await;
    // 30.00 credited back against that invoice, in base currency.
    add_credit(
        &db.admin,
        org,
        client,
        Some(invoice),
        20,
        3_000,
        "USD",
        day(6),
    )
    .await;
    // 20.00 credited in a foreign currency with no issuing invoice: no rate
    // can be borrowed, so it is surfaced, not silently converted or dropped.
    add_credit(&db.admin, org, client, None, 21, 2_000, "EUR", day(7)).await;

    let mut tx = org_tx(&db.app, org).await;
    let rows = run_margin(&mut tx, day(1), day(30)).await;
    let r = row_for(&rows, client);
    assert_eq!(r.revenue_minor, 7_000, "10,000 invoiced - 3,000 credited");
    assert_eq!(r.unconverted_credit_minor, 2_000);
    assert_eq!(r.margin_minor, Some(7_000));
}

/// Cross-currency revenue is read in base currency: a document issued in a
/// non-base currency carries the issue-time snapshot (0065), and the view
/// never re-converts.
#[tokio::test]
async fn cross_currency_revenue_uses_the_locked_base_snapshot() {
    let db = test_db().await;
    let org = seed_org(&db.admin, 4, None).await;
    let client = seed_client(&db.admin, org, 30).await;
    let (_, contract) = seed_ticket_and_contract(&db.admin, org, client, 30).await;
    // 100.00 EUR at 1.10 -> 110.00 base.
    add_invoice(
        &db.admin,
        org,
        client,
        contract,
        30,
        "issued",
        day(8),
        10_000,
        "EUR",
        Some(11_000),
    )
    .await;

    let mut tx = org_tx(&db.app, org).await;
    let rows = run_margin(&mut tx, day(1), day(30)).await;
    assert_eq!(row_for(&rows, client).revenue_minor, 11_000);
}

// --- cost fallbacks -----------------------------------------------------------

/// A user with no rate uses the org default. With no default either, the
/// minutes are uncosted and the margin is NULL — never a zero-cost margin.
#[tokio::test]
async fn missing_rates_fall_back_then_declare_the_cost_unknown() {
    let db = test_db().await;
    let org = seed_org(&db.admin, 5, None).await;
    let no_rate_user = seed_user(&db.admin, org, 5, None).await;
    let zero_rate_user = seed_user(&db.admin, org, 6, Some(0)).await;
    let client = seed_client(&db.admin, org, 40).await;
    let (ticket, _) = seed_ticket_and_contract(&db.admin, org, client, 40).await;
    add_time(&db.admin, org, ticket, no_rate_user, 40, 60, day(9)).await;

    // No user rate, no org default: cost unknown.
    {
        let mut tx = org_tx(&db.app, org).await;
        let rows = run_margin(&mut tx, day(1), day(30)).await;
        let r = row_for(&rows, client);
        assert_eq!(r.cost_minor, 0, "no cost was inferred at all");
        assert_eq!(r.costed_minutes, 0);
        assert_eq!(r.uncosted_minutes, 60);
        assert_eq!(
            r.margin_minor, None,
            "an uncosted client has no margin, not a revenue-shaped one"
        );
    }

    // Org default 30.00/hour: the same minutes become costed.
    sqlx::query("UPDATE orgs SET default_cost_rate_minor = 3000 WHERE id = $1")
        .bind(org)
        .execute(&db.admin)
        .await
        .expect("set org default");
    {
        let mut tx = org_tx(&db.app, org).await;
        let rows = run_margin(&mut tx, day(1), day(30)).await;
        let r = row_for(&rows, client);
        assert_eq!(r.cost_minor, 3_000);
        assert_eq!(r.uncosted_minutes, 0);
        assert_eq!(r.margin_minor, Some(-3_000));
    }

    // A rate of zero is an answer, not a missing value: costed at 0, not
    // uncosted.
    add_time(&db.admin, org, ticket, zero_rate_user, 41, 30, day(9)).await;
    {
        let mut tx = org_tx(&db.app, org).await;
        let rows = run_margin(&mut tx, day(1), day(30)).await;
        let r = row_for(&rows, client);
        assert_eq!(r.costed_minutes, 90, "both entries are costed");
        assert_eq!(r.uncosted_minutes, 0);
        assert_eq!(r.cost_minor, 3_000, "the zero-rate minutes add no cost");
    }
}

/// A user rate wins over the org default.
#[tokio::test]
async fn the_personal_rate_beats_the_org_default() {
    let db = test_db().await;
    let org = seed_org(&db.admin, 6, Some(1_000)).await;
    let user = seed_user(&db.admin, org, 7, Some(9_000)).await;
    let client = seed_client(&db.admin, org, 50).await;
    let (ticket, _) = seed_ticket_and_contract(&db.admin, org, client, 50).await;
    add_time(&db.admin, org, ticket, user, 50, 60, day(10)).await;

    let mut tx = org_tx(&db.app, org).await;
    let rows = run_margin(&mut tx, day(1), day(30)).await;
    assert_eq!(row_for(&rows, client).cost_minor, 9_000);
}

// --- period + invoice state ---------------------------------------------------

/// The window is inclusive at the start and exclusive at the end, and only
/// issued/paid documents are revenue.
#[tokio::test]
async fn window_boundaries_and_invoice_state() {
    let db = test_db().await;
    let org = seed_org(&db.admin, 7, None).await;
    let user = seed_user(&db.admin, org, 8, Some(6_000)).await;
    let client = seed_client(&db.admin, org, 60).await;
    let (ticket, contract) = seed_ticket_and_contract(&db.admin, org, client, 60).await;

    // Revenue inside the window at both edges.
    add_invoice(
        &db.admin,
        org,
        client,
        contract,
        60,
        "issued",
        day(1),
        5_000,
        "USD",
        None,
    )
    .await;
    add_invoice(
        &db.admin,
        org,
        client,
        contract,
        61,
        "paid",
        day(30),
        7_000,
        "USD",
        None,
    )
    .await;
    // Outside the window (the day after the inclusive end).
    add_invoice(
        &db.admin,
        org,
        client,
        contract,
        62,
        "issued",
        july_first(),
        99_000,
        "USD",
        None,
    )
    .await;
    // Never revenue: draft is not issued, void was withdrawn.
    add_invoice(
        &db.admin,
        org,
        client,
        contract,
        63,
        "draft",
        day(15),
        88_000,
        "USD",
        None,
    )
    .await;
    add_invoice(
        &db.admin,
        org,
        client,
        contract,
        64,
        "void",
        day(15),
        77_000,
        "USD",
        None,
    )
    .await;

    add_time(&db.admin, org, ticket, user, 60, 60, day(1)).await;
    add_time(&db.admin, org, ticket, user, 61, 60, day(30)).await;
    add_time(&db.admin, org, ticket, user, 62, 60, july_first()).await;

    let mut tx = org_tx(&db.app, org).await;
    // June as the period: day 1 and day 30 in, day 31 out.
    let rows = run_margin(&mut tx, day(1), day(30)).await;
    let r = row_for(&rows, client);
    assert_eq!(
        r.revenue_minor, 12_000,
        "5,000 + 7,000; drafts/voids excluded"
    );
    assert_eq!(r.cost_minor, 12_000, "two 60-minute entries at 60.00/hour");
    assert_eq!(r.margin_minor, Some(0));

    // A single-day window on day 30 is inclusive of that day.
    let one_day = run_margin(&mut tx, day(30), day(30)).await;
    let r30 = row_for(&one_day, client);
    assert_eq!(r30.revenue_minor, 7_000);
    assert_eq!(r30.cost_minor, 6_000);
}

// --- tenancy ------------------------------------------------------------------

/// The view is security_invoker: another org's revenue and cost never appear.
#[tokio::test]
async fn the_view_is_org_scoped() {
    let db = test_db().await;
    let org_a = seed_org(&db.admin, 8, None).await;
    let org_b = seed_org(&db.admin, 9, None).await;
    let client_b = seed_client(&db.admin, org_b, 70).await;
    let (_, contract_b) = seed_ticket_and_contract(&db.admin, org_b, client_b, 70).await;
    add_invoice(
        &db.admin,
        org_b,
        client_b,
        contract_b,
        70,
        "issued",
        day(5),
        42_000,
        "USD",
        None,
    )
    .await;

    let mut tx = org_tx(&db.app, org_a).await;
    let rows = run_margin(&mut tx, day(1), day(30)).await;
    assert!(
        rows.iter().all(|r| r.client_id != client_b),
        "org A saw org B's client through the margin view"
    );
    assert!(rows.is_empty(), "org A has no clients of its own yet");
}
