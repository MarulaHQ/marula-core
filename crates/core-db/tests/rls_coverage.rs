//! Standalone RLS-coverage gate — CI-enforceable without booting core-api.
//!
//! Purpose: run the same coverage assertion core-api performs at boot
//! (`core_db::assert_rls_coverage`, see main.rs "RLS coverage verified")
//! against a freshly migrated database, so a migration adding a user-data
//! table (org_id column) without an isolation policy fails this test —
//! independent of server startup. A negative probe then proves the gate
//! can actually fail: it creates a policy-less table and expects the
//! assertion to flag it, then cleans up.
//!
//! Non-purpose: behavioral RLS isolation tests (see `db_suite.rs ::
//! rls_blocks_cross_org_and_unscoped_reads`) and policy-content checks.
//! Database selection follows the contract in `tests/common/mod.rs`.
//! One test function by design: the probe must not race the positive
//! check on the shared database.

mod common;

use common::test_db;

#[tokio::test]
async fn rls_coverage_gate_passes_on_migrations_and_detects_policy_less_tables() {
    let db = test_db().await;

    // 1. Positive: the committed migration set is fully covered.
    core_db::assert_rls_coverage(&db.admin)
        .await
        .expect("migration set must leave no user-data table without RLS");

    // 2. Negative: a probe table with org_id but no RLS — exactly the
    //    shape the gate exists to catch — must be flagged by name.
    sqlx::query("CREATE TABLE _rls_probe_no_policy (id uuid PRIMARY KEY, org_id uuid NOT NULL)")
        .execute(&db.admin)
        .await
        .expect("create probe table");

    let result = core_db::assert_rls_coverage(&db.admin).await;
    sqlx::query("DROP TABLE _rls_probe_no_policy")
        .execute(&db.admin)
        .await
        .expect("drop probe table");

    let err = result.expect_err("policy-less probe table must be flagged");
    let msg = err.to_string();
    assert!(
        msg.contains("_rls_probe_no_policy"),
        "gate must name the offending table, got: {msg}"
    );

    // 3. After cleanup the gate is green again (probe left no residue).
    core_db::assert_rls_coverage(&db.admin)
        .await
        .expect("gate must recover after probe cleanup");
}
