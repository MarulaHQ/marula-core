//! Agent-platform proposal listing: the cursorless first page.
//!
//! Purpose: one defect, proved as a test. `list_proposals` bound a cursor-less
//! request to `(created_at, id) < ('1970-01-01', Uuid::nil())`, which is false
//! for every row written after the epoch. The first page is the only page a
//! caller holding no `next_cursor` can ask for, so the proposal list came back
//! empty even with a full table — and every consumer read that as "there are no
//! proposals". It cost the p41 UI-audit lane a misdiagnosis: it attributed the
//! workbench's empty "RECOMMENDED NEXT STEP" card to a missing demo fixture,
//! when the panel simply could not read the proposals that existed.
//!
//! The three claims this suite pins:
//!
//! 1. a cursor-less listing returns the org's newest proposals, not an empty page;
//! 2. the same listing on a later page keeps working, so the fix did not just
//!    widen the first page and break the cursor contract;
//! 3. another org's proposals stay invisible (RLS plus the explicit org_id),
//!    which is the half of the predicate the bug never touched and the fix must
//!    not disturb.
//!
//! Slot discipline: this suite owns fixed-UUID slots 91100–91149 only.
//!
//! Non-purpose: HTTP behaviour (core-api flow test), and the proposal decision
//! flow (core-api route tests). Database selection follows the documented
//! contract in `tests/common/mod.rs`.

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::{agent_platform, orgs};
use serde_json::json;
use uuid::Uuid;

/// Seeds one org (defaults-first roles) at slot `n`.
async fn seed_org(db: &common::TestDb, n: u64, slug: &str) -> Uuid {
    let org = fixed_uuid(n);
    let mut tx = org_tx(&db.app, org).await;
    orgs::insert_org(&mut *tx, org, "Proposal Co", slug, "MER")
        .await
        .expect("org insert");
    orgs::seed_default_roles(&mut tx, org).await.expect("roles");
    tx.commit().await.expect("commit org");
    org
}

/// Files one proposal against `org`, in the `proposed` state. `row_id` is a
/// disjoint slot per row so parallel rows never collide on a fixed UUID.
async fn file_proposal(db: &common::TestDb, org: Uuid, row_id: Uuid, rationale: &str) {
    let mut tx = org_tx(&db.app, org).await;
    agent_platform::insert_proposal(
        &mut tx,
        row_id,
        org,
        &agent_platform::ProposalFields {
            entity: "ticket".into(),
            entity_id: Some(row_id),
            op: "update".into(),
            before_value: json!({}),
            proposed_value: json!({ "next_step": "remote_triage_before_dispatch" }),
            classification: "internal".into(),
            capability: None,
            dial_mode: "proposed".into(),
            base_version: 1,
            agent_kind: "agent".into(),
            agent_ref: "user:test".into(),
            skill_slug: Some("triage".into()),
            skill_version: None,
            rule_set_hash: None,
            session_id: None,
            tool: None,
            for_user: None,
            rationale: Some(rationale.into()),
        },
    )
    .await
    .expect("proposal insert");
    tx.commit().await.expect("commit proposal");
}

#[tokio::test]
async fn a_cursorless_listing_returns_the_first_page() {
    let db = test_db().await;
    let org = seed_org(&db, 91100, "proposal-first-page").await;

    // Three rows, so a page of 2 has something to page into.
    file_proposal(&db, org, fixed_uuid(91101), "row one").await;
    file_proposal(&db, org, fixed_uuid(91102), "row two").await;
    file_proposal(&db, org, fixed_uuid(91103), "row three").await;

    let mut tx = org_tx(&db.app, org).await;
    let rows = agent_platform::list_proposals(&mut *tx, org, None, None, None, 2)
        .await
        .expect("cursorless list");

    // This assertion is the bug: it returned an empty vec.
    assert_eq!(rows.len(), 2, "the cursorless first page must not be empty");
    // Newest first, and the order must be stable between calls.
    assert_eq!(rows[0].rationale.as_deref(), Some("row three"));
    assert_eq!(rows[1].rationale.as_deref(), Some("row two"));
    tx.commit().await.expect("commit read");
}

#[tokio::test]
async fn a_cursorless_listing_pages_forward() {
    let db = test_db().await;
    let org = seed_org(&db, 91104, "proposal-second-page").await;

    file_proposal(&db, org, fixed_uuid(91105), "page one a").await;
    file_proposal(&db, org, fixed_uuid(91106), "page one b").await;
    file_proposal(&db, org, fixed_uuid(91107), "page two c").await;

    let mut tx = org_tx(&db.app, org).await;
    let first = agent_platform::list_proposals(&mut *tx, org, None, None, None, 2)
        .await
        .expect("first page");
    assert_eq!(
        first.len(),
        2,
        "the cursorless first page must come back full"
    );

    // The cursor contract: paging forward from the first page's last row
    // yields exactly the rows the first page did not show — no row twice, none
    // skipped. A sentinel-based "fix" (binding a no-cursor request to an epoch
    // rather than NULL) would widen the first page and quietly break this, so
    // assert the contract rather than any particular order. Ordering itself is
    // `created_at DESC, id DESC`, and `now()` is transaction-scoped, so rows
    // written back to back tie on `created_at` and the tie-break is the id.
    let last = first.last().expect("a last row on a full page");
    let second = agent_platform::list_proposals(
        &mut *tx,
        org,
        None,
        Some(last.created_at),
        Some(last.id),
        2,
    )
    .await
    .expect("second page");

    let all: Vec<String> = first
        .iter()
        .chain(second.iter())
        .map(|r| r.rationale.clone().unwrap_or_default())
        .collect();
    assert_eq!(all.len(), 3, "paging must cover every row exactly once");
    let distinct: std::collections::HashSet<&String> = all.iter().collect();
    assert_eq!(distinct.len(), 3, "no row may appear on both pages");
    assert_eq!(
        second.len(),
        1,
        "exactly one row remains after the first page"
    );
    tx.commit().await.expect("commit read");
}

#[tokio::test]
async fn a_cursorless_listing_stays_scoped_to_one_org() {
    let db = test_db().await;
    let mine = seed_org(&db, 91108, "proposal-mine").await;
    let theirs = seed_org(&db, 91109, "proposal-theirs").await;

    file_proposal(&db, theirs, fixed_uuid(91110), "someone else's row").await;
    file_proposal(&db, mine, fixed_uuid(91111), "my row").await;

    let mut tx = org_tx(&db.app, mine).await;
    let rows = agent_platform::list_proposals(&mut *tx, mine, None, None, None, 50)
        .await
        .expect("cursorless list");

    assert_eq!(rows.len(), 1, "only this org's rows are visible");
    assert_eq!(rows[0].rationale.as_deref(), Some("my row"));
    assert_eq!(rows[0].org_id, mine);
    tx.commit().await.expect("commit read");
}
