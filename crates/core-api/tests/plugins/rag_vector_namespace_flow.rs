//! End-to-end HTTP integration tests for the RAG and multimodal vector
//! namespace extension surface (`/api/v1/rag/...`, ADR 0020; Wave 45 lane 1,
//! migration 0134).
//!
//! Covers:
//! 1. Wrong-width vector refusal (0, 512, 1024 dims) with a typed `422`
//!    (`validation_failed`) and zero rows written.
//! 2. Media modality (`image | video | audio`) without a precomputed vector
//!    refused with a typed `422` (Core never decodes media).
//! 3. Provenance idempotency (`inserted` -> `unchanged` on replay -> `updated`
//!    on model upgrade) plus IF1–IF3 `idempotency_key` replay parity.
//! 4. Hybrid BM25 + exact-KNN cosine distance RRF fusion and cross-modal
//!    retrieval within a shared 768-dim namespace (text query retrieving
//!    `image`/`audio` chunks, and precomputed media `query_vector` retrieving
//!    `text` chunks) with mandatory source attribution.
//! 5. Hostile cross-tenant RLS isolation and cross-namespace isolation
//!    (Org A cannot read, overwrite, or delete Org B's chunks or receipts;
//!    queries in namespace X never return chunks from namespace Y).
//! 6. Source deletion propagation + verifiable deletion receipts via both
//!    `DELETE /api/v1/rag/sources/{kind}/{id}` and `POST /api/v1/rag/delete`,
//!    plus `GET /api/v1/rag/deletion-receipts/{id}`, and machine API-key access
//!    with `rag:read` / `rag:write` scopes.

use crate::common::{auth as bearer, body_json, send_headers as send, state_with};
use axum::Router;
use core_api::state::AppState;
use core_domain::SEARCH_EMBEDDING_DIM;
use serde_json::json;
use uuid::Uuid;

async fn test_state() -> AppState {
    state_with(|config| config.secret_key = core_api::config::SecretKey([45; 32])).await
}

fn api_key_header(key: &str) -> (&'static str, String) {
    ("x-api-key", key.to_string())
}

async fn register_org(app: &Router, state: &AppState, slug: &str) -> (String, Uuid) {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/auth/register",
        &[],
        Some(json!({
            "org_name": format!("RAG Co {slug}"),
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let token = body_json(response).await["access_token"]
        .as_str()
        .expect("access token")
        .to_string();
    let org_id: Uuid = sqlx::query_scalar("SELECT id FROM orgs WHERE slug = $1")
        .bind(slug)
        .fetch_one(&state.pool)
        .await
        .expect("org id");
    (token, org_id)
}

fn axis_vector(axis_a: usize, weight_a: f32, axis_b: usize, weight_b: f32) -> Vec<f32> {
    let mut v = vec![0.001_f32; SEARCH_EMBEDDING_DIM];
    v[axis_a % SEARCH_EMBEDDING_DIM] = weight_a;
    v[axis_b % SEARCH_EMBEDDING_DIM] = weight_b;
    v
}

#[tokio::test]
async fn wrong_width_vectors_and_missing_media_vectors_refused_and_zero_rows() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("rag-width-{}", Uuid::now_v7().simple());
    let (token, org_id) = register_org(&app, &state, &slug).await;

    for bad_dim in [0_usize, 512, 1024] {
        let bad_vec = vec![0.25_f32; bad_dim];
        let res = send(
            &app,
            axum::http::Method::POST,
            "/api/v1/rag/chunks",
            &[bearer(&token)],
            Some(json!({
                "namespace": "ns-strict",
                "chunks": [{
                    "source_kind": "kb_article",
                    "source_id": "art-bad",
                    "chunk_index": 0,
                    "modality": "text",
                    "content_text": "bad vector dimension test",
                    "metadata": {"title": "Bad"},
                    "model": "bad-embedder",
                    "embedding": bad_vec
                }]
            })),
        )
        .await;
        assert_eq!(
            res.status(),
            422,
            "{bad_dim}-dim vector on ingest must be refused as a typed validation error"
        );
        let err = body_json(res).await;
        assert_eq!(err["error"]["reason"], "validation_failed");
        assert!(
            err["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains("768"),
            "error message must cite 768 requirement: {err}"
        );

        let q_res = send(
            &app,
            axum::http::Method::POST,
            "/api/v1/rag/query",
            &[bearer(&token)],
            Some(json!({
                "namespace": "ns-strict",
                "query_vector": vec![0.1_f32; bad_dim]
            })),
        )
        .await;
        assert_eq!(
            q_res.status(),
            422,
            "{bad_dim}-dim query_vector must be refused as a typed validation error"
        );
    }

    // Media chunk without precomputed vector -> 422 validation_failed.
    let no_vec_media = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/rag/chunks",
        &[bearer(&token)],
        Some(json!({
            "namespace": "ns-strict",
            "chunks": [{
                "source_kind": "attachment",
                "source_id": "att-img-1",
                "chunk_index": 0,
                "modality": "image",
                "content_text": "switch rack photo",
                "metadata": {"mime_type": "image/jpeg"}
            }]
        })),
    )
    .await;
    assert_eq!(no_vec_media.status(), 422);

    // Confirm zero rows were written into rag_embeddings for this org.
    let mut tx = state.pool.begin().await.expect("begin");
    core_db::set_org(&mut tx, org_id).await.expect("set org");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM rag_embeddings WHERE org_id = $1")
        .bind(org_id)
        .fetch_one(&mut *tx)
        .await
        .expect("count");
    assert_eq!(count, 0, "rejected batches must write zero rows");
}

#[tokio::test]
async fn hybrid_rrf_cross_modal_provenance_isolation_and_deletion_receipts_flow() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());

    let slug_a = format!("rag-a-{}", Uuid::now_v7().simple());
    let slug_b = format!("rag-b-{}", Uuid::now_v7().simple());
    let (token_a, _org_a) = register_org(&app, &state, &slug_a).await;
    let (token_b, _org_b) = register_org(&app, &state, &slug_b).await;

    let vec_vpn = axis_vector(3, 1.0, 4, 0.85);
    let vec_printer = axis_vector(40, 1.0, 41, 0.75);

    // 1. Ingest a multimodal batch (text + image + audio) into Org A's "msp-kb" namespace.
    let ingest_body = json!({
        "idempotency_key": "idem-rag-batch-1",
        "namespace": "msp-kb",
        "chunks": [
            {
                "source_kind": "kb_article",
                "source_id": "kb-vpn-101",
                "chunk_index": 0,
                "modality": "text",
                "content_text": "Site-to-site IPsec VPN tunnel phase 2 rekey troubleshooting guide",
                "metadata": {"title": "IPsec Rekey Guide", "uri": "/kb/ipsec-rekey"},
                "model": "embeddinggemma-2-768",
                "content_hash": "hash-vpn-text-v1",
                "embedding": vec_vpn
            },
            {
                "source_kind": "attachment",
                "source_id": "att-screenshot-vpn",
                "chunk_index": 0,
                "modality": "image",
                "content_text": null,
                "metadata": {"title": "Firewall Phase 2 Error Screenshot", "mime_type": "image/png"},
                "model": "embeddinggemma-2-768",
                "content_hash": "hash-vpn-img-v1",
                "embedding": axis_vector(3, 0.98, 4, 0.88)
            },
            {
                "source_kind": "attachment",
                "source_id": "att-audio-printer",
                "chunk_index": 0,
                "modality": "audio",
                "content_text": "Voicemail recording from user about fuser error on floor 3 printer",
                "metadata": {"title": "Voicemail #88", "mime_type": "audio/wav", "timecode": "00:00-00:35"},
                "model": "embeddinggemma-2-768",
                "content_hash": "hash-printer-aud-v1",
                "embedding": vec_printer
            }
        ]
    });

    let res1 = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/rag/chunks",
        &[bearer(&token_a)],
        Some(ingest_body.clone()),
    )
    .await;
    assert_eq!(res1.status(), 200);
    let body1 = body_json(res1).await;
    assert_eq!(body1["total"], 3);
    assert_eq!(body1["inserted"], 3);
    assert_eq!(body1["updated"], 0);
    assert_eq!(body1["unchanged"], 0);
    assert_eq!(body1["mutation"]["seq"], 1);

    // Replaying the same idempotency_key returns the byte-identical stored response.
    let replay_res = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/rag/chunks",
        &[bearer(&token_a)],
        Some(ingest_body.clone()),
    )
    .await;
    assert_eq!(replay_res.status(), 200);
    let replay_body = body_json(replay_res).await;
    assert_eq!(replay_body, body1);

    // Re-ingesting with a fresh idempotency_key and identical (model, content_hash)
    // is a provenance no-op (unchanged = 3).
    let mut reingest_same = ingest_body.clone();
    reingest_same["idempotency_key"] = json!("idem-rag-batch-2");
    let res2 = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/rag/chunks",
        &[bearer(&token_a)],
        Some(reingest_same),
    )
    .await;
    assert_eq!(res2.status(), 200);
    let body2 = body_json(res2).await;
    assert_eq!(body2["inserted"], 0);
    assert_eq!(body2["updated"], 0);
    assert_eq!(body2["unchanged"], 3);

    // Model upgrade on chunk 0 -> updated = 1, unchanged = 2.
    let mut model_upgrade = ingest_body.clone();
    model_upgrade["idempotency_key"] = json!("idem-rag-batch-3");
    model_upgrade["chunks"][0]["model"] = json!("embeddinggemma-2-768-v2");
    let res3 = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/rag/chunks",
        &[bearer(&token_a)],
        Some(model_upgrade),
    )
    .await;
    assert_eq!(res3.status(), 200);
    let body3 = body_json(res3).await;
    assert_eq!(body3["inserted"], 0);
    assert_eq!(body3["updated"], 1);
    assert_eq!(body3["unchanged"], 2);

    // 2. Hybrid query (BM25 + vector RRF) & cross-modal retrieval.
    //    A query is ranked only inside the space it names (Wave 52 lane R14),
    //    and this batch now spans two: chunk 0 was upgraded to `-v2` while the
    //    two media chunks stayed in `-768`. The space id comes from the same
    //    catalogue the ingest used, not from a hand-written tag.
    let space_768 = core_domain::embedding::space_for_model(
        &core_domain::embedding::Catalogue::builtin(),
        "embeddinggemma-2-768",
    )
    .id();
    let space_v2 = core_domain::embedding::space_for_model(
        &core_domain::embedding::Catalogue::builtin(),
        "embeddinggemma-2-768-v2",
    )
    .id();

    //    Querying for "IPsec VPN tunnel rekey" + `vec_vpn` inside `-768` fuses
    //    the lexical and vector legs:
    //    - `att-screenshot-vpn` (image) is reached cross-modally through the
    //      vector leg.
    //    - `kb-vpn-101` matches BM25, but its vector lives in `-v2` now, so it
    //      is returned without a vector distance rather than ranked in a
    //      space it is not part of.
    let q_res = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/rag/query",
        &[bearer(&token_a)],
        Some(json!({
            "namespace": "msp-kb",
            "query_text": "IPsec VPN tunnel rekey",
            "query_vector": vec_vpn,
            "query_space": space_768,
            "limit": 10
        })),
    )
    .await;
    assert_eq!(q_res.status(), 200);
    let q_body = body_json(q_res).await;
    assert_eq!(q_body["lexical_leg_active"], true);
    assert_eq!(q_body["vector_leg_active"], true);
    let hits = q_body["hits"].as_array().expect("hits array");
    assert_eq!(
        hits.len(),
        3,
        "the lexical leg sees the whole namespace, the vector leg sees its own space"
    );
    let hit = |source_id: &str| {
        hits.iter()
            .find(|h| h["source_id"] == source_id)
            .cloned()
            .unwrap_or_else(|| panic!("missing {source_id} in {hits:?}"))
    };

    let text_hit = hit("kb-vpn-101");
    assert_eq!(text_hit["modality"], "text");
    assert_eq!(text_hit["model"], "embeddinggemma-2-768-v2");
    assert_eq!(text_hit["metadata"]["uri"], "/kb/ipsec-rekey");
    assert!(text_hit["lexical_score"].is_number());
    assert!(
        text_hit["vector_distance"].is_null(),
        "its vector belongs to the `-v2` space, so this query must not rank it: {text_hit}"
    );

    let image_hit = hit("att-screenshot-vpn");
    assert_eq!(image_hit["modality"], "image");
    assert!(image_hit["lexical_score"].is_null());
    assert!(image_hit["vector_distance"].is_number());

    let audio_hit = hit("att-audio-printer");
    assert_eq!(audio_hit["modality"], "audio");
    assert!(audio_hit["vector_distance"].is_number());

    //    The upgraded chunk's own space still answers for it: naming `-v2`
    //    returns the text passage and nothing from the other space.
    let upgraded_res = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/rag/query",
        &[bearer(&token_a)],
        Some(json!({
            "namespace": "msp-kb",
            "query_vector": vec_vpn,
            "query_space": space_v2,
            "limit": 10
        })),
    )
    .await;
    assert_eq!(upgraded_res.status(), 200);
    let upgraded_body = body_json(upgraded_res).await;
    let upgraded_hits = upgraded_body["hits"].as_array().expect("v2 hits");
    assert_eq!(
        upgraded_hits.len(),
        1,
        "only the upgraded chunk is in `-v2`"
    );
    assert_eq!(upgraded_hits[0]["source_id"], "kb-vpn-101");
    assert!(upgraded_hits[0]["vector_distance"].is_number());

    // Media->Text query: precomputed vector alone with `modalities: ["text"]`
    // retrieves the text passage from its own space.
    let m2t_res = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/rag/query",
        &[bearer(&token_a)],
        Some(json!({
            "namespace": "msp-kb",
            "query_vector": vec_vpn,
            "query_space": space_v2,
            "modalities": ["text"],
            "limit": 5
        })),
    )
    .await;
    assert_eq!(m2t_res.status(), 200);
    let m2t_body = body_json(m2t_res).await;
    assert_eq!(m2t_body["lexical_leg_active"], false);
    assert_eq!(m2t_body["vector_leg_active"], true);
    let m2t_hits = m2t_body["hits"].as_array().expect("m2t hits");
    assert_eq!(m2t_hits.len(), 1);
    assert_eq!(m2t_hits[0]["source_id"], "kb-vpn-101");

    // 3. Cross-namespace isolation: querying a different namespace in Org A returns 0 hits.
    let other_ns_res = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/rag/query",
        &[bearer(&token_a)],
        Some(json!({
            "namespace": "other-namespace",
            "query_text": "IPsec VPN tunnel rekey",
            "query_vector": vec_vpn,
            "limit": 10
        })),
    )
    .await;
    assert_eq!(other_ns_res.status(), 200);
    assert_eq!(
        body_json(other_ns_res).await["hits"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    // 4. Cross-tenant RLS isolation: Org B querying "msp-kb" gets 0 hits and cannot delete Org A's chunks.
    let org_b_q = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/rag/query",
        &[bearer(&token_b)],
        Some(json!({
            "namespace": "msp-kb",
            "query_text": "IPsec VPN tunnel rekey",
            "query_vector": vec_vpn,
            "limit": 10
        })),
    )
    .await;
    assert_eq!(org_b_q.status(), 200);
    assert_eq!(
        body_json(org_b_q).await["hits"].as_array().unwrap().len(),
        0,
        "Org B must never see Org A's vector namespace"
    );

    // 5. Machine API key access (`rag:read` vs `rag:write`):
    let create_read_key = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/api-keys",
        &[bearer(&token_a)],
        Some(json!({
            "name": "rag-reader-connector",
            "scopes": ["rag:read"]
        })),
    )
    .await;
    assert_eq!(create_read_key.status(), 201, "api key create returns 201");
    let read_key = body_json(create_read_key).await["raw_key"]
        .as_str()
        .expect("raw_key")
        .to_string();

    // `rag:read` key can query `/api/v1/rag/query` but is refused with 403 on `/api/v1/rag/chunks`.
    let key_q = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/rag/query",
        &[api_key_header(&read_key)],
        Some(json!({
            "namespace": "msp-kb",
            "query_vector": vec_vpn,
            "limit": 5
        })),
    )
    .await;
    assert_eq!(key_q.status(), 200);

    let key_write_denied = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/rag/chunks",
        &[api_key_header(&read_key)],
        Some(ingest_body.clone()),
    )
    .await;
    assert_eq!(
        key_write_denied.status(),
        403,
        "rag:read key must be denied write access"
    );

    // 6. Source deletion + provable deletion receipt verification:
    let del_res = send(
        &app,
        axum::http::Method::DELETE,
        "/api/v1/rag/sources/kb_article/kb-vpn-101?namespace=msp-kb",
        &[bearer(&token_a)],
        None,
    )
    .await;
    assert_eq!(del_res.status(), 200);
    let del_body = body_json(del_res).await;
    let receipt = &del_body["receipt"];
    let receipt_id = receipt["receipt_id"].as_str().expect("receipt_id");
    assert_eq!(receipt["namespace"], "msp-kb");
    assert_eq!(receipt["source_kind"], "kb_article");
    assert_eq!(receipt["source_id"], "kb-vpn-101");
    assert_eq!(receipt["deleted_chunks"], 1);
    assert_eq!(receipt["content_hashes"], json!(["hash-vpn-text-v1"]));

    // Fetch receipt by ID via `GET /api/v1/rag/deletion-receipts/{id}`:
    let get_rec = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/rag/deletion-receipts/{receipt_id}"),
        &[bearer(&token_a)],
        None,
    )
    .await;
    assert_eq!(get_rec.status(), 200);
    assert_eq!(body_json(get_rec).await["receipt_id"], receipt_id);

    // Org B cannot read Org A's deletion receipt (404 NotFound under RLS):
    let cross_rec = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/rag/deletion-receipts/{receipt_id}"),
        &[bearer(&token_b)],
        None,
    )
    .await;
    assert_eq!(cross_rec.status(), 404);
}
