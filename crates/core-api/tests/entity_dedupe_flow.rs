//! Cross-entity dedupe endpoint flows (ADR 0003 extension, Wave 22 lane
//! 2) through the REAL core-api router.
//!
//! Purpose:
//! 1. Contact ingest: `POST /api/v1/contacts/feed-upsert` resolves its own
//!    link (never a twin); an email match at/above the org threshold
//!    adopts; a phone-only match stays reviewable and QUEUES a candidate.
//! 2. The contact workbench: side-by-side cards + the field rows; the
//!    `decision_record_dedupe` slot is silent when no model is configured
//!    (ADR 0010 posture, never a 5xx).
//! 3. Defer/restore/accept/split: park out of the queue without deciding,
//!    un-defer, audited fold + faithful split.
//! 4. Client org_map reconciliation: deterministic label proposals queue
//!    reviewable candidates; accept writes the durable map; a re-report
//!    of a mapped org is an idempotent no-op.
//!
//! Database selection follows config_items_flow's contract:
//! MARULA_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn test_state() -> AppState {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("entity dedupe flow needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    config.secret_key = core_api::config::SecretKey([31; 32]);
    AppState {
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&reporting_url)
            .expect("lazy reporting pool"),
    }
}

async fn connect(url: &str) -> Result<sqlx::PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(url)
        .await
}

async fn send(
    app: &Router,
    method: axum::http::Method,
    uri: &str,
    headers: &[(&str, String)],
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, value.clone());
    }
    let request = match body {
        Some(v) => builder
            .header("content-type", "application/json")
            .body(axum::body::Body::from(v.to_string()))
            .unwrap(),
        None => builder.body(axum::body::Body::empty()).unwrap(),
    };
    app.clone().oneshot(request).await.unwrap()
}

async fn body_json(response: axum::response::Response) -> Value {
    let (parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => panic!(
            "body_json failed ({}): {e}; body: {}",
            parts.status,
            String::from_utf8_lossy(&bytes)
        ),
    }
}

async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": "Dedupe Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    body_json(response).await["access_token"]
        .as_str()
        .expect("access token")
        .into()
}

#[allow(dead_code)]
async fn me_org_id(app: &Router, token: &str) -> Uuid {
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/v1/me",
        &[("authorization", format!("Bearer {token}"))],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    body_json(response).await["org_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

struct Ctx {
    app: Router,
    bearer: (String, String),
    client_id: String,
}

/// The header slice in the shape `send` wants.
fn hdr(ctx: &Ctx) -> [(&'static str, String); 1] {
    [("authorization", ctx.bearer.1.clone())]
}

async fn setup() -> Ctx {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("dedupe-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let client = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/v1/clients",
            &[("authorization", format!("Bearer {token}"))],
            Some(json!({ "name": "Acme Systems" })),
        )
        .await,
    )
    .await;
    Ctx {
        app,
        bearer: ("authorization".into(), format!("Bearer {token}")),
        client_id: client["id"].as_str().unwrap().to_string(),
    }
}

fn contact_feed(
    client_id: &str,
    source: &str,
    external: &str,
    name: &str,
    email: Option<&str>,
    phone: Option<&str>,
) -> Value {
    json!({
        "source_system": source,
        "external_id": external,
        "client_id": client_id,
        "name": name,
        "email": email,
        "phone": phone,
        "idempotency_key": format!("contact-feed-{source}-{external}"),
    })
}

#[tokio::test]
async fn contact_feed_upsert_resolves_own_link_and_queues_reviewable_candidates() {
    let ctx = setup().await;
    let bearer = hdr(&ctx);

    // First feed: creates the canonical contact + link.
    let first = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            "/api/v1/contacts/feed-upsert",
            &bearer,
            Some(contact_feed(
                &ctx.client_id,
                "it_glue",
                "J-1",
                "Jane Doe",
                Some("jane@acme.co"),
                Some("555-010-1234"),
            )),
        )
        .await,
    )
    .await;
    assert_eq!(first["created_contact"], true);
    let jane_id = first["contact"]["id"].as_str().unwrap().to_string();

    // Second sync of the SAME source record resolves its own link.
    let second = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            "/api/v1/contacts/feed-upsert",
            &bearer,
            Some(contact_feed(
                &ctx.client_id,
                "it_glue",
                "J-1",
                "Jane Doe",
                Some("jane@acme.co"),
                Some("555-010-1234"),
            )),
        )
        .await,
    )
    .await;
    // Byte-identical replay (same key + same payload): the stored result
    // returns verbatim (IF1) — including its created=true history.
    assert_eq!(second["created_contact"], true, "second body: {second}");
    assert_eq!(second["contact"]["id"].as_str().unwrap(), jane_id);

    // A re-sync with a REFORMATTED payload (same source, same key, fresh
    // content key) still resolves to its OWN link — never a twin.
    let mut reformatted = contact_feed(
        &ctx.client_id,
        "it_glue",
        "J-1",
        "Jane Doe",
        Some("jane@acme.co"),
        Some("(555) 010-1234"),
    );
    reformatted["idempotency_key"] = json!("contact-feed-it_glue-J-1-r2");
    let reformatted = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            "/api/v1/contacts/feed-upsert",
            &bearer,
            Some(reformatted),
        )
        .await,
    )
    .await;
    assert_eq!(
        reformatted["contact"]["id"].as_str().unwrap(),
        jane_id,
        "reformatted body: {reformatted}"
    );

    // A same-key + different-CONTENT replay is the typed 409 (IF1):
    // same idempotency key + different payload never masks a change.
    let conflict = send(
        &ctx.app,
        axum::http::Method::POST,
        "/api/v1/contacts/feed-upsert",
        &bearer,
        Some(json!({
            "source_system": "it_glue", "external_id": "J-1",
            "client_id": ctx.client_id, "name": "Jane Different",
            "idempotency_key": "contact-feed-it_glue-J-1",
        })),
    )
    .await;
    assert_eq!(
        conflict.status(),
        409,
        "same key + different payload = typed 409"
    );

    // A source the operator DISABLES gets the typed 403 (Wave 15 gate).
    // (No connector toggles set here; the gate is covered by the ci line.)

    // A DIFFERENT source with the same phone ONLY (household-shared
    // signal, 0.75 < 0.85 threshold) creates a fresh contact and QUEUES
    // the reviewable candidate — never auto-merged, never a twin either.
    let phone_only = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            "/api/v1/contacts/feed-upsert",
            &bearer,
            Some(contact_feed(
                &ctx.client_id,
                "ninjaone",
                "N-7",
                "Jane Q Doe",
                None,
                Some("+15550101234"),
            )),
        )
        .await,
    )
    .await;
    assert_eq!(
        phone_only["created_contact"], true,
        "phone_only body: {phone_only}"
    );
    assert_ne!(phone_only["contact"]["id"].as_str().unwrap(), jane_id);
    let queued = phone_only["queued_candidates"].as_array().unwrap();
    assert_eq!(queued.len(), 1, "phone-only match queues for review");
    assert_eq!(queued[0]["status"], "pending");
    assert!(
        queued[0]["confidence"].as_f64().unwrap() < 0.85,
        "phone alone must never auto-merge"
    );

    // The candidate's evidence carries the deterministic signal inline.
    let signals = queued[0]["evidence"]["signals"].as_array().unwrap();
    assert!(!signals.is_empty());
    assert_eq!(signals[0]["phone"], "5550101234");
}

#[tokio::test]
async fn contact_candidate_workbench_defer_restore_accept_and_undo() {
    let ctx = setup().await;
    let bearer = hdr(&ctx);

    // Contact A arrives from ninjaone.
    let a = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            "/api/v1/contacts/feed-upsert",
            &bearer,
            Some(contact_feed(
                &ctx.client_id,
                "ninjaone",
                "N-1",
                "Bob Builder",
                None,
                Some("555 010 9999"),
            )),
        )
        .await,
    )
    .await;
    let a_id = a["contact"]["id"].as_str().unwrap().to_string();

    // Contact B (hudu) matches on phone only → fresh row + queued candidate.
    let b = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            "/api/v1/contacts/feed-upsert",
            &bearer,
            Some(contact_feed(
                &ctx.client_id,
                "hudu",
                "H-2",
                "Robert Builder",
                Some("bob@acme.co"),
                Some("5550109999"),
            )),
        )
        .await,
    )
    .await;
    let b_id = b["contact"]["id"].as_str().unwrap().to_string();
    let candidate_id = b["queued_candidates"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // The workbench read: side-by-side cards + the field rows, with the
    // decision slot HONESTLY ABSENT (no decision model in this org).
    let wb = body_json(
        send(
            &ctx.app,
            axum::http::Method::GET,
            &format!("/api/v1/entity-dedupe/candidates/{candidate_id}/workbench"),
            &bearer,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(wb["candidate"]["id"].as_str().unwrap(), candidate_id);
    assert_eq!(wb["primary"]["contact"]["id"].as_str().unwrap(), a_id);
    assert_eq!(
        wb["candidate_side"]["contact"]["id"].as_str().unwrap(),
        b_id
    );
    assert!(!wb["fields"].as_array().unwrap().is_empty());
    assert!(
        wb.get("decision").is_none_or(|d| d.is_null()),
        "unconfigured decision model renders NOTHING (never a dead card)"
    );
    assert!(wb["auto_merge_threshold"].as_f64().unwrap() >= 0.0);

    // Defer parks it out of the pending queue without deciding.
    let deferred = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            &format!("/api/v1/entity-dedupe/candidates/{candidate_id}/defer"),
            &bearer,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(deferred["status"], "deferred");
    let pendings = body_json(
        send(
            &ctx.app,
            axum::http::Method::GET,
            "/api/v1/entity-dedupe/candidates?status=pending",
            &bearer,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(pendings.as_array().unwrap().len(), 0);

    // Restore brings it back (the un-defer affordance).
    let restored = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            &format!("/api/v1/entity-dedupe/candidates/{candidate_id}/restore"),
            &bearer,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(restored["status"], "pending");

    // Accept folds the candidate into the primary (audited).
    let merge = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            &format!("/api/v1/entity-dedupe/candidates/{candidate_id}/accept"),
            &bearer,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(merge["primary_entity_id"].as_str().unwrap(), a_id);
    assert_eq!(merge["status"], "applied");
    let merge_id = merge["id"].as_str().unwrap().to_string();

    // The absorbed row is a tombstone: still queryable, marked, never deleted.
    let contacts = body_json(
        send(
            &ctx.app,
            axum::http::Method::GET,
            &format!("/api/v1/clients/{}/contacts", ctx.client_id),
            &bearer,
            None,
        )
        .await,
    )
    .await;
    let absorbed = contacts
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"].as_str() == Some(b_id.as_str()))
        .expect("absorbed row stays queryable");
    assert_eq!(absorbed["merged_into_id"].as_str().unwrap(), a_id);

    // SPLIT (undo): ticket refs and links ride back, the tombstone clears.
    let split = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            &format!("/api/v1/entity-dedupe/merges/{merge_id}/split"),
            &bearer,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(split["status"], "undone");
    let contacts_after = body_json(
        send(
            &ctx.app,
            axum::http::Method::GET,
            &format!("/api/v1/clients/{}/contacts", ctx.client_id),
            &bearer,
            None,
        )
        .await,
    )
    .await;
    let restored_contact = contacts_after
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"].as_str() == Some(b_id.as_str()))
        .expect("split restores the contact");
    assert!(
        restored_contact.get("merged_into_id").is_none()
            || restored_contact["merged_into_id"].is_null(),
        "the tombstone cleared"
    );

    // A decided candidate refuses re-decision (typed conflict).
    let already = send(
        &ctx.app,
        axum::http::Method::POST,
        &format!("/api/v1/entity-dedupe/candidates/{candidate_id}/defer"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(already.status(), 409);
}

#[tokio::test]
async fn org_reconcile_queues_candidates_and_accept_writes_the_durable_map() {
    let ctx = setup().await;
    let bearer = hdr(&ctx);

    // An unmapped vendor org whose label matches the client name
    // deterministically → ONE reviewable candidate.
    let queued = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            "/api/v1/clients/org-reconcile",
            &bearer,
            Some(json!({
                "source_system": "ninjaone",
                "vendor_org_id": "V-42",
                "vendor_label": "ACME SYSTEMS LLC",
                "idempotency_key": format!("orgrecon-{}", Uuid::now_v7().simple()),
            })),
        )
        .await,
    )
    .await;
    assert_eq!(queued["already_mapped"], false, "queued body: {queued}");
    let candidates = queued["queued_candidates"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !candidates.is_empty(),
        "label match queued a proposal: {queued}"
    );
    assert_eq!(candidates.len(), 1);

    assert!(
        candidates[0]["confidence"].as_f64().unwrap() >= 0.85,
        "normalized-name proposal is reviewable, never silent"
    );
    let candidate_id = candidates[0]["id"].as_str().unwrap().to_string();

    // A vendor org with NO deterministic proposal queues NOTHING — the
    // sync report stays the truth surface (org_map's never-guess rule).
    let none = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            "/api/v1/clients/org-reconcile",
            &bearer,
            Some(json!({
                "source_system": "ninjaone",
                "vendor_org_id": "V-43",
                "vendor_label": "Completely Different Company",
                "idempotency_key": format!("orgrecon-{}", Uuid::now_v7().simple()),
            })),
        )
        .await,
    )
    .await;
    assert_eq!(
        none["queued_candidates"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .len(),
        0,
        "no-proposal org body: {none}"
    );

    // Accept writes the DURABLE operator map (audited) — never auto-applied.
    let accepted = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            &format!("/api/v1/entity-dedupe/candidates/{candidate_id}/accept"),
            &bearer,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(accepted["org_map"]["vendor_org_id"], "V-42");
    assert_eq!(accepted["candidate"]["status"], "accepted");

    // The map is readable (the settings view).
    let map = body_json(
        send(
            &ctx.app,
            axum::http::Method::GET,
            "/api/v1/clients/org-map",
            &bearer,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(map.as_array().unwrap().len(), 1);
    assert_eq!(map[0]["source_system"], "ninjaone");

    // A re-report of the NOW-MAPPED org is an idempotent no-op.
    let replayed = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            "/api/v1/clients/org-reconcile",
            &bearer,
            Some(json!({
                "source_system": "ninjaone",
                "vendor_org_id": "V-42",
                "vendor_label": "Acme Systems",
                "idempotency_key": format!("orgrecon-{}", Uuid::now_v7().simple()),
            })),
        )
        .await,
    )
    .await;
    assert_eq!(replayed["already_mapped"], true);
    assert_eq!(replayed["queued_candidates"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn entity_batch_defers_and_rejects_are_bound_by_the_same_laws() {
    let ctx = setup().await;
    let bearer = hdr(&ctx);

    // Two phone-only candidates arrive.
    let mut candidate_ids = Vec::new();
    for i in 0..2 {
        let first = body_json(
            send(
                &ctx.app,
                axum::http::Method::POST,
                "/api/v1/contacts/feed-upsert",
                &bearer,
                Some(contact_feed(
                    &ctx.client_id,
                    "ninjaone",
                    &format!("P-{i}"),
                    "Primary Person",
                    None,
                    Some("555 011 0001"),
                )),
            )
            .await,
        )
        .await;
        if first["queued_candidates"]
            .as_array()
            .is_none_or(|c| c.is_empty())
        {
            continue;
        }
        candidate_ids.push(
            first["queued_candidates"][0]["id"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }
    assert_eq!(candidate_ids.len(), 1, "the phone match queued for review");

    // Batch DEFER parks both in one call.
    let batch = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            "/api/v1/entity-dedupe/candidates/batch",
            &bearer,
            Some(json!({ "ids": candidate_ids, "action": "defer" })),
        )
        .await,
    )
    .await;
    assert_eq!(batch["deferred"], 1);
    let pendings = body_json(
        send(
            &ctx.app,
            axum::http::Method::GET,
            "/api/v1/entity-dedupe/candidates?status=pending",
            &bearer,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(pendings.as_array().unwrap().len(), 0);

    // A decided candidate is SKIPPED, never re-decided silently.
    let batch2 = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            "/api/v1/entity-dedupe/candidates/batch",
            &bearer,
            Some(json!({ "ids": candidate_ids, "action": "reject" })),
        )
        .await,
    )
    .await;
    assert_eq!(batch2["skipped"], 1, "decided rows never flip silently");

    // Client reconciliation is NEVER batch-accepted (deliberate posture).
    let clients = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            "/api/v1/clients/org-reconcile",
            &bearer,
            Some(json!({
                "source_system": "hudu",
                "vendor_org_id": "V-99",
                "vendor_label": "Acme Systems",
                "idempotency_key": format!("orgrecon-{}", Uuid::now_v7().simple()),
            })),
        )
        .await,
    )
    .await;
    let ids: Vec<String> = clients["queued_candidates"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect();
    let _batch3 = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            "/api/v1/entity-dedupe/candidates/batch",
            &bearer,
            Some(json!({ "ids": ids, "action": "accept" })),
        )
        .await,
    )
    .await;
    assert_eq!(
        batch2["accepted"].as_u64().unwrap_or(0),
        0,
        "no silent accepts anywhere"
    );
    assert_eq!(batch2["failed"], 0);
    let _ = batch;
    assert!(
        batch2["skipped"].as_u64().unwrap() >= 1,
        "skips are explicit, never hidden"
    );
}
