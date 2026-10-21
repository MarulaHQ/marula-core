//! Capacity watch: the page-1 plan read, against a real planner.
//!
//! Purpose: the plan signal (`marula.db.page1_sort_index_scan`) is only worth
//! having if the two halves it stands on work against Postgres and not just
//! against a fixture. This suite pins both:
//!
//! 1. the page-1 statement, with its binds filled in by
//!    `capacity::page1_explain_sql`, is valid SQL — `ops.explain_plan` returns a
//!    plan rather than a syntax error, which is what a bind the substitution
//!    missed would produce;
//! 2. `capacity::classify_plan` reads that plan's REAL JSON. The unit tests in
//!    `core-db::capacity` use hand-written plan values, and a hand-written value
//!    is exactly what cannot catch a key rename in a Postgres release.
//!
//! What it deliberately does not assert: which plan the planner picks. An empty
//! `tickets` table has no ordered path to choose, so `index_scan` is `false` —
//! the F4 baseline — and asserting the scan node under the sort would pin a
//! planner version instead of asserting that a real plan was read.
//!
//! Slot discipline: this suite owns fixed-UUID slots 91500–91549 only.
//!
//! Non-purpose: the p95 arm (`capacity::page1_p95`) needs a populated page and
//! forty timed runs. The perf harness owns that measurement, and this suite does
//! not duplicate it.

mod common;

use common::{fixed_uuid, test_db};
use core_db::capacity;

/// The page-1 statement explains and classifies.
///
/// Read through the admin pool, which is the role the worker's sampler runs as,
/// so the plan is what that caller actually sees.
#[tokio::test]
async fn the_page1_statement_explains_and_classifies() {
    let db = test_db().await;
    let org = fixed_uuid(91_500);

    let mut tx = db.admin.begin().await.expect("begin");
    core_db::set_org(&mut tx, org).await.expect("set org");
    core_db::orgs::insert_org(&mut *tx, org, "Capacity Co", "capacity-91500", "CAP")
        .await
        .expect("org insert");
    tx.commit().await.expect("commit");

    let plan = capacity::page1_plan(&db.admin, org)
        .await
        .expect("the plan read must not error")
        .expect("ops.explain_plan must return a plan for the page-1 statement");

    assert_eq!(
        plan.chain.first().map(String::as_str),
        Some("Limit"),
        "the walk must read the outermost node of a real plan, got {:?}",
        plan.chain
    );
    assert!(
        !plan.index_scan,
        "an empty table has no ordered index path; the F4 baseline is a materialized sort (chain {:?})",
        plan.chain
    );
}
