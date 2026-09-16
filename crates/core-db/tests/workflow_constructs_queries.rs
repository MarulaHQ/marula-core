//! Workflow-constructs persistence tests (Wave 11 lane 9, migration 0056).
//!
//! Covers the promises the API layer leans on: the tag taxonomy + AND
//! filter semantics on the ticket list, the link/rollup reads and the
//! one-shot merge fold, the costs/expenses candidate feed + "billed once,
//! ever" consumption marker, and the PM-schedule claim/ledger gate.

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::{costs, orgs, recurring, tags, ticket_links, tickets, users};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

async fn seed_org(app: &PgPool, n: u64) -> Uuid {
    let org = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    orgs::insert_org(&mut *tx, org, "Acme MSP", &format!("wf-{n}"), "MER")
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

async fn seed_user(app: &PgPool, org: Uuid, n: u64) -> Uuid {
    let user = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    users::create_user(
        &mut *tx,
        user,
        org,
        &format!("u{n}@wf.test"),
        "Test User",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user insert");
    tx.commit().await.expect("commit user");
    user
}

async fn seed_ticket(
    conn: &mut PgConnection,
    org: Uuid,
    board: Uuid,
    status: Uuid,
    id: Uuid,
    title: &str,
) -> core_domain::entities::Ticket {
    let (prefix, seq) = orgs::next_ticket_number(&mut *conn, org)
        .await
        .expect("number alloc");
    let number = format!("{prefix}-{seq}");
    tickets::insert_ticket(
        conn,
        tickets::NewTicket {
            id,
            org_id: org,
            number: &number,
            board_id: board,
            status_id: status,
            title,
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
    .expect("ticket insert")
}

/// A raw org connection (no RLS scope needed for these single-statement
/// writes; org_id rides every query).
async fn org_conn<'a>(app: &'a PgPool, org: Uuid) -> sqlx::Transaction<'a, sqlx::Postgres> {
    org_tx(app, org).await
}

async fn board_and_status(app: &PgPool, org: Uuid) -> (Uuid, Uuid, Uuid) {
    let mut tx = org_tx(app, org).await;
    let board: Uuid = sqlx::query_scalar("SELECT id FROM boards WHERE org_id = $1")
        .bind(org)
        .fetch_one(&mut *tx)
        .await
        .expect("default board");
    let status: Uuid =
        sqlx::query_scalar("SELECT id FROM board_statuses WHERE board_id = $1 AND name = 'New'")
            .bind(board)
            .fetch_one(&mut *tx)
            .await
            .expect("entry status");
    let closed: Uuid = sqlx::query_scalar(
        "SELECT id FROM board_statuses WHERE board_id = $1 AND category = 'closed' LIMIT 1",
    )
    .bind(board)
    .fetch_one(&mut *tx)
    .await
    .expect("closed status");
    tx.commit().await.expect("commit");
    (board, status, closed)
}

// --- tags ---------------------------------------------------------------------

#[tokio::test]
async fn tags_crud_apply_and_and_filter() {
    let db = test_db().await;
    let org = seed_org(&db.app, 900).await;
    let (board, status, _closed_status) = board_and_status(&db.app, org).await;
    let t1 = {
        let mut conn = org_conn(&db.app, org).await;
        let t = seed_ticket(&mut conn, org, board, status, fixed_uuid(901), "one").await;
        conn.commit().await.unwrap();
        t
    };
    let t2 = {
        let mut conn = org_conn(&db.app, org).await;
        let t = seed_ticket(&mut conn, org, board, status, fixed_uuid(902), "two").await;
        conn.commit().await.unwrap();
        t
    };

    let tag_a = fixed_uuid(910);
    let tag_b = fixed_uuid(911);
    {
        let mut tx = org_tx(&db.app, org).await;
        tags::insert_tag(
            &mut *tx,
            &tags::NewTag {
                id: tag_a,
                org_id: org,
                name: "hardware",
                color: Some("#f00"),
                created_by: None,
            },
        )
        .await
        .expect("tag a");
        tags::insert_tag(
            &mut *tx,
            &tags::NewTag {
                id: tag_b,
                org_id: org,
                name: "network",
                color: None,
                created_by: None,
            },
        )
        .await
        .expect("tag b");
        tags::apply_tag(&mut *tx, org, t1.id, tag_a, None)
            .await
            .expect("apply a");
        tags::apply_tag(&mut *tx, org, t1.id, tag_b, None)
            .await
            .expect("apply b");
        tags::apply_tag(&mut *tx, org, t2.id, tag_a, None)
            .await
            .expect("apply a2");
        // idempotent re-apply
        tags::apply_tag(&mut *tx, org, t1.id, tag_a, None)
            .await
            .expect("re-apply");
        tx.commit().await.unwrap();
    }

    // AND filter: both tags matches only t1.
    {
        let mut tx = org_tx(&db.app, org).await;
        let both = tickets::list_tickets(
            &mut *tx,
            org,
            tickets::TicketFilter {
                tag_ids: Some(vec![tag_a, tag_b]),
                ..filter_all()
            },
        )
        .await
        .expect("both filter");
        assert_eq!(both.iter().map(|t| t.id).collect::<Vec<_>>(), vec![t1.id]);

        let only_a = tickets::list_tickets(
            &mut *tx,
            org,
            tickets::TicketFilter {
                tag_ids: Some(vec![tag_a]),
                ..filter_all()
            },
        )
        .await
        .expect("a filter");
        assert_eq!(only_a.len(), 2, "tag A alone matches both tickets");

        let counts = tags::tag_application_counts(&mut *tx, org)
            .await
            .expect("counts");
        let a_count = counts.iter().find(|(id, _)| *id == tag_a).unwrap().1;
        assert_eq!(a_count, 2);

        let removed = tags::remove_tag(&mut *tx, org, t2.id, tag_a)
            .await
            .expect("remove");
        assert!(removed);
        let applied = tags::list_ticket_tags(&mut *tx, org, t2.id)
            .await
            .expect("list");
        assert!(applied.is_empty());
        tx.commit().await.unwrap();
    }

    // Delete cascades the join rows; count is cited for the event payload.
    {
        let mut tx = org_tx(&db.app, org).await;
        let applied = tags::delete_tag(&mut tx, org, tag_a).await.expect("delete");
        assert_eq!(applied, 1, "t1 still carried tag A");
        tx.commit().await.unwrap();
        let mut tx = org_tx(&db.app, org).await;
        assert!(tags::get_tag(&mut *tx, org, tag_a).await.is_err());
        let remaining = tags::list_ticket_tags(&mut *tx, org, t1.id)
            .await
            .expect("t1 tags");
        assert_eq!(remaining.len(), 1, "only tag B survives");
    }
}

fn filter_all() -> tickets::TicketFilter {
    tickets::TicketFilter {
        board_id: None,
        assignee_id: None,
        client_id: None,
        category: None,
        priority: None,
        tag_ids: None,
        limit: 100,
        offset: 0,
        cursor: None,
    }
}

// --- links + merge ----------------------------------------------------------------

#[tokio::test]
async fn links_rollup_and_merge_fold() {
    let db = test_db().await;
    let org = seed_org(&db.app, 950).await;
    let (board, status, closed_status) = board_and_status(&db.app, org).await;
    let survivor = {
        let mut conn = org_conn(&db.app, org).await;
        let t = seed_ticket(&mut conn, org, board, status, fixed_uuid(951), "survivor").await;
        conn.commit().await.unwrap();
        t
    };
    let source = {
        let mut conn = org_conn(&db.app, org).await;
        let t = seed_ticket(&mut conn, org, board, status, fixed_uuid(952), "source").await;
        conn.commit().await.unwrap();
        t
    };
    let child = {
        let mut conn = org_conn(&db.app, org).await;
        let t = seed_ticket(&mut conn, org, board, status, fixed_uuid(953), "child").await;
        conn.commit().await.unwrap();
        t
    };

    let user = seed_user(&db.app, org, 955).await;
    {
        let mut tx = org_tx(&db.app, org).await;
        // Fold materials on the source: a comment, a time entry, a cost
        // line, an expense.
        sqlx::query(
            "INSERT INTO ticket_comments (id, org_id, ticket_id, kind, body) \
             VALUES ($1, $2, $3, 'internal', 'to fold')",
        )
        .bind(fixed_uuid(954))
        .bind(org)
        .bind(source.id)
        .execute(&mut *tx)
        .await
        .expect("comment");
        sqlx::query(
            "INSERT INTO time_entries (id, org_id, ticket_id, user_id, started_at, ended_at, minutes) \
             VALUES ($1, $2, $3, $4, now() - interval '1 hour', now(), 30)",
        )
        .bind(fixed_uuid(955))
        .bind(org)
        .bind(source.id)
        .bind(user)
        .execute(&mut *tx)
        .await
        .expect("time");
        costs::insert_cost_line(
            &mut *tx,
            &core_domain::entities::TicketCostLine {
                fx_rate_used: None,
                fx_rate_date: None,
                fx_source: None,
                base_amount_minor: None,
                id: fixed_uuid(956),
                org_id: org,
                ticket_id: source.id,
                kind: core_domain::entities::CostLineKind::Parts,
                description: "PSU".into(),
                quantity: 1.0,
                unit_cost_minor: 1_000,
                currency: "USD".into(),
                billable: true,
                invoice_line_id: None,
                created_by: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
        )
        .await
        .expect("cost");
        costs::insert_expense(
            &mut *tx,
            &core_domain::entities::TicketExpense {
                fx_rate_used: None,
                fx_rate_date: None,
                fx_source: None,
                base_amount_minor: None,
                id: fixed_uuid(957),
                org_id: org,
                ticket_id: source.id,
                description: "Courier".into(),
                amount_minor: 500,
                currency: "USD".into(),
                billable: true,
                invoice_line_id: None,
                incurred_by: None,
                incurred_at: chrono::Utc::now(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
        )
        .await
        .expect("expense");
        // Source is a parent of `child`.
        let link = core_domain::entities::TicketLink {
            id: fixed_uuid(958),
            org_id: org,
            from_ticket_id: child.id,
            to_ticket_id: source.id,
            link_type: core_domain::entities::TicketLinkType::ParentChild,
            created_by: None,
            created_at: chrono::Utc::now(),
        };
        ticket_links::insert_link(&mut *tx, &link)
            .await
            .expect("link");

        // Pre-merge rollup of the source: one child, 30 min, 1_500 billable.
        let rollup = ticket_links::rollup_for_ticket(&mut *tx, org, source.id)
            .await
            .expect("rollup");
        assert_eq!(rollup.child_count, 1);
        assert_eq!(rollup.time_minutes, 30);
        assert_eq!(rollup.billable_cost_minor, 1_500);
        tx.commit().await.unwrap();
    }

    // The fold, in one transaction.
    {
        let mut tx = org_tx(&db.app, org).await;
        let fold = ticket_links::merge_fold(&mut tx, org, source.id, survivor.id)
            .await
            .expect("merge fold");
        assert_eq!(fold.comments_folded, 1);
        assert_eq!(fold.time_entries_folded, 1);
        assert_eq!(fold.cost_lines_folded, 1);
        assert_eq!(fold.expenses_folded, 1);
        assert_eq!(fold.children_relinked, 1, "child re-points to the survivor");

        // Close the source through the board-move path (category crossing
        // stamps resolved_at; the merge marker was set by the fold).
        tickets::move_to_board(&mut *tx, org, source.id, board, closed_status)
            .await
            .expect("close source");
        tx.commit().await.unwrap();
    }

    {
        let mut tx = org_tx(&db.app, org).await;
        let source_after = tickets::get_ticket(&mut *tx, org, source.id)
            .await
            .expect("source");
        assert_eq!(source_after.merged_into_ticket_id, Some(survivor.id));
        assert_eq!(
            source_after.category,
            core_domain::status::TicketStatusCategory::Closed
        );

        // The survivor carries the folded comment + rollup of child work.
        let comments: i64 =
            sqlx::query_scalar("SELECT count(*) FROM ticket_comments WHERE ticket_id = $1")
                .bind(survivor.id)
                .fetch_one(&mut *tx)
                .await
                .expect("comments");
        assert_eq!(comments, 1);
        let rollup = ticket_links::rollup_for_ticket(&mut *tx, org, survivor.id)
            .await
            .expect("survivor rollup");
        assert_eq!(rollup.child_count, 1);
        assert_eq!(rollup.time_minutes, 30);
        assert_eq!(rollup.billable_cost_minor, 1_500);

        // Idempotence: a second fold attempt refuses (already merged).
        let again = ticket_links::merge_fold(&mut tx, org, source.id, survivor.id).await;
        assert!(again.is_err(), "merged source must not fold twice");
    }
}

// --- costs & expenses --------------------------------------------------------------

#[tokio::test]
async fn costs_candidate_feed_and_billed_once_guard() {
    let db = test_db().await;
    let org = seed_org(&db.app, 970).await;
    let (board, status, _closed_status) = board_and_status(&db.app, org).await;
    let t = {
        let mut conn = org_conn(&db.app, org).await;
        let t = seed_ticket(&mut conn, org, board, status, fixed_uuid(971), "billable").await;
        conn.commit().await.unwrap();
        t
    };
    let cost_id = fixed_uuid(972);
    let expense_id = fixed_uuid(973);

    {
        let mut tx = org_tx(&db.app, org).await;
        costs::insert_cost_line(
            &mut *tx,
            &core_domain::entities::TicketCostLine {
                fx_rate_used: None,
                fx_rate_date: None,
                fx_source: None,
                base_amount_minor: None,
                id: cost_id,
                org_id: org,
                ticket_id: t.id,
                kind: core_domain::entities::CostLineKind::Flat,
                description: "Flat diagnostics".into(),
                quantity: 2.0,
                unit_cost_minor: 5_000,
                currency: "USD".into(),
                billable: true,
                invoice_line_id: None,
                created_by: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
        )
        .await
        .expect("cost");
        costs::insert_expense(
            &mut *tx,
            &core_domain::entities::TicketExpense {
                fx_rate_used: None,
                fx_rate_date: None,
                fx_source: None,
                base_amount_minor: None,
                id: expense_id,
                org_id: org,
                ticket_id: t.id,
                description: "Parts courier".into(),
                amount_minor: 1_200,
                currency: "USD".into(),
                billable: true,
                invoice_line_id: None,
                incurred_by: None,
                incurred_at: chrono::Utc::now(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
        )
        .await
        .expect("expense");
        tx.commit().await.unwrap();
    }

    // Candidate feed: both rows, non-billable excluded.
    {
        let mut tx = org_tx(&db.app, org).await;
        let candidates = costs::list_invoice_candidates(&mut tx, org)
            .await
            .expect("candidates");
        assert_eq!(candidates.len(), 2);
        let cost = candidates
            .iter()
            .find(|c| c.source == "cost")
            .expect("cost row");
        assert_eq!(cost.amount_minor, 10_000, "quantity x unit cost");
        let expense = candidates
            .iter()
            .find(|c| c.source == "expense")
            .expect("expense row");
        assert_eq!(expense.amount_minor, 1_200);

        // Consumption marks both; "billed once, ever".
        costs::mark_candidates_billed(&mut tx, org, &[cost_id], &[expense_id], fixed_uuid(974))
            .await
            .expect("mark billed");
        let after = costs::list_invoice_candidates(&mut tx, org)
            .await
            .expect("after");
        assert!(after.is_empty(), "marked rows leave the candidate feed");

        // A billed cost line refuses every mutation.
        let billed_guard =
            costs::update_cost_line(&mut tx, org, cost_id, Some("edited"), None, None, None).await;
        assert!(billed_guard.is_err(), "billed line must not edit");
        let billed_guard = costs::remove_cost_line(&mut tx, org, cost_id).await;
        assert!(billed_guard.is_err(), "billed line must not delete");
        tx.commit().await.unwrap();
    }
}

// --- PM schedules --------------------------------------------------------------------

#[tokio::test]
async fn pm_schedules_claim_ledger_and_advance() {
    let db = test_db().await;
    let org = seed_org(&db.app, 990).await;
    let (board, _status, _closed) = board_and_status(&db.app, org).await;
    let schedule_id = fixed_uuid(991);
    let user = seed_user(&db.app, org, 995).await;
    let pm_ticket = {
        let (board0, status0, _) = board_and_status(&db.app, org).await;
        let mut conn = org_conn(&db.app, org).await;
        let t = seed_ticket(
            &mut conn,
            org,
            board0,
            status0,
            fixed_uuid(996),
            "pm target",
        )
        .await;
        conn.commit().await.unwrap();
        t
    };

    // Past-due daily schedule.
    {
        let mut tx = org_tx(&db.app, org).await;
        recurring::insert_schedule(
            &mut *tx,
            &recurring::NewPmSchedule {
                id: schedule_id,
                org_id: org,
                name: "Nightly check",
                board_id: board,
                title: "Run backups",
                body: None,
                priority: "p3",
                client_id: None,
                site_id: None,
                assignee_id: None,
                cadence: "daily",
                time_of_day: chrono::NaiveTime::from_hms_opt(3, 0, 0).unwrap(),
                day_of_week: None,
                day_of_month: None,
                timezone: "UTC",
                next_run_at: chrono::Utc::now() - chrono::Duration::hours(2),
                created_by: user,
            },
        )
        .await
        .expect("schedule insert");
        tx.commit().await.unwrap();
    }

    let slot;
    {
        // Claim: exactly one due row, slot = pre-claim next_run_at, and
        // the defensive +1h bump keeps it unclaimable in the same pass.
        let claims = recurring::claim_due_schedules(&db.admin, 10)
            .await
            .expect("claim");
        assert_eq!(claims.len(), 1);
        let c = &claims[0];
        assert_eq!(c.id, schedule_id);
        assert_eq!(c.org_id, org);
        slot = c.slot_at;
        assert!(c.slot_at <= chrono::Utc::now() - chrono::Duration::hours(1));

        // Stamp the next occurrence via cadence math.
        let stamped = recurring::stamp_next_run(&db.admin, c, None)
            .await
            .expect("stamp");
        assert!(stamped, "daily cadence always has a next occurrence");
        let next: chrono::DateTime<chrono::Utc> = {
            let mut tx = org_tx(&db.app, org).await;
            sqlx::query_scalar("SELECT next_run_at FROM pm_schedules WHERE id = $1")
                .bind(schedule_id)
                .fetch_one(&mut *tx)
                .await
                .expect("next run")
        };
        assert!(next > slot, "next occurrence strictly after the slot");
    }

    {
        let mut tx = org_tx(&db.app, org).await;
        // Ledger gate: first claim wins, replay is inert.
        let run_id = fixed_uuid(992);
        let first = recurring::claim_run_slot(&mut *tx, run_id, org, schedule_id, slot)
            .await
            .expect("claim slot");
        assert!(first, "first discharge inserts the ledger row");
        let replay = recurring::claim_run_slot(&mut *tx, fixed_uuid(993), org, schedule_id, slot)
            .await
            .expect("replay slot");
        assert!(!replay, "same schedule+slot can never double-generate");

        recurring::record_run_ticket(&mut *tx, org, run_id, pm_ticket.id)
            .await
            .expect("record ticket");
        let runs = recurring::list_runs(&mut *tx, org, schedule_id, 10)
            .await
            .expect("runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "generated");
        assert_eq!(runs[0].ticket_id, Some(pm_ticket.id));
        tx.commit().await.unwrap();
    }
}
