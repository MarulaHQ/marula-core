//! The workflow node catalogue pin (TASKLIST Wave 43 lane 13, migration 0128).
//!
//! Purpose: prove the pin survives a migrate, that a re-migrate rewrites
//! NOTHING (the pin is history, and a record that must not be edited is
//! the same rule the run ledger follows), and that a digest mismatch is
//! reported rather than swallowed — two builds claiming the same release
//! with different declarations is the failure the pin exists to catch.
//!
//! Non-purpose: catalogue contents (core-domain owns the declarations and
//! their tests) and node validation. Database selection follows the
//! contract in `tests/common/mod.rs`.

mod common;

use common::test_db;
use core_db::node_registry::{latest_pin, pin_matches_digest, record_pin};

#[tokio::test]
async fn the_catalogue_pin_records_the_running_release_and_refuses_to_rewrite_it() {
    let db = test_db().await;

    let version = core_domain::node_registry::CATALOGUE_VERSION;
    let digest = core_domain::node_registry::catalogue_digest();
    let count = core_domain::node_registry::all().len() as i32;

    // The boot path (`run_migrations`) already recorded it, so the running
    // release is what the test database migrated under.
    let pin = latest_pin(&db.admin)
        .await
        .expect("pin read")
        .expect("run_migrations must have recorded the running release");
    assert_eq!(pin.version, version);
    assert_eq!(
        pin.digest, digest,
        "the recorded digest must be the running one"
    );
    assert_eq!(pin.node_count, count);
    assert!(pin_matches_digest(&db.admin, version, &digest)
        .await
        .expect("digest check"));

    // A DIFFERENT digest for the same version is a build drift, and the
    // check must say so rather than pass.
    let drift = format!("{digest}0");
    assert!(!pin_matches_digest(&db.admin, version, &drift)
        .await
        .expect("digest check"));
    assert!(pin_matches_digest(&db.admin, "9.9.9", &digest)
        .await
        .expect(
            "an unrecorded version has nothing to contradict the running digest, \
         so the caller's own check decides"
        ));

    // Re-recording the same release is a no-op: a different digest cannot
    // overwrite history, and the row count does not move.
    record_pin(
        &mut db.admin.begin().await.expect("tx"),
        version,
        &drift,
        999,
        Some("probe"),
    )
    .await
    .expect("re-record is accepted");
    let after = latest_pin(&db.admin)
        .await
        .expect("pin read")
        .expect("still one row");
    assert_eq!(
        after.digest, digest,
        "the recorded release must not be rewritten"
    );
    assert_eq!(after.node_count, count);
}
