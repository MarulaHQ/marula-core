//! RAG and multimodal vector namespace DB tests (`rag_embeddings` and
//! `rag_deletion_receipts`, migration 0134; ADR 0020; Wave 45 lane 1).
//!
//! Slot discipline: this suite uses fixed-UUID slots 94500–94599 only.

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::{kb, orgs, rag, users};
use core_domain::entities::KbArticleStatus;
use core_domain::rag::{PreparedRagChunk, RagModality};
use core_domain::SEARCH_EMBEDDING_DIM;
use serde_json::json;
use uuid::Uuid;

async fn seed_org(db: &common::TestDb, n: u64) -> Uuid {
    let org = fixed_uuid(n);
    let mut tx = org_tx(&db.app, org).await;
    orgs::insert_org(
        &mut *tx,
        org,
        &format!("RAG Org {n}"),
        &format!("rag-org-{n}"),
        "RAG",
    )
    .await
    .expect("org insert");
    orgs::seed_default_roles(&mut tx, org).await.expect("roles");
    orgs::seed_default_board(&mut tx, org).await.expect("board");
    tx.commit().await.expect("commit org");
    org
}

async fn seed_user(db: &common::TestDb, org: Uuid, n: u64) -> Uuid {
    let user = fixed_uuid(n);
    let mut tx = org_tx(&db.app, org).await;
    users::create_user(
        &mut *tx,
        user,
        org,
        &format!("user-{n}@rag.test"),
        "RAG Author",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user insert");
    tx.commit().await.expect("commit user");
    user
}

/// Builds a 768-dim unit-ish directional vector with weight concentrated at
/// `axis_a` and `axis_b`.
fn axis_vector(axis_a: usize, weight_a: f32, axis_b: usize, weight_b: f32) -> Vec<f32> {
    let mut v = vec![0.001_f32; SEARCH_EMBEDDING_DIM];
    v[axis_a % SEARCH_EMBEDDING_DIM] = weight_a;
    v[axis_b % SEARCH_EMBEDDING_DIM] = weight_b;
    v
}

#[tokio::test]
async fn rag_migration_shape_rls_force_and_no_ann_index() {
    let db = test_db().await;

    for table in ["rag_embeddings", "rag_deletion_receipts"] {
        let (rls, force): (bool, bool) = sqlx::query_as(
            "SELECT relrowsecurity, relforcerowsecurity FROM pg_class WHERE relname = $1",
        )
        .bind(table)
        .fetch_one(&db.admin)
        .await
        .expect("pg_class rls check");
        assert!(rls, "{table} must enable RLS");
        assert!(force, "{table} must force RLS");
    }

    let typ: String = sqlx::query_scalar(
        "SELECT format_type(atttypid, atttypmod) FROM pg_attribute \
         WHERE attrelid = 'rag_embeddings'::regclass AND attname = 'embedding'",
    )
    .fetch_one(&db.admin)
    .await
    .expect("embedding column type");
    assert_eq!(typ, "vector(768)");

    // ADR 0020 §1: exact KNN in v1 — no HNSW or IVFFlat ANN index on rag_embeddings.
    let ann_indexes: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_indexes \
         WHERE tablename = 'rag_embeddings' \
           AND (indexdef ILIKE '%hnsw%' OR indexdef ILIKE '%ivfflat%')",
    )
    .fetch_one(&db.admin)
    .await
    .expect("ann index check");
    assert_eq!(
        ann_indexes, 0,
        "ADR 0020 requires exact KNN in v1 (no ANN index)"
    );
}

#[tokio::test]
async fn provenance_idempotency_and_model_upgrade_reembed() {
    let db = test_db().await;
    let org = seed_org(&db, 94501).await;
    let mut tx = org_tx(&db.app, org).await;

    let chunk_v1 = PreparedRagChunk {
        namespace: "kb-main".into(),
        source_kind: "kb_article".into(),
        source_id: "art-100".into(),
        chunk_index: 0,
        modality: RagModality::Text,
        content_text: Some("Configure BGP peering on the core router.".into()),
        metadata: json!({"title": "BGP Peering", "section": "1.1"}),
        model: "nomic-embed-text-v1".into(),
        content_hash: "hash-v1".into(),
        embedding: axis_vector(0, 1.0, 1, 0.5),
    };

    // 1. Initial insert -> inserted = 1.
    let out1 = rag::upsert_chunks(&mut tx, org, std::slice::from_ref(&chunk_v1))
        .await
        .expect("upsert 1");
    assert_eq!(out1.total, 1);
    assert_eq!(out1.inserted, 1);
    assert_eq!(out1.updated, 0);
    assert_eq!(out1.unchanged, 0);

    // 2. Re-ingesting identical (model, content_hash) -> unchanged = 1 (idempotent no-op).
    let out2 = rag::upsert_chunks(&mut tx, org, std::slice::from_ref(&chunk_v1))
        .await
        .expect("upsert 2");
    assert_eq!(out2.inserted, 0);
    assert_eq!(out2.updated, 0);
    assert_eq!(out2.unchanged, 1);

    // 3. Model upgrade with new vector -> updated = 1.
    let chunk_model_upgrade = PreparedRagChunk {
        model: "embeddinggemma-2-768".into(),
        embedding: axis_vector(0, 0.9, 2, 0.8),
        ..chunk_v1.clone()
    };
    let out3 = rag::upsert_chunks(&mut tx, org, std::slice::from_ref(&chunk_model_upgrade))
        .await
        .expect("upsert 3");
    assert_eq!(out3.inserted, 0);
    assert_eq!(out3.updated, 1);
    assert_eq!(out3.unchanged, 0);

    // 4. Content hash change -> updated = 1.
    let chunk_content_change = PreparedRagChunk {
        content_text: Some("Configure BGP peering and BFD timers on the core router.".into()),
        content_hash: "hash-v2".into(),
        embedding: axis_vector(0, 0.95, 2, 0.85),
        ..chunk_model_upgrade
    };
    let out4 = rag::upsert_chunks(&mut tx, org, std::slice::from_ref(&chunk_content_change))
        .await
        .expect("upsert 4");
    assert_eq!(out4.inserted, 0);
    assert_eq!(out4.updated, 1);
    assert_eq!(out4.unchanged, 0);

    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn hybrid_bm25_vector_rrf_and_cross_modal_retrieval() {
    let db = test_db().await;
    let org = seed_org(&db, 94510).await;
    let mut tx = org_tx(&db.app, org).await;

    // Seed 3 chunks in namespace "ops-corpus":
    // - chunk A (text): matches both BM25 ("firewall packet drop") and vector axis 10
    // - chunk B (image): has no content_text (pure media vector on axis 10)
    // - chunk C (audio): has transcript text ("voicemail about printer") and vector on axis 50
    let chunks = vec![
        PreparedRagChunk {
            namespace: "ops-corpus".into(),
            source_kind: "ticket".into(),
            source_id: "t-1".into(),
            chunk_index: 0,
            modality: RagModality::Text,
            content_text: Some("Firewall packet drop on WAN interface eth0".into()),
            metadata: json!({"title": "WAN Firewall Drop", "ticket_number": "MAR-101"}),
            model: "embeddinggemma-2-768".into(),
            content_hash: "h-t1".into(),
            embedding: axis_vector(10, 1.0, 11, 0.8),
        },
        PreparedRagChunk {
            namespace: "ops-corpus".into(),
            source_kind: "attachment".into(),
            source_id: "img-screenshot-1".into(),
            chunk_index: 0,
            modality: RagModality::Image,
            content_text: None,
            metadata: json!({"filename": "wan-drop-screenshot.png", "mime_type": "image/png"}),
            model: "embeddinggemma-2-768".into(),
            content_hash: "h-img1".into(),
            embedding: axis_vector(10, 0.99, 11, 0.82),
        },
        PreparedRagChunk {
            namespace: "ops-corpus".into(),
            source_kind: "attachment".into(),
            source_id: "aud-call-1".into(),
            chunk_index: 0,
            modality: RagModality::Audio,
            content_text: Some("Client voicemail reporting printer paper jam on floor 2".into()),
            metadata: json!({"filename": "voicemail.wav", "duration_secs": 42}),
            model: "embeddinggemma-2-768".into(),
            content_hash: "h-aud1".into(),
            embedding: axis_vector(50, 1.0, 51, 0.7),
        },
    ];

    rag::upsert_chunks(&mut tx, org, &chunks)
        .await
        .expect("seed chunks");

    // 1. Hybrid query (query_text + query_vector on axis 10):
    //    - Chunk A matches BOTH BM25 ("firewall packet drop") and vector axis 10 -> ranks #1.
    //    - Chunk B (image without text) is retrieved cross-modally via the shared 768 vector space -> ranks #2.
    let q_vec = axis_vector(10, 1.0, 11, 0.8);
    let hits = rag::query_hybrid(
        &mut tx,
        rag::RagQueryParams {
            org_id: org,
            namespace: "ops-corpus",
            modalities: &[],
            source_kinds: &[],
            source_ids: &[],
            query_text: Some("firewall packet drop"),
            query_vector: Some(&q_vec),
            limit: 10,
        },
    )
    .await
    .expect("hybrid query");

    assert_eq!(hits.len(), 3);
    assert_eq!(hits[0].source_id, "t-1");
    assert_eq!(hits[0].modality, RagModality::Text);
    assert!(hits[0].lexical_score.is_some());
    assert!(hits[0].vector_distance.is_some());
    assert_eq!(hits[0].metadata["ticket_number"], "MAR-101");

    // Cross-modal hit: text/vector query retrieved the image chunk even though content_text is None.
    assert_eq!(hits[1].source_id, "img-screenshot-1");
    assert_eq!(hits[1].modality, RagModality::Image);
    assert!(hits[1].lexical_score.is_none());
    assert!(hits[1].vector_distance.is_some());

    // 2. Media->Text query (vector-only query on axis 10 filtered to `text` modality):
    let media_to_text = rag::query_hybrid(
        &mut tx,
        rag::RagQueryParams {
            org_id: org,
            namespace: "ops-corpus",
            modalities: &[RagModality::Text],
            source_kinds: &[],
            source_ids: &[],
            query_text: None,
            query_vector: Some(&q_vec),
            limit: 5,
        },
    )
    .await
    .expect("media->text query");
    assert_eq!(media_to_text.len(), 1);
    assert_eq!(media_to_text[0].source_id, "t-1");
    assert_eq!(media_to_text[0].modality, RagModality::Text);

    // 3. Modality filter for Image + Audio only:
    let media_only = rag::query_hybrid(
        &mut tx,
        rag::RagQueryParams {
            org_id: org,
            namespace: "ops-corpus",
            modalities: &[RagModality::Image, RagModality::Audio],
            source_kinds: &[],
            source_ids: &[],
            query_text: None,
            query_vector: Some(&q_vec),
            limit: 5,
        },
    )
    .await
    .expect("media-only filter");
    assert_eq!(media_only.len(), 2);
    assert_eq!(media_only[0].source_id, "img-screenshot-1");

    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn hostile_cross_tenant_rls_and_cross_namespace_isolation() {
    let db = test_db().await;
    let org_a = seed_org(&db, 94520).await;
    let org_b = seed_org(&db, 94521).await;

    let vec_shared = axis_vector(5, 1.0, 6, 0.9);

    // Org A writes chunks into "ns-alpha" and "ns-beta".
    {
        let mut tx_a = org_tx(&db.app, org_a).await;
        let chunks_a = vec![
            PreparedRagChunk {
                namespace: "ns-alpha".into(),
                source_kind: "document".into(),
                source_id: "doc-a1".into(),
                chunk_index: 0,
                modality: RagModality::Text,
                content_text: Some("Secret merger runbook for Org A alpha".into()),
                metadata: json!({"org": "A", "ns": "alpha"}),
                model: "bge-base".into(),
                content_hash: "ha-1".into(),
                embedding: vec_shared.clone(),
            },
            PreparedRagChunk {
                namespace: "ns-beta".into(),
                source_kind: "document".into(),
                source_id: "doc-a2".into(),
                chunk_index: 0,
                modality: RagModality::Text,
                content_text: Some("Secret merger runbook for Org A beta".into()),
                metadata: json!({"org": "A", "ns": "beta"}),
                model: "bge-base".into(),
                content_hash: "ha-2".into(),
                embedding: vec_shared.clone(),
            },
        ];
        rag::upsert_chunks(&mut tx_a, org_a, &chunks_a)
            .await
            .expect("org A upsert");
        tx_a.commit().await.expect("org A commit");
    }

    // 1. Cross-namespace isolation within Org A: querying "ns-alpha" never returns "ns-beta".
    {
        let mut tx_a = org_tx(&db.app, org_a).await;
        let alpha_hits = rag::query_hybrid(
            &mut tx_a,
            rag::RagQueryParams {
                org_id: org_a,
                namespace: "ns-alpha",
                modalities: &[],
                source_kinds: &[],
                source_ids: &[],
                query_text: Some("Secret merger runbook"),
                query_vector: Some(&vec_shared),
                limit: 10,
            },
        )
        .await
        .expect("alpha query");
        assert_eq!(alpha_hits.len(), 1);
        assert_eq!(alpha_hits[0].namespace, "ns-alpha");
        assert_eq!(alpha_hits[0].source_id, "doc-a1");
    }

    // 2. Cross-tenant RLS isolation: Org B querying "ns-alpha" sees 0 rows,
    //    even if a hostile caller passes `org_a` to the query function while
    //    bound to `org_b`'s RLS session.
    {
        let mut tx_b = org_tx(&db.app, org_b).await;
        let b_hits = rag::query_hybrid(
            &mut tx_b,
            rag::RagQueryParams {
                org_id: org_a, // hostile org_id parameter under org_b RLS transaction
                namespace: "ns-alpha",
                modalities: &[],
                source_kinds: &[],
                source_ids: &[],
                query_text: Some("Secret merger runbook"),
                query_vector: Some(&vec_shared),
                limit: 10,
            },
        )
        .await
        .expect("org B query");
        assert!(
            b_hits.is_empty(),
            "RLS must block Org B from reading Org A's chunks even with a forged org_id"
        );

        // Hostile cross-tenant write attempt: Org B trying to insert a row claiming org_a fails RLS WITH CHECK.
        let hostile_write = rag::upsert_chunks(
            &mut tx_b,
            org_a,
            &[PreparedRagChunk {
                namespace: "ns-alpha".into(),
                source_kind: "document".into(),
                source_id: "doc-a1".into(),
                chunk_index: 0,
                modality: RagModality::Text,
                content_text: Some("Poisoned by Org B".into()),
                metadata: json!({}),
                model: "poison-model".into(),
                content_hash: "poison-hash".into(),
                embedding: vec_shared.clone(),
            }],
        )
        .await;
        assert!(
            hostile_write.is_err(),
            "RLS WITH CHECK must refuse cross-tenant insert/overwrite"
        );
    }

    // 3. Unscoped pool default-denies both tables.
    let unscoped = db.fresh_app_pool().await;
    let emb_count: i64 = sqlx::query_scalar("SELECT count(*) FROM rag_embeddings")
        .fetch_one(&unscoped)
        .await
        .expect("unscoped embeddings count");
    assert_eq!(emb_count, 0);
}

#[tokio::test]
async fn deletion_propagation_and_append_only_receipts() {
    let db = test_db().await;
    let org = seed_org(&db, 94530).await;
    let author = seed_user(&db, org, 94531).await;
    let article_id = fixed_uuid(94532);

    let mut tx = org_tx(&db.app, org).await;
    core_db::set_org_and_user(&mut tx, org, author)
        .await
        .expect("set org and user");

    // Create a real KB article and ingest 2 RAG chunks referencing its UUID.
    let article = kb::create_article(
        &mut *tx,
        org,
        article_id,
        None,
        "Firewall Failover SOP",
        "firewall-failover-sop",
        "Step 1: verify HA state.\nStep 2: fail over.",
        None,
        KbArticleStatus::Published,
        false,
        author,
    )
    .await
    .expect("create kb article");

    let chunks = vec![
        PreparedRagChunk {
            namespace: "kb-rag".into(),
            source_kind: "kb_article".into(),
            source_id: article.id.to_string(),
            chunk_index: 0,
            modality: RagModality::Text,
            content_text: Some("Step 1: verify HA state.".into()),
            metadata: json!({"step": 1}),
            model: "bge-base".into(),
            content_hash: "sop-chunk-0".into(),
            embedding: axis_vector(20, 1.0, 21, 0.5),
        },
        PreparedRagChunk {
            namespace: "kb-rag".into(),
            source_kind: "kb_article".into(),
            source_id: article.id.to_string(),
            chunk_index: 1,
            modality: RagModality::Text,
            content_text: Some("Step 2: fail over.".into()),
            metadata: json!({"step": 2}),
            model: "bge-base".into(),
            content_hash: "sop-chunk-1".into(),
            embedding: axis_vector(20, 0.8, 21, 0.9),
        },
    ];
    rag::upsert_chunks(&mut tx, org, &chunks)
        .await
        .expect("ingest kb chunks");

    // Delete the underlying KB article -> AFTER DELETE trigger cascades to
    // `rag_embeddings` and mints a `rag_deletion_receipts` row automatically.
    let deleted = kb::delete_article(&mut *tx, org, article.id)
        .await
        .expect("delete kb article");
    assert!(deleted);

    let remaining_chunks: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM rag_embeddings WHERE org_id = $1 AND source_id = $2",
    )
    .bind(org)
    .bind(article.id.to_string())
    .fetch_one(&mut *tx)
    .await
    .expect("remaining chunks");
    assert_eq!(
        remaining_chunks, 0,
        "cascade trigger must remove all source chunks"
    );

    let receipts = rag::list_deletion_receipts(
        &mut *tx,
        org,
        Some("kb-rag"),
        Some("kb_article"),
        Some(&article.id.to_string()),
        10,
    )
    .await
    .expect("list receipts");
    assert_eq!(
        receipts.len(),
        1,
        "cascade delete must mint a deletion receipt"
    );
    assert_eq!(receipts[0].deleted_chunks, 2);
    assert_eq!(
        receipts[0].content_hashes,
        vec!["sop-chunk-0".to_string(), "sop-chunk-1".to_string()]
    );
    assert_eq!(receipts[0].deleted_by, Some(author));

    // Receipts are append-only: attempting to DELETE or UPDATE a receipt row fails.
    let receipt_id = receipts[0].receipt_id;
    let tamper = sqlx::query("DELETE FROM rag_deletion_receipts WHERE receipt_id = $1")
        .bind(receipt_id)
        .execute(&mut *tx)
        .await;
    assert!(
        tamper.is_err(),
        "rag_deletion_receipts append-only trigger must reject DELETE"
    );
}
