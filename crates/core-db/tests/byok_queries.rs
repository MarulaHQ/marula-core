//! BYOK sealed-secret persistence tests (migration 0044, adapters lane).
//!
//! Covers the promises the API layer leans on: the stable entity id across
//! upserts (write-trail + idempotency anchoring), presence-only summaries
//! (zero secret bytes), owner-role pool reads (the API's `state.pool`
//! profile) and RLS default-deny inside `marula_app` transactions.

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::byok;
use sqlx::PgPool;

async fn seed_org(app: &PgPool, n: u64) -> uuid::Uuid {
    let org = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    core_db::orgs::insert_org(&mut *tx, org, "Acme MSP", &format!("byok-{n}"), "BOK")
        .await
        .expect("org insert");
    tx.commit().await.expect("commit seed");
    org
}

#[tokio::test]
async fn upsert_keeps_stable_row_id_and_reports_insertion() {
    let db = test_db().await;
    let org = seed_org(&db.app, 441).await;

    let first = fixed_uuid(4411);
    {
        let mut tx = org_tx(&db.app, org).await;
        let (id, inserted) = byok::upsert_secret(&mut tx, first, org, "openai", b"ct-1", None)
            .await
            .expect("first upsert");
        assert!(inserted);
        assert_eq!(id, first);
        tx.commit().await.unwrap();
    }

    let second = fixed_uuid(4412);
    {
        let mut tx = org_tx(&db.app, org).await;
        let (id, inserted) = byok::upsert_secret(&mut tx, second, org, "openai", b"ct-2", None)
            .await
            .expect("second upsert");
        assert!(!inserted, "update must keep the original entity id");
        assert_eq!(id, first);
        tx.commit().await.unwrap();
    }

    let meta = {
        let mut tx = org_tx(&db.app, org).await;
        byok::get_secret_meta(&mut tx, org, "openai")
            .await
            .expect("meta read")
            .expect("row exists")
    };
    assert_eq!(meta.0, first);
}

#[tokio::test]
async fn summaries_stay_presence_only_and_scope_cleansely() {
    let db = test_db().await;
    let org_a = seed_org(&db.app, 442).await;
    let org_b = seed_org(&db.app, 443).await;

    for (org, provider) in [(org_a, "openai"), (org_a, "qbo"), (org_b, "xero")] {
        let mut tx = org_tx(&db.app, org).await;
        byok::upsert_secret(&mut tx, uuid::Uuid::now_v7(), org, provider, b"ct", None)
            .await
            .expect("upsert");
        tx.commit().await.unwrap();
    }

    // Pool profile = production `state.pool` (owner role, explicit org
    // predicates). Summaries never carry secret bytes.
    let summaries_a = byok::list_secret_summaries(&db.admin, org_a)
        .await
        .expect("list");
    assert_eq!(
        summaries_a
            .iter()
            .map(|s| s.provider.as_str())
            .collect::<Vec<_>>(),
        vec!["openai", "qbo"]
    );

    // Ciphertext round-trips for provider execution.
    let ct = byok::get_secret(&db.admin, org_a, "qbo")
        .await
        .expect("get");
    assert_eq!(ct.as_deref(), Some(b"ct".as_slice()));

    // From the RLS-constrained app role with a never-scoped session, the
    // same listing default-denies (sees zero rows).
    let unscoped_pool = db.fresh_app_pool().await;
    let unscoped = byok::list_secret_summaries(&unscoped_pool, org_a)
        .await
        .expect("list");
    assert!(unscoped.is_empty(), "no GUC ⇒ default-deny");
}

#[tokio::test]
async fn delete_removes_only_the_target_provider() {
    let db = test_db().await;
    let org = seed_org(&db.app, 444).await;

    for provider in ["openai", "xero"] {
        let mut tx = org_tx(&db.app, org).await;
        byok::upsert_secret(&mut tx, uuid::Uuid::now_v7(), org, provider, b"ct", None)
            .await
            .expect("upsert");
        tx.commit().await.unwrap();
    }

    {
        let mut tx = org_tx(&db.app, org).await;
        byok::delete_secret(&mut tx, org, "openai")
            .await
            .expect("delete");
        tx.commit().await.unwrap();
    }

    let remaining = byok::list_secret_summaries(&db.admin, org)
        .await
        .expect("list");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].provider, "xero");

    let meta = {
        let mut tx = org_tx(&db.app, org).await;
        byok::get_secret_meta(&mut tx, org, "openai")
            .await
            .expect("meta")
    };
    assert!(meta.is_none());
}
