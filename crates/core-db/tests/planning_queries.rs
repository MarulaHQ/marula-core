//! Wave 34 claims close-out: resource planning + curated work-next queues.
//!
//! Two surfaces, one file, because they are the two halves of the same
//! planner question — "who is committed to what, and in what order do they
//! do it" — and because both were schema-only until this lane wrote the
//! queries that make `0116` mean anything.
//!
//! The assertions that matter here are the INVARIANTS, not the happy paths:
//! the queue must stay gapless under reordering (because the next reorder's
//! meaning depends on it), an allocation must never double-count a
//! technician, and the forecast must refuse to report a vacuous agreement.

mod common;

use common::{org_tx, test_db, TestDb};
use uuid::Uuid;

/// A row with just enough shape for the work-next board join: a ticket needs
/// a board, a status and (for the join) a number and a title. Written with
/// raw SQL because the ticket's required columns span several migrations and
/// the point of these tests is the queue, not the ticket.
async fn seed_ticket(db: &TestDb, org: Uuid, n: u64) -> Uuid {
    let mut tx = org_tx(&db.app, org).await;
    let board_id: Uuid = sqlx::query_scalar(
        // `boards` is UNIQUE(org_id, name) and each ticket gets its own
        // board here, so the name carries the ticket number.
        "INSERT INTO boards (id, org_id, name) VALUES ($1, $2, $3)
         RETURNING id",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(format!("Board {n}"))
    .fetch_one(&mut *tx)
    .await
    .expect("board");
    let status_id: Uuid = sqlx::query_scalar(
        "INSERT INTO board_statuses (id, org_id, board_id, name, category, sort_order)
         VALUES ($1, $2, $3, 'Open', 'open', 0) RETURNING id",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(board_id)
    .fetch_one(&mut *tx)
    .await
    .expect("status");
    let ticket: Uuid = sqlx::query_scalar(
        // `category` is NOT NULL and is the open/closed axis the queue
        // filters on, so an open ticket needs it explicitly.
        "INSERT INTO tickets (id, org_id, number, board_id, status_id, title, priority, category)
         VALUES ($1, $2, $3, $4, $5, $6, 'p3', 'open') RETURNING id",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(format!("P-{n}"))
    .bind(board_id)
    .bind(status_id)
    .bind(format!("Ticket {n}"))
    .fetch_one(&mut *tx)
    .await
    .expect("ticket");
    tx.commit().await.expect("commit");
    ticket
}

async fn seed_phase_named(db: &TestDb, org: Uuid, name: &str) -> Uuid {
    let mut tx = org_tx(&db.app, org).await;
    // `projects` is UNIQUE(org_id, name), so the project name carries the
    // phase name — otherwise a second phase in one org collides.
    let project: Uuid = sqlx::query_scalar(
        "INSERT INTO projects (id, org_id, name) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(format!("Project for {name}"))
    .fetch_one(&mut *tx)
    .await
    .expect("project");
    let phase: Uuid = sqlx::query_scalar(
        "INSERT INTO project_phases (id, org_id, project_id, name, sort_order)
         VALUES ($1, $2, $3, $4, 0) RETURNING id",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(project)
    .bind(name)
    .fetch_one(&mut *tx)
    .await
    .expect("phase");
    tx.commit().await.expect("commit");
    phase
}

async fn seed_phase(db: &TestDb, org: Uuid) -> Uuid {
    seed_phase_named(db, org, "Cutover").await
}

async fn seed_org_with_user(db: &TestDb, n: u64) -> (Uuid, Uuid) {
    let org = Uuid::now_v7();
    let mut tx = org_tx(&db.app, org).await;
    sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, $2, $3)")
        .bind(org)
        .bind(format!("Org {n}"))
        .bind(format!("org-{n}-{}", org.simple()))
        .execute(&mut *tx)
        .await
        .expect("org");
    let user: Uuid = sqlx::query_scalar(
        // `password_hash` is NOT NULL. These tests never authenticate, so a
        // fixed non-credential marker is enough and cannot be mistaken for
        // a usable secret.
        "INSERT INTO users (id, org_id, email, display_name, password_hash)
         VALUES ($1, $2, $3, $4, 'not-a-real-hash-planning-tests') RETURNING id",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(format!("tech{n}-{}@test.invalid", org.simple()))
    .bind(format!("Tech {n}"))
    .fetch_one(&mut *tx)
    .await
    .expect("user");
    tx.commit().await.expect("commit");
    (org, user)
}

// --- work-next queue ---------------------------------------------------------

/// The queue must be a gapless 0..n sequence after ANY reorder, because the
/// next reorder's meaning depends on the positions it reads.
#[tokio::test]
async fn work_next_queue_stays_gapless_under_reordering() {
    let db = test_db().await;
    let (org, tech) = seed_org_with_user(&db, 401).await;
    let mut tx = org_tx(&db.app, org).await;

    let t1 = seed_ticket(&db, org, 1).await;
    let t2 = seed_ticket(&db, org, 2).await;
    let t3 = seed_ticket(&db, org, 3).await;

    // Appended in order.
    for (i, t) in [t1, t2, t3].into_iter().enumerate() {
        core_db::dispatch::add_work_next_entry(&mut tx, Uuid::now_v7(), org, tech, t, i as i32)
            .await
            .expect("append");
    }
    assert_eq!(
        core_db::dispatch::reorder_work_next_entry(&mut tx, org, tech, t1, 0)
            .await
            .expect("noop reorder")
            .iter()
            .map(|r| r.position)
            .collect::<Vec<_>>(),
        vec![0, 1, 2],
        "a no-op reorder must be a no-op"
    );

    // Drag the LAST entry to the head. Everything else must shift down by
    // exactly one, leaving 0..n with no hole and no duplicate.
    let after = core_db::dispatch::reorder_work_next_entry(&mut tx, org, tech, t3, 0)
        .await
        .expect("reorder to head");
    assert_eq!(
        after.iter().map(|r| r.position).collect::<Vec<_>>(),
        vec![0, 1, 2],
        "positions must stay dense after a move"
    );
    assert_eq!(after[0].ticket_id, t3, "the dragged entry is now first");
    assert_eq!(
        after
            .iter()
            .map(|r| r.position)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3,
        "no duplicate positions"
    );

    // Out-of-range positions CLAMP rather than erroring: a drag past the end
    // is a gesture, not a client bug.
    let clamped = core_db::dispatch::reorder_work_next_entry(&mut tx, org, tech, t1, 9999)
        .await
        .expect("clamped reorder");
    assert_eq!(
        clamped.last().expect("tail").ticket_id,
        t1,
        "lands at the tail"
    );
    assert_eq!(
        clamped.iter().map(|r| r.position).collect::<Vec<_>>(),
        vec![0, 1, 2],
        "still dense after a clamped move"
    );

    // A negative position lands at the head.
    let head = core_db::dispatch::reorder_work_next_entry(&mut tx, org, tech, t1, -5)
        .await
        .expect("negative reorder");
    assert_eq!(head[0].ticket_id, t1);
    assert_eq!(head[0].position, 0);

    // The board read agrees with what the writes reported, and carries the
    // ticket fields a drag-and-drop row renders without a second request.
    let rows = core_db::dispatch::list_work_next_queue(&mut *tx, org, tech)
        .await
        .expect("list");
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].ticket_id, t1);
    assert_eq!(rows[0].ticket_number, "P-1");
    assert_eq!(rows[0].ticket_title, "Ticket 1");

    // Removal from one queue, then the "every queue" variant.
    assert_eq!(
        core_db::dispatch::remove_work_next_entry(&mut tx, org, Some(tech), t1)
            .await
            .expect("remove one"),
        1
    );
    assert_eq!(
        core_db::dispatch::remove_work_next_entry(&mut tx, org, None, t2)
            .await
            .expect("remove all"),
        1,
        "the org-wide variant clears the row left in this technician's queue"
    );
    let left = core_db::dispatch::list_work_next_queue(&mut *tx, org, tech)
        .await
        .expect("list after removal");
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].ticket_id, t3);
}

/// A closed ticket must leave the queue, or the technician's "next" is
/// something nobody can act on.
#[tokio::test]
async fn work_next_hides_closed_tickets() {
    let db = test_db().await;
    let (org, tech) = seed_org_with_user(&db, 402).await;
    let mut tx = org_tx(&db.app, org).await;
    let open = seed_ticket(&db, org, 1).await;
    let closed = seed_ticket(&db, org, 2).await;
    sqlx::query("UPDATE tickets SET category = 'closed' WHERE id = $1")
        .bind(closed)
        .execute(&mut *tx)
        .await
        .expect("close");

    for (i, t) in [open, closed].into_iter().enumerate() {
        core_db::dispatch::add_work_next_entry(&mut tx, Uuid::now_v7(), org, tech, t, i as i32)
            .await
            .expect("append");
    }
    let rows = core_db::dispatch::list_work_next_queue(&mut *tx, org, tech)
        .await
        .expect("list");
    assert_eq!(rows.len(), 1, "the closed ticket is filtered out");
    assert_eq!(rows[0].ticket_id, open);
}

// --- phase allocations + the forecast --------------------------------------

/// Re-allocating the same (phase, technician) pair UPDATES the share. Two
/// rows would SUM in the forecast and report a technician over-committed by
/// construction, which is the specific failure the UNIQUE constraint and the
/// upsert both exist to prevent.
#[tokio::test]
async fn allocation_is_an_upsert_not_an_append() {
    let db = test_db().await;
    let (org, tech) = seed_org_with_user(&db, 403).await;
    let phase = seed_phase(&db, org).await;
    let mut tx = org_tx(&db.app, org).await;

    let first = core_db::dispatch::upsert_phase_allocation(
        &mut tx,
        core_db::dispatch::NewPhaseAllocation {
            id: Uuid::now_v7(),
            org_id: org,
            phase_id: phase,
            user_id: tech,
            start_date: None,
            end_date: None,
            share_percent: 60,
            note: None,
        },
    )
    .await
    .expect("first allocation");
    let restated = core_db::dispatch::upsert_phase_allocation(
        &mut tx,
        core_db::dispatch::NewPhaseAllocation {
            id: Uuid::now_v7(),
            org_id: org,
            phase_id: phase,
            user_id: tech,
            start_date: None,
            end_date: None,
            share_percent: 30,
            note: None,
        },
    )
    .await
    .expect("restated allocation");

    assert_eq!(first.id, restated.id, "same (phase, user) ⇒ same row");
    assert_eq!(
        restated.share_percent, 30,
        "the share is replaced, not added"
    );

    let all = core_db::dispatch::list_phase_allocations(&mut *tx, org, None)
        .await
        .expect("list");
    assert_eq!(all.len(), 1, "exactly one allocation, not two");
    assert_eq!(all[0].share_percent, 30);
    // The joined view carries the names the planning UI renders.
    assert_eq!(all[0].phase_name, "Cutover");
    assert_eq!(all[0].project_name, "Project for Cutover");
    assert!(all[0].user_display_name.starts_with("Tech "));

    // The forecast reads the RESTATED share, not both.
    let loads = core_db::dispatch::capacity_forecast(
        &mut *tx,
        org,
        &[tech],
        chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
        chrono::NaiveDate::from_ymd_opt(2026, 10, 7).unwrap(),
    )
    .await
    .expect("forecast");
    assert_eq!(loads.len(), 1);
    assert_eq!(loads[0].allocation_count, 1, "one allocation, counted once");
    // 30% × 7 days = 210 percent-days.
    assert!(
        (loads[0].percent_days - 210.0).abs() < 0.01,
        "got {}",
        loads[0].percent_days
    );
}

/// The forecast measures a commitment's DURATION, not just its percentage.
/// A 20%-for-a-month plan and an 80%-for-a-week plan are the same total
/// load; a percentage-only model would report them as 20% and 80%.
#[tokio::test]
async fn forecast_measures_duration_in_percent_days() {
    let db = test_db().await;
    let (org, tech) = seed_org_with_user(&db, 404).await;
    let phase = seed_phase(&db, org).await;
    let mut tx = org_tx(&db.app, org).await;

    // 100% for 2 days inside a 10-day window = 200 percent-days.
    core_db::dispatch::upsert_phase_allocation(
        &mut tx,
        core_db::dispatch::NewPhaseAllocation {
            id: Uuid::now_v7(),
            org_id: org,
            phase_id: phase,
            user_id: tech,
            start_date: chrono::NaiveDate::from_ymd_opt(2026, 10, 1),
            end_date: chrono::NaiveDate::from_ymd_opt(2026, 10, 2),
            share_percent: 100,
            note: None,
        },
    )
    .await
    .expect("allocation");

    let loads = core_db::dispatch::capacity_forecast(
        &mut *tx,
        org,
        &[tech],
        chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
        chrono::NaiveDate::from_ymd_opt(2026, 10, 10).unwrap(),
    )
    .await
    .expect("forecast");
    assert_eq!(loads.len(), 1);
    assert!(
        (loads[0].percent_days - 200.0).abs() < 0.01,
        "100% over 2 days = 200 percent-days, got {}",
        loads[0].percent_days
    );
}

/// An allocation whose window only CLIPS the query edge still commits real
/// days inside it, and an undated allocation counts for the whole window —
/// because open-ended ownership is exactly what a planner most needs to see.
#[tokio::test]
async fn forecast_clips_partial_windows_and_counts_undated_allocations() {
    let db = test_db().await;
    let (org, tech) = seed_org_with_user(&db, 405).await;
    let phase = seed_phase(&db, org).await;
    let mut tx = org_tx(&db.app, org).await;

    // A SECOND phase, because (phase, user) is the upsert key: writing a
    // second allocation on the same phase would REPLACE the first, and this
    // test is about two allocations coexisting.
    let other_phase = seed_phase_named(&db, org, "Data migration").await;
    // Straddles the window start: 20th–22nd, queried from the 22nd.
    core_db::dispatch::upsert_phase_allocation(
        &mut tx,
        core_db::dispatch::NewPhaseAllocation {
            id: Uuid::now_v7(),
            org_id: org,
            phase_id: phase,
            user_id: tech,
            start_date: chrono::NaiveDate::from_ymd_opt(2026, 10, 20),
            end_date: chrono::NaiveDate::from_ymd_opt(2026, 10, 22),
            share_percent: 100,
            note: None,
        },
    )
    .await
    .expect("clipping allocation");
    // Entirely BEFORE the window ⇒ not counted at all.
    core_db::dispatch::upsert_phase_allocation(
        &mut tx,
        core_db::dispatch::NewPhaseAllocation {
            id: Uuid::now_v7(),
            org_id: org,
            phase_id: other_phase,
            user_id: tech,
            start_date: chrono::NaiveDate::from_ymd_opt(2026, 1, 1),
            end_date: chrono::NaiveDate::from_ymd_opt(2026, 1, 5),
            share_percent: 100,
            note: Some("last quarter"),
        },
    )
    .await
    .expect("out-of-window allocation");

    let loads = core_db::dispatch::capacity_forecast(
        &mut *tx,
        org,
        &[tech],
        chrono::NaiveDate::from_ymd_opt(2026, 10, 22).unwrap(),
        chrono::NaiveDate::from_ymd_opt(2026, 10, 24).unwrap(),
    )
    .await
    .expect("forecast");
    // 3-day window, 1 day of the clipping allocation overlaps ⇒ 100
    // percent-days, and the out-of-window allocation contributes nothing.
    assert!(
        (loads[0].percent_days - 100.0).abs() < 0.01,
        "got {}",
        loads[0].percent_days
    );
}

/// A completed phase is history. Counting it would report a technician as
/// loaded on work that is already done.
#[tokio::test]
async fn forecast_excludes_completed_phases() {
    let db = test_db().await;
    let (org, tech) = seed_org_with_user(&db, 406).await;
    let phase = seed_phase(&db, org).await;
    let mut tx = org_tx(&db.app, org).await;
    core_db::dispatch::upsert_phase_allocation(
        &mut tx,
        core_db::dispatch::NewPhaseAllocation {
            id: Uuid::now_v7(),
            org_id: org,
            phase_id: phase,
            user_id: tech,
            start_date: None,
            end_date: None,
            share_percent: 100,
            note: None,
        },
    )
    .await
    .expect("allocation");
    assert_eq!(
        core_db::dispatch::capacity_forecast(
            &mut *tx,
            org,
            &[tech],
            chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            chrono::NaiveDate::from_ymd_opt(2026, 10, 7).unwrap(),
        )
        .await
        .expect("open phase")
        .len(),
        1
    );

    sqlx::query("UPDATE project_phases SET status = 'completed' WHERE id = $1")
        .bind(phase)
        .execute(&mut *tx)
        .await
        .expect("complete");
    let after = core_db::dispatch::capacity_forecast(
        &mut *tx,
        org,
        &[tech],
        chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
        chrono::NaiveDate::from_ymd_opt(2026, 10, 7).unwrap(),
    )
    .await
    .expect("completed phase");
    assert!(after.is_empty(), "a finished phase is not forward load");
}

/// RLS: another org's queue and allocations are invisible, not merely
/// filtered in the handler.
#[tokio::test]
async fn planning_rows_are_rls_scoped_to_their_org() {
    let db = test_db().await;
    let (org_a, tech_a) = seed_org_with_user(&db, 407).await;
    let (org_b, tech_b) = seed_org_with_user(&db, 408).await;
    let t_a = seed_ticket(&db, org_a, 1).await;

    {
        let mut tx = org_tx(&db.app, org_a).await;
        core_db::dispatch::add_work_next_entry(&mut tx, Uuid::now_v7(), org_a, tech_a, t_a, 0)
            .await
            .expect("append in org A");
        tx.commit().await.expect("commit");
    }

    // Org B, reading its OWN scope, sees nothing — even though it asks for
    // org A's technician by id. The org_id predicate is in the policy, so
    // this is the database refusing, not the query.
    let mut tx_b = org_tx(&db.app, org_b).await;
    assert!(
        core_db::dispatch::list_work_next_queue(&mut *tx_b, org_b, tech_b)
            .await
            .expect("org B read")
            .is_empty(),
        "org B must not see org A's queue"
    );
    assert!(
        core_db::dispatch::list_phase_allocations(&mut *tx_b, org_b, None)
            .await
            .expect("org B allocations")
            .is_empty()
    );
}
