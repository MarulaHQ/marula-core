//! D2-remainder persistence coverage (TASKLIST D2): saved field-mapping
//! templates and the async giant-batch bookkeeping (staged chunks + the
//! once-only worker-enqueue stamp) against a real Postgres.
//!
//! Purpose: prove at the query layer that
//! 1. Template CRUD round-trips and the (org, source_system, entity_kind,
//!    name) uniqueness holds; cross-org RLS keeps templates isolated.
//! 2. `staged_chunks` only moves while a batch is open and unqueued —
//!    staging closes the moment a batch is enqueued or finished.
//! 3. `queued_at` stamps exactly once; the deterministic chunk-key scheme
//!    covers the staged sequence.
//! 4. The widened entity-kind vocabulary (0033) accepts board/ticket/
//!    time_entry batches.
//!
//! Non-purpose: engine planning semantics (core-domain suite) and the HTTP
//! + worker flow (core-api suite).
//!
//! Org rows use fixed UUIDs 180–186 (the import/export lane band).

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::imports::{self, ImportBatch, ImportMappingTemplate};
use uuid::Uuid;

/// One freshly seeded org, lane slot `n`.
async fn seed_org(app: &sqlx::PgPool, n: u64) -> Uuid {
    let org = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    core_db::orgs::insert_org(&mut *tx, org, "D2 MSP", &format!("d2-{n}"), "DTW")
        .await
        .expect("org insert");
    core_db::orgs::seed_default_roles(&mut tx, org)
        .await
        .expect("roles");
    core_db::orgs::seed_default_board(&mut tx, org)
        .await
        .expect("board");
    core_db::orgs::seed_default_sla_policy(&mut *tx, org)
        .await
        .expect("sla");
    tx.commit().await.expect("commit seed");
    org
}

fn template(org: Uuid, name: &str, source: &str, kind: &str) -> ImportMappingTemplate {
    ImportMappingTemplate {
        id: Uuid::now_v7(),
        org_id: org,
        name: name.into(),
        source_system: source.into(),
        entity_kind: kind.into(),
        mapping: serde_json::json!({ "Company": "name", "ID": "external_key" }),
        created_by: fixed_uuid(199),
        created_by_kind: "user".into(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

fn batch_row(org_id: Uuid, kind: &str, dry_run: bool) -> ImportBatch {
    ImportBatch {
        id: Uuid::now_v7(),
        org_id,
        label: "async".into(),
        source_note: Some("test source".into()),
        entity_kind: kind.into(),
        conflict_policy: "skip".into(),
        dry_run,
        rows_total: 0,
        rows_created: 0,
        rows_updated: 0,
        rows_skipped: 0,
        rows_failed: 0,
        created_by: fixed_uuid(199),
        created_by_kind: "user".into(),
        started_at: chrono::Utc::now(),
        finished_at: None,
        staged_chunks: 0,
        queued_at: None,
    }
}

#[tokio::test]
async fn template_crud_round_trips_and_uniqueness_holds() {
    let db = test_db().await;
    let org = seed_org(&db.app, 180).await;

    let record = template(org, "ConnectWise clients", "connectwise", "client");
    {
        let mut tx = org_tx(&db.app, org).await;
        imports::insert_template(&mut *tx, &record)
            .await
            .expect("insert");
        let fetched = imports::get_template(&mut *tx, org, record.id)
            .await
            .expect("fetch");
        assert_eq!(fetched.name, "ConnectWise clients");
        assert_eq!(fetched.entity_kind, "client");
        assert_eq!(fetched.mapping["Company"], "name");

        let listed = imports::list_templates(&mut *tx, org, Some("connectwise"), 50)
            .await
            .expect("list");
        assert_eq!(listed.len(), 1);
        let none = imports::list_templates(&mut *tx, org, Some("hansworld"), 50)
            .await
            .expect("list filtered");
        assert!(none.is_empty());

        let updated = imports::update_template(
            &mut *tx,
            org,
            record.id,
            "CW clients v2",
            "connectwise",
            &serde_json::json!({ "Company": "name" }),
        )
        .await
        .expect("update");
        assert_eq!(updated.name, "CW clients v2");
        tx.commit().await.expect("commit");
    }

    // Same (org, source, kind, name) is a uniqueness conflict.
    let duplicate = template(org, "CW clients v2", "connectwise", "client");
    let conflict = {
        let mut tx = org_tx(&db.app, org).await;
        let result = imports::insert_template(&mut *tx, &duplicate).await;
        tx.commit().await.expect("commit");
        result
    };
    assert!(
        matches!(
            conflict,
            Err(core_db::DbError::Sql(sqlx::Error::Database(_)))
        ),
        "duplicate template must hit the unique index, got {conflict:?}"
    );

    // Delete removes it; the row is then gone (org-scoped).
    {
        let mut tx = org_tx(&db.app, org).await;
        imports::delete_template(&mut *tx, org, record.id)
            .await
            .expect("delete");
        assert!(imports::get_template(&mut *tx, org, record.id)
            .await
            .is_err());
        tx.commit().await.expect("commit");
    }
}

#[tokio::test]
async fn templates_are_rls_isolated_across_orgs() {
    let db = test_db().await;
    let org_a = seed_org(&db.app, 181).await;
    let org_b = seed_org(&db.app, 182).await;

    let record = template(org_a, "Shared name", "connectwise", "client");
    {
        let mut tx = org_tx(&db.app, org_a).await;
        imports::insert_template(&mut *tx, &record)
            .await
            .expect("insert");
        tx.commit().await.expect("commit");
    }

    // Org B cannot see or delete A's template; the same name is FREE in B.
    {
        let mut tx = org_tx(&db.app, org_b).await;
        assert!(imports::get_template(&mut *tx, org_b, record.id)
            .await
            .is_err());
        let same_name = template(org_b, "Shared name", "connectwise", "client");
        imports::insert_template(&mut *tx, &same_name)
            .await
            .expect("same name in org B");
        tx.commit().await.expect("commit");
    }
}

#[tokio::test]
async fn staged_chunks_move_only_while_open_and_unqueued() {
    let db = test_db().await;
    let org = seed_org(&db.app, 183).await;
    let batch = batch_row(org, "client", false);
    {
        let mut tx = org_tx(&db.app, org).await;
        imports::insert_batch(&mut *tx, &batch)
            .await
            .expect("batch");

        assert_eq!(
            imports::bump_staged_chunks(&mut *tx, org, batch.id)
                .await
                .expect("bump"),
            Some(1)
        );
        assert_eq!(
            imports::bump_staged_chunks(&mut *tx, org, batch.id)
                .await
                .expect("bump"),
            Some(2)
        );

        // Deterministic chunk keys cover the staged sequence.
        assert_eq!(
            imports::staged_chunk_key(org, batch.id, 0),
            format!("org/{org}/import-batches/{}/chunk-00000", batch.id)
        );
        assert_eq!(
            imports::staged_chunk_key(org, batch.id, 7),
            format!("org/{org}/import-batches/{}/chunk-00007", batch.id)
        );

        // Exactly-once enqueue.
        assert!(imports::mark_batch_queued(&mut *tx, org, batch.id)
            .await
            .expect("queue"));
        assert!(
            !imports::mark_batch_queued(&mut *tx, org, batch.id)
                .await
                .expect("re-queue"),
            "second enqueue must be rejected"
        );
        // Staging is closed the moment the batch is queued.
        assert_eq!(
            imports::bump_staged_chunks(&mut *tx, org, batch.id)
                .await
                .expect("bump"),
            None
        );
        tx.commit().await.expect("commit");
    }

    // Queued state is durable across transactions (read inside org scope —
    // the app pool is RLS-constrained and refuses unscoped reads).
    let fetched = {
        let mut tx = org_tx(&db.app, org).await;
        let fetched = imports::get_batch(&mut *tx, org, batch.id)
            .await
            .expect("fetch");
        tx.commit().await.expect("commit");
        fetched
    };
    assert_eq!(fetched.staged_chunks, 2);
    assert!(fetched.queued_at.is_some());

    // A finished batch also refuses staging.
    {
        let mut tx = org_tx(&db.app, org).await;
        imports::mark_batch_finished(&mut *tx, org, batch.id)
            .await
            .expect("finish");
        assert_eq!(
            imports::bump_staged_chunks(&mut *tx, org, batch.id)
                .await
                .expect("bump"),
            None
        );
        tx.commit().await.expect("commit");
    }
}

#[tokio::test]
async fn widened_entity_kinds_are_accepted_end_to_end() {
    let db = test_db().await;
    let org = seed_org(&db.app, 184).await;

    for kind in ["board", "ticket", "time_entry"] {
        let batch = batch_row(org, kind, false);
        let mut tx = org_tx(&db.app, org).await;
        imports::insert_batch(&mut *tx, &batch)
            .await
            .expect("batch");
        imports::upsert_mapping(
            &mut *tx,
            Uuid::now_v7(),
            org,
            kind,
            "KEY-1",
            Uuid::now_v7(),
            batch.id,
        )
        .await
        .expect("mapping for widened kind");
        let fetched = imports::get_batch(&mut *tx, org, batch.id)
            .await
            .expect("fetch");
        assert_eq!(fetched.entity_kind, kind);
        tx.commit().await.expect("commit");
    }

    // Dry-run batches record dry_run and stay staging-ineligible too.
    let probe = batch_row(org, "board", true);
    let mut tx = org_tx(&db.app, org).await;
    imports::insert_batch(&mut *tx, &probe)
        .await
        .expect("batch");
    imports::mark_batch_queued(&mut *tx, org, probe.id)
        .await
        .expect("queue");
    let fetched = imports::get_batch(&mut *tx, org, probe.id)
        .await
        .expect("fetch");
    assert_eq!(fetched.staged_chunks, 0);
    tx.commit().await.expect("commit");
}
