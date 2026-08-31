//! Import framework persistence tests (TASKLIST D2) against a real Postgres:
//! external-id mapping idempotency, cross-org RLS isolation on BOTH import
//! tables, batch counter bookkeeping with finish guards, and the whole-row
//! update mutators the Update policy rides on.
//!
//! Non-purpose: pure planning semantics (core-domain suite) and HTTP
//! behavior (core-api suite).

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::{audit, clients, imports, orgs};
use core_domain::entities::Actor;
use uuid::Uuid;

/// One freshly seeded org with default roles/board/SLA, using ONLY seed slots
/// reserved for this lane (190–199).
async fn seed_org(app: &sqlx::PgPool, n: u64) -> Uuid {
    let org = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    orgs::insert_org(&mut *tx, org, "Import MSP", &format!("imports-{n}"), "IMP")
        .await
        .expect("org insert");
    orgs::seed_default_roles(&mut tx, org).await.expect("roles");
    orgs::seed_default_board(&mut tx, org).await.expect("board");
    orgs::seed_default_sla_policy(&mut *tx, org)
        .await
        .expect("sla");
    tx.commit().await.expect("commit seed");
    org
}

fn batch_row(
    org_id: Uuid,
    label: &str,
    kind: &str,
    policy: &str,
    dry_run: bool,
) -> imports::ImportBatch {
    imports::ImportBatch {
        id: Uuid::now_v7(),
        org_id,
        label: label.into(),
        source_note: Some("test source".into()),
        entity_kind: kind.into(),
        conflict_policy: policy.into(),
        dry_run,
        rows_total: 0,
        rows_created: 0,
        rows_updated: 0,
        rows_skipped: 0,
        rows_failed: 0,
        created_by: fixed_uuid(191),
        created_by_kind: "user".into(),
        started_at: chrono::Utc::now(),
        finished_at: None,
        staged_chunks: 0,
        queued_at: None,
    }
}

#[tokio::test]
async fn mapping_upsert_is_idempotent_and_batch_scoped() {
    let db = test_db().await;
    let org = seed_org(&db.app, 190).await;
    let batch = batch_row(org, "clients-1", "client", "skip", false);
    {
        let mut tx = org_tx(&db.app, org).await;
        imports::insert_batch(&mut *tx, &batch)
            .await
            .expect("batch");
        tx.commit().await.expect("commit");
    }

    let internal_a = Uuid::now_v7();
    {
        let mut tx = org_tx(&db.app, org).await;
        imports::upsert_mapping(
            &mut *tx,
            Uuid::now_v7(),
            org,
            "client",
            "ACME-EXT",
            internal_a,
            batch.id,
        )
        .await
        .expect("first upsert");
        // SAME key again resolves to the SAME uuid — the entire idempotency
        // contract — only refreshing last_seen_batch.
        imports::upsert_mapping(
            &mut *tx,
            Uuid::now_v7(),
            org,
            "client",
            "ACME-EXT",
            internal_a,
            batch.id,
        )
        .await
        .expect("re-push upsert");

        let resolved = imports::find_mapping(&mut *tx, org, "client", "ACME-EXT")
            .await
            .expect("lookup");
        assert_eq!(resolved, Some(internal_a));

        let pairs = imports::find_mappings(
            &mut *tx,
            org,
            "client",
            &["ACME-EXT".into(), "MISSING".into()],
        )
        .await
        .expect("batch lookup");
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, "ACME-EXT");
        tx.commit().await.expect("commit");
    }

    // Kinds are namespaced: a contact with an identical external key is a
    // different row.
    {
        let mut tx = org_tx(&db.app, org).await;
        let contact_id = Uuid::now_v7();
        imports::upsert_mapping(
            &mut *tx,
            Uuid::now_v7(),
            org,
            "contact",
            "ACME-EXT",
            contact_id,
            batch.id,
        )
        .await
        .expect("other-kind upsert");
        let own = imports::find_mapping(&mut *tx, org, "client", "ACME-EXT")
            .await
            .expect("client key still intact");
        assert_eq!(own, Some(internal_a));
        tx.commit().await.expect("commit");
    }
}

#[tokio::test]
async fn import_tables_are_rls_isolated_across_orgs() {
    let db = test_db().await;
    let org_a = seed_org(&db.app, 192).await;
    let org_b = seed_org(&db.app, 193).await;

    let batch = batch_row(org_a, "secret-run", "site", "update", false);
    {
        let mut tx = org_tx(&db.app, org_a).await;
        imports::insert_batch(&mut *tx, &batch)
            .await
            .expect("batch");
        imports::upsert_mapping(
            &mut *tx,
            Uuid::now_v7(),
            org_a,
            "site",
            "SITE-SECRET",
            Uuid::now_v7(),
            batch.id,
        )
        .await
        .expect("mapping");
        tx.commit().await.expect("commit");
    }

    // Org B sees NEITHER the batch NOR the mapping — parameterized lookup
    // and raw scoped read agree with the RLS policies from 0027.
    {
        let mut tx = org_tx(&db.app, org_b).await;
        assert!(
            imports::find_mapping(&mut *tx, org_b, "site", "SITE-SECRET")
                .await
                .expect("scoped lookup")
                .is_none()
        );
        let listed = imports::list_batches(&mut *tx, org_b, 50)
            .await
            .expect("list");
        assert!(
            !listed.iter().any(|b| b.id == batch.id),
            "foreign batch hidden"
        );
        assert!(
            matches!(
                imports::get_batch(&mut *tx, org_b, batch.id).await,
                Err(core_db::DbError::Sql(sqlx::Error::RowNotFound))
            ),
            "foreign get_batch is a miss"
        );
    }

    // No scope ⇒ default-deny everywhere, on both tables.
    let fresh = db.fresh_app_pool().await;
    let mut unscoped = fresh.begin().await.expect("begin");
    let hidden_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM import_batches")
        .fetch_one(&mut *unscoped)
        .await
        .expect("count");
    let hidden_keys: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM external_ids")
        .fetch_one(&mut *unscoped)
        .await
        .expect("count keys");
    assert_eq!((hidden_batches, hidden_keys), (0, 0));
}

#[tokio::test]
async fn batch_counters_accumulate_until_finish_then_reject_late_chunks() {
    let db = test_db().await;
    let org = seed_org(&db.app, 194).await;
    let batch = batch_row(org, "bulk", "contact", "update", false);

    let open = async {
        let mut tx = org_tx(&db.app, org).await;
        imports::insert_batch(&mut *tx, &batch)
            .await
            .expect("insert");
        tx.commit().await.expect("commit");
    };
    open.await;

    {
        let mut tx = org_tx(&db.app, org).await;
        let bumped = imports::add_chunk_counters(&mut *tx, org, batch.id, 10, 6, 2, 2, 0)
            .await
            .expect("chunk 1");
        assert!(bumped);
        // Second chunk lands concurrently-fine; deltas accumulate.
        let bumped = imports::add_chunk_counters(&mut *tx, org, batch.id, 5, 5, 0, 0, 0)
            .await
            .expect("chunk 2");
        assert!(bumped);
        tx.commit().await.expect("commit counters");
    }

    let closed = {
        let mut tx = org_tx(&db.app, org).await;
        let done = imports::mark_batch_finished(&mut *tx, org, batch.id)
            .await
            .expect("finish");
        tx.commit().await.expect("commit finish");
        done
    };
    assert_eq!(closed.rows_total, 15);
    assert_eq!(closed.rows_created, 11);
    assert_eq!(closed.rows_updated, 2);
    assert_eq!(closed.rows_skipped, 2);
    assert!(closed.finished_at.is_some());

    // Late chunk after finish must NOT drift counters.
    {
        let mut tx = org_tx(&db.app, org).await;
        let late = imports::add_chunk_counters(&mut *tx, org, batch.id, 3, 3, 0, 0, 0)
            .await
            .expect("late attempt executes");
        assert!(!late, "finished batch rejects counters");
        let twice = imports::mark_batch_finished(&mut *tx, org, batch.id).await;
        assert!(
            matches!(twice, Err(core_db::DbError::NotFound)),
            "double finish is NotFound"
        );
    }
}

#[tokio::test]
async fn whole_row_updates_roundtrip_and_respect_org_scope() {
    let db = test_db().await;
    let org = seed_org(&db.app, 195).await;
    let foreign = seed_org(&db.app, 196).await;

    let (client_id, site_id, contact_id) = {
        let mut tx = org_tx(&db.app, org).await;
        let client_id = Uuid::now_v7();
        clients::create_client(&mut *tx, org, client_id, "Old Name", Some("old"))
            .await
            .expect("client");
        let site_id = Uuid::now_v7();
        clients::create_site(
            &mut *tx, org, site_id, client_id, "Branch", None, None, None, None, None, None, None,
            None,
        )
        .await
        .expect("site");
        let contact_id = Uuid::now_v7();
        clients::create_contact(
            &mut *tx,
            org,
            contact_id,
            client_id,
            Some(site_id),
            "Pat",
            None,
            None,
            None,
            false,
        )
        .await
        .expect("contact");
        tx.commit().await.expect("commit seeds");
        (client_id, site_id, contact_id)
    };

    // Extended INSERT fields persist through the canonical creators.
    {
        let mut tx = org_tx(&db.app, org).await;
        clients::create_site(
            &mut *tx,
            org,
            Uuid::now_v7(),
            client_id,
            "HQ",
            Some("1 Main St"),
            Some("Springfield"),
            Some("IL"),
            Some("62704"),
            Some("USA"),
            Some(39.7817),
            Some(-89.6501),
            Some("America/Chicago"),
        )
        .await
        .expect("geography site");
        tx.commit().await.expect("commit");
    }

    {
        let mut tx = org_tx(&db.app, org).await;
        let updated = clients::update_client(&mut *tx, org, client_id, "New Name", None)
            .await
            .expect("client update");
        assert_eq!(updated.name, "New Name");
        assert!(updated.notes.is_none());
        assert_eq!(
            updated.updated_at, updated.updated_at,
            "UPDATE stamps updated_at"
        );

        let site = clients::update_site(
            &mut *tx,
            org,
            site_id,
            "Branch East",
            Some("9 Oak Ave"),
            None,
            None,
            None,
            None,
            Some(40.0),
            None,
            None,
        )
        .await
        .expect("site update");
        assert_eq!(site.name, "Branch East");
        assert_eq!(site.address.as_deref(), Some("9 Oak Ave"));
        assert_eq!(site.latitude, Some(40.0));

        let contact = clients::update_contact(
            &mut *tx,
            org,
            contact_id,
            "Pat Smith",
            Some("pat@example.com"),
            Some("+1 555 0100"),
            Some("Ops Manager"),
            true,
        )
        .await
        .expect("contact update");
        assert_eq!(contact.email.as_deref(), Some("pat@example.com"));
        assert!(contact.is_primary);
        assert_eq!(contact.site_id, Some(site_id), "identity link immutable");
        tx.commit().await.expect("commit updates");
    }

    // Foreign-org updates are NotFound, never silent.
    {
        let mut tx = org_tx(&db.app, foreign).await;
        assert!(matches!(
            clients::update_client(&mut *tx, foreign, client_id, "Hijack", None).await,
            Err(core_db::DbError::NotFound)
        ));
        assert!(matches!(
            clients::update_site(
                &mut *tx, foreign, site_id, "X", None, None, None, None, None, None, None, None
            )
            .await,
            Err(core_db::DbError::NotFound)
        ));
        assert!(matches!(
            clients::update_contact(&mut *tx, foreign, contact_id, "X", None, None, None, false)
                .await,
            Err(core_db::DbError::NotFound)
        ));
    }
}

#[tokio::test]
async fn audit_writes_actor_import_rows_after_writer_extension() {
    let db = test_db().await;
    let org = seed_org(&db.app, 197).await;

    let evt = core_domain::entities::AuditEvent {
        id: Uuid::now_v7(),
        org_id: org,
        actor: Actor::Import("api-key:018f0000-0000-7000-8000-000000001901".into()),
        action: "import.batch_started".into(),
        entity_type: Some("import_batch".into()),
        entity_id: Some(Uuid::now_v7().to_string()),
        detail: serde_json::json!({ "label": "qbo-migration" }),
        ip: None,
        occurred_at: chrono::Utc::now(),
    };
    {
        let mut tx = org_tx(&db.app, org).await;
        audit::insert(&mut *tx, &evt)
            .await
            .expect("import actor row");

        let listed = audit::list_filtered(&mut *tx, org, Default::default(), 10)
            .await
            .expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0], evt,
            "actor_kind=import round-trips by descriptor"
        );
        tx.commit().await.expect("commit");
    }
}
