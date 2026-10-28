//! Read-model smoke tests for the versioned `ops.v1_*` view surface
//! (the private build tracker U3; upgrade-compatibility-policy §2; ADR 0001).
//!
//! Purpose: prove the direct-SQL/reporting contract — every view in the
//! `ops` schema (a) exists, (b) is `security_invoker = true` so underlying
//! RLS applies to non-owner readers, (c) is queryable as the application
//! role, (d) scopes rows by org exactly like the base tables (own org ⇒
//! visible; foreign org ⇒ hidden; NO `app.org_id` scope ⇒ default-deny
//! zero rows), (e) retains an `org_id` column as the tenancy seam, and
//! (f) matches the documented column contracts name-and-type exactly —
//! the mechanical guarantee behind "column contracts are the deliverable".
//! Also proves the 0059 taxonomy extension of `ops.v1_tickets` resolves
//! the two-level classification tree (category = root, subcategory = child).
//!
//! Non-purpose: view content correctness beyond scoping/classification
//! (the joining logic is exercised by the reporting layer's own tests) and
//! RLS policy coverage of base tables (db_suite :: rls_blocks_cross_org…).

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::{clients, orgs, tickets, timers, users};
use core_domain::entities::TimeEntry;
use core_domain::status::Priority;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

/// Every view the `ops` schema must expose: the v1 read-model slices
/// (0026 + 0041 + 0059) and the operator views that share the schema.
/// Views in `ops` that are operator surfaces rather than tenant-scoped read
/// models: no `org_id` seam, and the default-deny and column contracts below do
/// not apply to them. `ops.query_tail` is created only where
/// pg_stat_statements is installed and preloaded, so the existence test below
/// keeps it in the expected set only when the database could create it.
// 0145 database observability: the signal rollup and the per-statement query
// tail. Both are operator views rather than v1 slices, and both are
// `security_invoker`, which the reloptions sweep below checks (it sweeps
// whatever exists, so it covers the tail where the tail is there).
const OPERATOR_VIEWS: &[&str] = &["db_signals", "query_tail"];

const EXPECTED_V1_VIEWS: &[&str] = &[
    // 0026 reports slice
    "v1_tickets",
    "v1_time_entries",
    "v1_sla_outcomes",
    "v1_gap_days",
    "v1_first_response",
    // 0041 undo slice
    "v1_record_undo_stack",
    "v1_field_undo_stack",
    // 0144 operator view, not a v1 slice: the dead-work union across jobs,
    // email, webhook deliveries and dead stream events.
    "dead_work",
    // 0059 operational tranche
    "v1_clients",
    "v1_sites",
    "v1_contacts",
    "v1_board_statuses",
    "v1_invoices",
    "v1_invoice_lines",
    // 0067 fx reconciliation slice
    "v1_fx_reconciliation",
    // 0081 AI availability matrix (ADR 0008, Wave 18 lane 1)
    "v1_ai_usage_user_progress",
    "v1_ai_usage_org_progress",
    // 0131 org resource usage for hosted-tier monitoring (Wave 47 lane A6).
    // Registered here so the exact-catalog count, the `security_invoker`
    // reloptions check, and the default-deny-without-`app.org_id` sweep below
    // all cover it — the one tenant-scoped `ops` view this surface adds must
    // carry the same mechanical tenancy proof as the other sixteen, since
    // `marula_app` reads it directly (0131's GRANT).
    "v1_org_usage",
];

/// (view, [(column, data_type)]) — the 0059 tranche column contracts,
/// asserted name-and-type from `information_schema.columns`.
const TRANCHE_CONTRACTS: &[(&str, &[(&str, &str)])] = &[
    (
        "v1_clients",
        &[
            ("id", "uuid"),
            ("org_id", "uuid"),
            ("name", "text"),
            ("notes", "text"),
            ("created_at", "timestamp with time zone"),
            ("updated_at", "timestamp with time zone"),
        ],
    ),
    (
        "v1_sites",
        &[
            ("id", "uuid"),
            ("org_id", "uuid"),
            ("client_id", "uuid"),
            ("client_name", "text"),
            ("name", "text"),
            ("address", "text"),
            ("city", "text"),
            ("region", "text"),
            ("postal_code", "text"),
            ("country", "text"),
            ("latitude", "double precision"),
            ("longitude", "double precision"),
            ("timezone", "text"),
            ("created_at", "timestamp with time zone"),
        ],
    ),
    (
        "v1_contacts",
        &[
            ("id", "uuid"),
            ("org_id", "uuid"),
            ("client_id", "uuid"),
            ("client_name", "text"),
            ("site_id", "uuid"),
            ("site_name", "text"),
            ("name", "text"),
            ("email", "text"),
            ("phone", "text"),
            ("title", "text"),
            ("is_primary", "boolean"),
            ("created_at", "timestamp with time zone"),
        ],
    ),
    (
        "v1_board_statuses",
        &[
            ("id", "uuid"),
            ("org_id", "uuid"),
            ("board_id", "uuid"),
            ("board_name", "text"),
            ("name", "text"),
            ("category", "text"),
            ("sort_order", "integer"),
            ("is_entry", "boolean"),
            ("color", "text"),
        ],
    ),
    (
        "v1_invoices",
        &[
            ("id", "uuid"),
            ("org_id", "uuid"),
            ("run_id", "uuid"),
            ("run_number", "text"),
            ("client_id", "uuid"),
            ("client_name", "text"),
            ("contract_id", "uuid"),
            ("number", "text"),
            ("status", "text"),
            ("currency", "text"),
            ("period_from", "date"),
            ("period_to", "date"),
            ("issued_at", "date"),
            ("due_on", "date"),
            ("total_minor", "bigint"),
            ("total_paid_minor", "bigint"),
            ("balance_minor", "bigint"),
            ("dunning_state", "text"),
            ("dunning_updated_at", "timestamp with time zone"),
            ("created_by", "uuid"),
            ("created_at", "timestamp with time zone"),
            ("updated_at", "timestamp with time zone"),
        ],
    ),
    (
        "v1_invoice_lines",
        &[
            ("id", "uuid"),
            ("org_id", "uuid"),
            ("invoice_id", "uuid"),
            ("invoice_number", "text"),
            ("client_id", "uuid"),
            ("contract_id", "uuid"),
            ("kind", "text"),
            ("description", "text"),
            ("period_start", "date"),
            ("period_end", "date"),
            ("quantity_minutes", "bigint"),
            ("unit_amount_minor", "bigint"),
            ("amount_minor", "bigint"),
            ("currency", "text"),
            ("position", "integer"),
        ],
    ),
    (
        // 0131 org usage: one row per org, counts only. `org_id` leads and
        // is the tenancy seam; the rest are the metrics the materialized
        // table mirrors. Column contract asserted here so the API's view
        // cannot drift from what `OrgUsageRow` reads.
        "v1_org_usage",
        &[
            ("org_id", "uuid"),
            ("active_users", "bigint"),
            ("tickets_open", "bigint"),
            ("tickets_in_progress", "bigint"),
            ("tickets_waiting", "bigint"),
            ("tickets_closed", "bigint"),
            ("managed_devices", "bigint"),
            ("object_store_bytes", "bigint"),
            ("automation_runs", "bigint"),
            ("automation_runs_24h", "bigint"),
            ("webhook_deliveries", "bigint"),
            ("webhook_deliveries_24h", "bigint"),
            ("audit_events_24h", "bigint"),
        ],
    ),
];

/// The 0026 column order of ops.v1_tickets — CREATE OR REPLACE must keep
/// this prefix byte-stable; everything else appends at the end.
const V1_TICKETS_PREFIX: &[&str] = &[
    "id",
    "org_id",
    "number",
    "board_id",
    "board_name",
    "status_id",
    "status_name",
    "category",
    "priority",
    "title",
    "client_id",
    "client_name",
    "site_id",
    "assignee_id",
    "assignee_name",
    "requester_id",
    "due_at",
    "resolved_at",
    "created_at",
    "updated_at",
    "first_response_due",
    "resolution_due",
    "breached",
];

/// The 0059 appended taxonomy columns.
const V1_TICKETS_APPENDED: &[&str] = &[
    "ticket_type_id",
    "ticket_type_name",
    "ticket_category_id",
    "ticket_category_name",
    "ticket_subcategory_id",
    "ticket_subcategory_name",
];

struct Seeded {
    org: Uuid,
    board: Uuid,
    entry_status: Uuid,
    client: Uuid,
    site: Uuid,
    contact: Uuid,
    ticket: Uuid,
    invoice: Uuid,
    invoice_line: Uuid,
    time_entry: Uuid,
}

async fn status_id(tx: &mut Transaction<'_, Postgres>, board: Uuid, name: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM board_statuses WHERE board_id = $1 AND name = $2")
        .bind(board)
        .bind(name)
        .fetch_one(&mut **tx)
        .await
        .expect("seeded board status")
}

/// Seeds one org with a full tranche-1 data set: client, site, contact,
/// classified ticket, time entry, and an invoice with one line — all in the
/// org's RLS scope. Deterministic UUIDs derived from `n`.
async fn seed_org_with_tranche_data(app: &PgPool, n: u64) -> Seeded {
    let org = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    orgs::insert_org(
        &mut *tx,
        org,
        "Readmodel MSP",
        &format!("readmodel-{n}"),
        "RM",
    )
    .await
    .expect("org insert");
    orgs::seed_default_roles(&mut tx, org).await.expect("roles");
    let board = orgs::seed_default_board(&mut tx, org).await.expect("board");
    orgs::seed_default_sla_policy(&mut *tx, org)
        .await
        .expect("sla seed");
    let entry_status = status_id(&mut tx, board, "New").await;

    // Staff user (assignee for the ticket + author of the time entry).
    let user = fixed_uuid(n + 1);
    users::create_user(
        &mut *tx,
        user,
        org,
        &format!("u{n}@readmodel.test"),
        "Readmodel User",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user insert");

    // Client → site → contact.
    let client = fixed_uuid(n + 2);
    clients::create_client(&mut *tx, org, client, "Acme Corp", Some("smoke client"))
        .await
        .expect("client insert");
    let site = fixed_uuid(n + 3);
    clients::create_site(
        &mut *tx,
        org,
        site,
        client,
        "HQ",
        Some("1 Main St"),
        Some("Springfield"),
        Some("IL"),
        Some("62701"),
        Some("US"),
        Some(39.7817),
        Some(-89.6501),
        Some("America/Chicago"),
    )
    .await
    .expect("site insert");
    let contact = fixed_uuid(n + 4);
    clients::create_contact(
        &mut *tx,
        org,
        contact,
        client,
        Some(site),
        "Dana Contact",
        Some("dana@acme.test"),
        Some("+1 555 0100"),
        Some("Office Manager"),
        true,
    )
    .await
    .expect("contact insert");

    // Ticket on the default board's entry status.
    let (prefix, seq) = orgs::next_ticket_number(&mut *tx, org)
        .await
        .expect("alloc");
    let ticket = fixed_uuid(n + 5);
    tickets::insert_ticket(
        &mut tx,
        tickets::NewTicket {
            id: ticket,
            org_id: org,
            number: &format!("{prefix}-{seq}"),
            board_id: board,
            status_id: entry_status,
            title: "Read-model smoke ticket",
            body: None,
            priority: Priority::P2,
            client_id: Some(client),
            site_id: Some(site),
            contact_id: Some(contact),
            assignee_id: Some(user),
            requester_id: None,
            due_at: None,
        },
    )
    .await
    .expect("ticket insert");

    // Time entry against the ticket.
    let started = chrono::Utc::now() - chrono::Duration::hours(2);
    timers::insert_time_entry(
        &mut *tx,
        &TimeEntry {
            approval_status: None,
            approval_reason_code: None,
            unbillable_reason: None,
            id: fixed_uuid(n + 6),
            org_id: org,
            ticket_id: Some(ticket),
            phase_id: None,
            user_id: user,
            started_at: started,
            ended_at: started + chrono::Duration::minutes(45),
            minutes: 45,
            billable: true,
            notes: Some("read-model smoke entry".into()),
            work_type: Some("incident".into()),
            timer_session_id: None,
            created_at: started,
        },
    )
    .await
    .expect("time entry insert");

    // Contract → invoice → line (raw SQL: the read-model smoke test is
    // deliberately decoupled from the invoicing module API).
    let contract = fixed_uuid(n + 7);
    sqlx::query(
        "INSERT INTO contracts (id, org_id, client_id, number, name, contract_type, \
         status, currency, effective_from) \
         VALUES ($1, $2, $3, $4, $5, 'recurring', 'active', 'USD', DATE '2026-01-01')",
    )
    .bind(contract)
    .bind(org)
    .bind(client)
    .bind(format!("CT-{n}"))
    .bind("Managed services")
    .execute(&mut *tx)
    .await
    .expect("contract insert");

    let invoice = fixed_uuid(n + 8);
    sqlx::query(
        "INSERT INTO invoices (id, org_id, client_id, contract_id, number, status, \
         currency, period_from, period_to, issued_at, due_on, total_minor, total_paid_minor) \
         VALUES ($1, $2, $3, $4, $5, 'issued', 'USD', DATE '2026-01-01', DATE '2026-01-31', \
                 DATE '2026-02-01', DATE '2026-03-01', 15000, 5000)",
    )
    .bind(invoice)
    .bind(org)
    .bind(client)
    .bind(contract)
    .bind(format!("INV-{n}"))
    .execute(&mut *tx)
    .await
    .expect("invoice insert");

    let invoice_line = fixed_uuid(n + 9);
    sqlx::query(
        "INSERT INTO invoice_lines (id, org_id, invoice_id, contract_id, kind, description, \
         period_start, period_end, quantity_minutes, unit_amount_minor, amount_minor, position) \
         VALUES ($1, $2, $3, $4, 'recurring_base', 'Managed service fee', \
                 DATE '2026-01-01', DATE '2026-01-31', 0, 15000, 15000, 0)",
    )
    .bind(invoice_line)
    .bind(org)
    .bind(invoice)
    .bind(contract)
    .execute(&mut *tx)
    .await
    .expect("invoice line insert");

    tx.commit().await.expect("commit seed");

    Seeded {
        org,
        board,
        entry_status,
        client,
        site,
        contact,
        ticket,
        invoice,
        invoice_line,
        time_entry: fixed_uuid(n + 6),
    }
}

async fn view_names(admin: &PgPool) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT c.relname FROM pg_class c \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'ops' AND c.relkind = 'v' ORDER BY c.relname",
    )
    .fetch_all(admin)
    .await
    .expect("ops view catalog")
}

async fn view_columns(tx: &mut Transaction<'_, Postgres>, view: &str) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT column_name, data_type FROM information_schema.columns \
         WHERE table_schema = 'ops' AND table_name = $1 ORDER BY ordinal_position",
    )
    .bind(view)
    .fetch_all(&mut **tx)
    .await
    .expect("view columns")
}

async fn count_view(tx: &mut Transaction<'_, Postgres>, view: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM ops.{view}"))
        .fetch_one(&mut **tx)
        .await
        .expect("view count")
}

async fn count_view_where_id(tx: &mut Transaction<'_, Postgres>, view: &str, id: Uuid) -> i64 {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM ops.{view} WHERE id = $1"))
        .bind(id)
        .fetch_one(&mut **tx)
        .await
        .expect("view id count")
}

// --- existence + security_invoker ---------------------------------------------

/// Whether migration 0145 could create `ops.query_tail` here. It needs
/// `pg_stat_statements` both installed and preloaded, and says so with a notice
/// when it cannot; the CI service container and `scripts/gate-db.sh` run a
/// stock image, so the view is legitimately absent from a clean database.
async fn pg_stat_statements_preloaded(pool: &PgPool) -> bool {
    let libs: String = sqlx::query_scalar("SELECT current_setting('shared_preload_libraries')")
        .fetch_one(pool)
        .await
        .expect("shared_preload_libraries");
    libs.split(',').any(|l| l.trim() == "pg_stat_statements")
}

#[tokio::test]
async fn every_expected_v1_view_exists_and_is_security_invoker() {
    let db = test_db().await;
    let names = view_names(&db.admin).await;
    let conditional = pg_stat_statements_preloaded(&db.admin).await;
    let expected: Vec<&str> = EXPECTED_V1_VIEWS
        .iter()
        .copied()
        .chain(
            OPERATOR_VIEWS
                .iter()
                .copied()
                .filter(|v| *v != "query_tail" || conditional),
        )
        .collect();
    for expected_view in &expected {
        assert!(
            names.iter().any(|n| n == expected_view),
            "ops.{expected_view} missing from the read model (found: {names:?})"
        );
    }
    assert_eq!(
        names.len(),
        expected.len(),
        "unexpected extra views in schema ops: {names:?} \
         (pg_stat_statements preloaded: {conditional})"
    );

    let reloptions: Vec<(String, Option<Vec<String>>)> = sqlx::query_as(
        "SELECT c.relname, c.reloptions FROM pg_class c \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'ops' AND c.relkind = 'v'",
    )
    .fetch_all(&db.admin)
    .await
    .expect("reloptions");
    for (name, options) in reloptions {
        let invoker = options
            .unwrap_or_default()
            .iter()
            .any(|o| o.contains("security_invoker=true"));
        assert!(
            invoker,
            "ops.{name} must be security_invoker=true: definer-rights views \
             would bypass RLS for non-owner readers"
        );
    }
}

// --- queryability + RLS scoping --------------------------------------------------

#[tokio::test]
async fn views_are_queryable_and_rls_scoped_app_role_vs_owner() {
    let db = test_db().await;
    let sa = seed_org_with_tranche_data(&db.app, 9100).await;
    let sb = seed_org_with_tranche_data(&db.app, 9200).await;

    // Own scope: seeded rows visible through every view, each carrying the
    // tenancy seam (org_id) equal to the bound org.
    {
        let mut tx = org_tx(&db.app, sa.org).await;
        assert_eq!(
            count_view_where_id(&mut tx, "v1_clients", sa.client).await,
            1
        );
        assert_eq!(count_view_where_id(&mut tx, "v1_sites", sa.site).await, 1);
        assert_eq!(
            count_view_where_id(&mut tx, "v1_contacts", sa.contact).await,
            1
        );
        assert_eq!(
            count_view_where_id(&mut tx, "v1_tickets", sa.ticket).await,
            1
        );
        assert_eq!(
            count_view_where_id(&mut tx, "v1_time_entries", sa.time_entry).await,
            1
        );
        assert_eq!(
            count_view_where_id(&mut tx, "v1_invoices", sa.invoice).await,
            1
        );
        assert_eq!(
            count_view_where_id(&mut tx, "v1_invoice_lines", sa.invoice_line).await,
            1
        );
        // Board statuses: the seeded default board's four semantic statuses.
        let statuses: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM ops.v1_board_statuses WHERE board_id = $1")
                .bind(sa.board)
                .fetch_one(&mut *tx)
                .await
                .expect("board status count");
        assert_eq!(statuses, 4);

        // Foreign org's rows are hidden through the views too.
        assert_eq!(
            count_view_where_id(&mut tx, "v1_clients", sb.client).await,
            0
        );
        assert_eq!(
            count_view_where_id(&mut tx, "v1_tickets", sb.ticket).await,
            0
        );
        assert_eq!(
            count_view_where_id(&mut tx, "v1_invoices", sb.invoice).await,
            0
        );

        // Tenancy seam: every view row belongs to the bound org.
        for view in [
            "v1_clients",
            "v1_sites",
            "v1_contacts",
            "v1_board_statuses",
            "v1_invoices",
            "v1_invoice_lines",
            "v1_tickets",
            "v1_time_entries",
        ] {
            let foreign: i64 = sqlx::query_scalar(&format!(
                "SELECT COUNT(*) FROM ops.{view} WHERE org_id <> $1"
            ))
            .bind(sa.org)
            .fetch_one(&mut *tx)
            .await
            .expect("foreign-org rows through view");
            assert_eq!(
                foreign, 0,
                "ops.{view} exposed rows outside the bound org scope"
            );
        }
    }

    // NO scope: default-deny — zero rows through every view (virgin pool;
    // see the harness note on defined-but-empty GUCs).
    {
        let fresh = db.fresh_app_pool().await;
        let mut tx = fresh.begin().await.expect("begin unscoped");
        for view in EXPECTED_V1_VIEWS {
            assert_eq!(
                count_view(&mut tx, view).await,
                0,
                "ops.{view} must default-deny without app.org_id"
            );
        }
    }

    // Owner scope: RLS is bypassed by the table owner, and the views add
    // NO gate of their own — both orgs' rows visible. Proves the scoping
    // above comes from security_invoker + table policies, not the views.
    {
        let mut tx = db.admin.begin().await.expect("begin owner");
        assert_eq!(
            count_view_where_id(&mut tx, "v1_clients", sa.client).await,
            1
        );
        assert_eq!(
            count_view_where_id(&mut tx, "v1_clients", sb.client).await,
            1
        );
        assert_eq!(
            count_view_where_id(&mut tx, "v1_tickets", sa.ticket).await,
            1
        );
        assert_eq!(
            count_view_where_id(&mut tx, "v1_tickets", sb.ticket).await,
            1
        );
        assert_eq!(
            count_view_where_id(&mut tx, "v1_invoices", sa.invoice).await,
            1
        );
        // Total scoped to THIS test's two orgs (sibling tests in this
        // binary seed their own orgs in parallel against the shared DB).
        let clients_total: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM ops.v1_clients WHERE org_id = ANY($1)")
                .bind(vec![sa.org, sb.org])
                .fetch_one(&mut *tx)
                .await
                .expect("org-scoped owner count");
        assert_eq!(clients_total, 2);
    }
}

// --- column contracts -------------------------------------------------------------

#[tokio::test]
async fn tranche_view_columns_match_documented_contracts() {
    let db = test_db().await;
    let mut tx = db.admin.begin().await.expect("begin");

    for (view, expected) in TRANCHE_CONTRACTS {
        let actual = view_columns(&mut tx, view).await;
        let expected: Vec<(String, String)> = expected
            .iter()
            .map(|(n, t)| (n.to_string(), t.to_string()))
            .collect();
        assert_eq!(
            actual, expected,
            "ops.{view} column contract drifted — additive additions must \
             append at the END; renames/removals/retypes require ops.v2_*"
        );
    }

    // org_id tenancy seam present on every view in the schema.
    for view in EXPECTED_V1_VIEWS {
        let columns = view_columns(&mut tx, view).await;
        assert!(
            columns.iter().any(|(n, _)| n == "org_id"),
            "ops.{view} must retain org_id as the tenancy seam"
        );
    }
}

#[tokio::test]
async fn v1_tickets_keeps_0026_column_prefix_and_appends_taxonomy() {
    let db = test_db().await;
    let mut tx = db.admin.begin().await.expect("begin");
    let columns = view_columns(&mut tx, "v1_tickets").await;
    let names: Vec<String> = columns.into_iter().map(|(n, _)| n).collect();

    let mut expected: Vec<&str> = V1_TICKETS_PREFIX.to_vec();
    expected.extend_from_slice(V1_TICKETS_APPENDED);
    let expected: Vec<String> = expected.into_iter().map(String::from).collect();

    assert_eq!(
        names, expected,
        "ops.v1_tickets column order drifted — the 0026 prefix must stay \
         byte-stable and new columns may only append at the end"
    );
}

// --- taxonomy resolution -----------------------------------------------------------

/// One ops.v1_tickets taxonomy row: (id, type_name, category_id,
/// category_name, subcategory_id, subcategory_name).
type TaxonomyRow = (
    Uuid,
    Option<String>,
    Option<Uuid>,
    Option<String>,
    Option<Uuid>,
    Option<String>,
);

#[tokio::test]
async fn v1_tickets_resolves_type_category_and_subcategory() {
    let db = test_db().await;
    let s = seed_org_with_tranche_data(&db.app, 9300).await;

    let (t_type, t_root, t_sub, t_cat_direct) = {
        let mut tx = org_tx(&db.app, s.org).await;
        let t_type = fixed_uuid(9310);
        sqlx::query("INSERT INTO ticket_types (id, org_id, name) VALUES ($1, $2, 'Smoke Type')")
            .bind(t_type)
            .bind(s.org)
            .execute(&mut *tx)
            .await
            .expect("type insert");
        let t_root = fixed_uuid(9311);
        sqlx::query(
            "INSERT INTO ticket_categories (id, org_id, parent_id, name) \
             VALUES ($1, $2, NULL, 'Smoke Category')",
        )
        .bind(t_root)
        .bind(s.org)
        .execute(&mut *tx)
        .await
        .expect("root category insert");
        let t_sub = fixed_uuid(9312);
        sqlx::query(
            "INSERT INTO ticket_categories (id, org_id, parent_id, name) \
             VALUES ($1, $2, $3, 'Smoke Subcategory')",
        )
        .bind(t_sub)
        .bind(s.org)
        .bind(t_root)
        .execute(&mut *tx)
        .await
        .expect("subcategory insert");
        tx.commit().await.expect("commit taxonomy");
        (t_type, t_root, t_sub, t_root)
    };

    // t_ticket: classified with type + SUBCATEGORY (parent resolution needed).
    // t_direct: type + the ROOT category directly.
    // t_unclassified: neither.
    let t_ticket = fixed_uuid(9320);
    let t_direct = fixed_uuid(9321);
    let t_unclassified = fixed_uuid(9322);
    for (id, type_id, category_id) in [
        (t_ticket, Some(t_type), Some(t_sub)),
        (t_direct, Some(t_type), Some(t_cat_direct)),
        (t_unclassified, None, None),
    ] {
        let mut tx = org_tx(&db.app, s.org).await;
        let (prefix, seq) = orgs::next_ticket_number(&mut *tx, s.org)
            .await
            .expect("alloc");
        tickets::insert_ticket(
            &mut tx,
            tickets::NewTicket {
                id,
                org_id: s.org,
                number: &format!("{prefix}-{seq}"),
                board_id: s.board,
                status_id: s.entry_status,
                title: "taxonomy smoke",
                body: None,
                priority: Priority::P3,
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
        sqlx::query("UPDATE tickets SET type_id = $2, category_id = $3 WHERE id = $1")
            .bind(id)
            .bind(type_id)
            .bind(category_id)
            .execute(&mut *tx)
            .await
            .expect("classify ticket");
        tx.commit().await.expect("commit classified ticket");
    }

    let mut tx = org_tx(&db.app, s.org).await;
    let rows: Vec<TaxonomyRow> = sqlx::query_as(
        "SELECT id, ticket_type_name, ticket_category_id, ticket_category_name, \
                ticket_subcategory_id, ticket_subcategory_name \
         FROM ops.v1_tickets WHERE id = ANY($1) ORDER BY id",
    )
    .bind(vec![t_ticket, t_direct, t_unclassified])
    .fetch_all(&mut *tx)
    .await
    .expect("taxonomy view rows");

    assert_eq!(rows.len(), 3);
    let by_id: std::collections::HashMap<Uuid, _> = rows.into_iter().map(|r| (r.0, r)).collect();

    let sub = by_id[&t_ticket].clone();
    assert_eq!(sub.1.as_deref(), Some("Smoke Type"));
    assert_eq!(
        sub.2,
        Some(t_root),
        "subcategory ticket resolves the ROOT category"
    );
    assert_eq!(sub.3.as_deref(), Some("Smoke Category"));
    assert_eq!(sub.4, Some(t_sub));
    assert_eq!(sub.5.as_deref(), Some("Smoke Subcategory"));

    let direct = by_id[&t_direct].clone();
    assert_eq!(direct.1.as_deref(), Some("Smoke Type"));
    assert_eq!(direct.2, Some(t_root));
    assert_eq!(direct.3.as_deref(), Some("Smoke Category"));
    assert!(
        direct.4.is_none() && direct.5.is_none(),
        "root-category ticket has no subcategory"
    );

    let none = by_id[&t_unclassified].clone();
    assert!(
        none.1.is_none()
            && none.2.is_none()
            && none.3.is_none()
            && none.4.is_none()
            && none.5.is_none(),
        "unclassified ticket carries NULL taxonomy columns"
    );
}
