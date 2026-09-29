//! Configuration-identity endpoint flows (ADR 0003, Wave 12 lane 3,
//! migration 0061) through the REAL core-api router.
//!
//! Purpose: the /api/config-items contract —
//! 1. CI2 feed upsert: first sync creates the canonical item + link; a
//!    second sync with the SAME (source_system, external_id) replays into
//!    the SAME item (never a twin); a same-key+different-payload replay is
//!    the typed 409 (IF1).
//! 2. CI5 curation: PATCH sets curated values; a later feed sync cannot
//!    overwrite them; accept-observed promotes the observed value.
//! 3. CI4 merge queue: ambiguous matches queue candidates; accept folds
//!    the candidate into the primary with its evidence; split undoes the
//!    merge; the org threshold is readable/updatable.
//! 4. Permission posture: read-only gets typed 403s on writes.
//!
//! Database selection follows assets_flow's contract:
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
        .expect("config items flow needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    config.secret_key = core_api::config::SecretKey([23; 32]);
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
            "org_name": "CI Co",
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

#[tokio::test]
async fn config_items_flow_feed_upsert_curation_merge_queue() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("ci-flow-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let bearer = [("authorization", format!("Bearer {token}"))];
    let org_id = me_org_id(&app, &token).await;

    // --- client to scope items under ------------------------------------
    let client = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/v1/clients",
            &bearer,
            Some(json!({ "name": "Acme Systems" })),
        )
        .await,
    )
    .await;
    let client_id = client["id"].as_str().unwrap().to_string();

    let feed_body = |observed: Value, key: &str| {
        json!({
            "source_system": "ninjaone", "external_id": "N-1", "client_id": client_id,
            "tag": "SRV-01", "name": "Server 01",
            "identity_keys": { "serial": "SN-2024", "hostname": "mail" },
            "observed_fields": observed,
            "idempotency_key": key
        })
    };

    // --- CI2: first sync creates the canonical item + link ---------------
    let first = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/v1/config-items/feed-upsert",
            &bearer,
            Some(feed_body(
                json!({ "ip_address": "10.0.0.1", "owner": "Feed One" }),
                "sync-1",
            )),
        )
        .await,
    )
    .await;
    assert_eq!(first["created_item"], json!(true));
    assert_eq!(first["resolved_existing"], json!(false));
    assert_eq!(first["source_link"]["source_system"], json!("ninjaone"));
    let item_id = first["id"].as_str().unwrap().to_string();
    assert_eq!(first["fields"]["ip_address"], json!("10.0.0.1"));

    // --- CI2: the same source record re-syncs into the SAME item --------
    let second = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/v1/config-items/feed-upsert",
            &bearer,
            Some(feed_body(json!({ "ip_address": "10.0.0.2" }), "sync-2")),
        )
        .await,
    )
    .await;
    assert_eq!(
        second["created_item"],
        json!(false),
        "second sync never creates a twin"
    );
    assert_eq!(second["id"], json!(item_id));
    assert_eq!(second["fields"]["ip_address"], json!("10.0.0.2"));

    // --- IF1: same idempotency key + same payload replays the SAME result
    let replay = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/v1/config-items/feed-upsert",
            &bearer,
            Some(feed_body(
                json!({ "ip_address": "10.0.0.1", "owner": "Feed One" }),
                "sync-1",
            )),
        )
        .await,
    )
    .await;
    assert_eq!(replay["id"], json!(item_id), "idempotent replay");
    // A different payload under the same key is the typed 409.
    let conflicting = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/config-items/feed-upsert",
        &bearer,
        Some(feed_body(json!({}), "sync-1")),
    )
    .await;
    assert_eq!(
        conflicting.status(),
        409,
        "same key + different payload is typed 409"
    );

    // --- CI5 curation: human values are sticky --------------------------
    let curated = body_json(
        send(
            &app,
            axum::http::Method::PATCH,
            &format!("/api/v1/config-items/{item_id}"),
            &bearer,
            Some(json!({ "fields": { "owner": "Alice" } })),
        )
        .await,
    )
    .await;
    assert_eq!(curated["curated_fields"]["owner"], json!("Alice"));

    // A feed sync cannot overwrite the curated owner — contradiction recorded.
    let third = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/v1/config-items/feed-upsert",
            &bearer,
            Some(json!({
                "source_system": "ninjaone", "external_id": "N-1", "client_id": client_id,
                "tag": "SRV-01", "name": "Server 01",
                "identity_keys": { "serial": "SN-2024", "hostname": "mail" },
                "observed_fields": { "owner": "Bot McBotface" },
                "idempotency_key": "sync-3"
            })),
        )
        .await,
    )
    .await;
    assert_eq!(
        third["fields"]["owner"],
        json!("Alice"),
        "curated beats observed"
    );
    assert_eq!(third["contradictions"], json!(["owner"]));
    assert_eq!(
        third["field_sources"]["owner"]["contradicts_curated"],
        json!(true)
    );

    // --- CI5 one-click accept-observed ----------------------------------
    let accepted = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/v1/config-items/{item_id}/accept-observed"),
            &bearer,
            Some(json!({ "field_key": "owner" })),
        )
        .await,
    )
    .await;
    assert_eq!(accepted["curated_fields"]["owner"], json!("Bot McBotface"));
    assert!(
        accepted["field_sources"]["owner"]
            .get("contradicts_curated")
            .is_none(),
        "contradiction cleared on accept"
    );

    // --- CI4: an ambiguous hostname match queues a reviewable candidate -
    let ambiguous = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/v1/config-items/feed-upsert",
            &bearer,
            Some(json!({
                "source_system": "it_glue", "external_id": "G-1", "client_id": client_id,
                "tag": "SRV-01-B", "name": "Server 01 B",
                "identity_keys": { "hostname": "mail" },
                "idempotency_key": "sync-4"
            })),
        )
        .await,
    )
    .await;
    assert_eq!(ambiguous["created_item"], json!(true));
    assert_eq!(
        ambiguous["queued_candidates"].as_array().map(Vec::len),
        Some(1),
        "reviewable candidate with evidence"
    );

    // The queue lists it with both tags; reject keeps both items.
    let queue_response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/config-items/merge-candidates?status=pending",
        &bearer,
        None,
    )
    .await;
    eprintln!("queue status: {}", queue_response.status());
    let queue = body_json(queue_response).await;
    assert!(
        !queue.as_array().expect("queue array").is_empty(),
        "queue lists the pending candidate"
    );
    let candidate_id = ambiguous["queued_candidates"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let rejected = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/v1/config-items/merge-candidates/{candidate_id}/reject"),
            &bearer,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(rejected["status"], json!("rejected"));

    // --- CI4: explicit merge is audited + split restores ----------------
    let merge_response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/config-items/{item_id}/merge"),
        &bearer,
        Some(json!({ "merged_config_item_id": ambiguous["id"], "reason": "same host" })),
    )
    .await;
    eprintln!("merge status: {}", merge_response.status());
    let merged = body_json(merge_response).await;
    assert_eq!(merged["status"], json!("applied"));

    // Tombstone excluded from the active list.
    let list = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/v1/config-items?client_id={client_id}"),
            &bearer,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 1);

    let split_response = send(
        &app,
        axum::http::Method::POST,
        &format!(
            "/api/v1/config-items/merges/{}/split",
            merged["id"].as_str().unwrap()
        ),
        &bearer,
        None,
    )
    .await;
    eprintln!("split status: {}", split_response.status());
    let split = body_json(split_response).await;
    assert_eq!(split["status"], json!("undone"));

    // After the split, both items are active again.
    let list = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/v1/config-items?client_id={client_id}"),
            &bearer,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        list.as_array().unwrap().len(),
        2,
        "split restores the absorbed item"
    );

    // --- org threshold is readable + configurable ------------------------
    let settings = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/v1/config-items/settings",
            &bearer,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(settings["auto_merge_threshold"], json!(0.85));
    let updated = body_json(
        send(
            &app,
            axum::http::Method::PUT,
            "/api/v1/config-items/settings",
            &bearer,
            Some(json!({ "auto_merge_threshold": 0.9 })),
        )
        .await,
    )
    .await;
    assert_eq!(updated["auto_merge_threshold"], json!(0.9));

    // --- permission posture: read-only cannot write -----------------------
    let ro_password = "still-correct-horse";
    let ro_token = provision_read_only(
        &state,
        &app,
        org_id,
        &format!("ro@{slug}.test"),
        ro_password,
    )
    .await;
    let ro = [("authorization", format!("Bearer {ro_token}"))];
    let forbidden = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/config-items/{item_id}"),
        &ro,
        Some(json!({ "fields": { "owner": "Nope" } })),
    )
    .await;
    assert_eq!(forbidden.status(), 403, "read-only cannot curate");
    let ok_read = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/config-items/{item_id}"),
        &ro,
        None,
    )
    .await;
    assert_eq!(ok_read.status(), 200, "read-only can still view the CI");
}

async fn provision_read_only(
    state: &AppState,
    app: &Router,
    org_id: Uuid,
    email: &str,
    password: &str,
) -> String {
    let password_hash = core_api::auth::password::hash(password).expect("hash");
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    let role: Uuid =
        sqlx::query_scalar("SELECT id FROM roles WHERE org_id = $1 AND builtin = 'read_only'")
            .bind(org_id)
            .fetch_one(&mut *tx)
            .await
            .expect("read_only role exists from registration");
    let user_id = Uuid::now_v7();
    core_db::users::create_user(
        &mut *tx,
        user_id,
        org_id,
        email,
        "RO User",
        &password_hash,
        "UTC",
    )
    .await
    .expect("user insert");
    core_db::users::assign_role(&mut *tx, user_id, role)
        .await
        .expect("role assign");
    tx.commit().await.expect("commit");
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/login",
        &[],
        Some(json!({ "email": email, "password": password })),
    )
    .await;
    assert_eq!(response.status(), 200);
    body_json(response).await["access_token"]
        .as_str()
        .unwrap()
        .to_string()
}

// Per-connector enable/disable gate (Wave 15 lane 4 `p15-docs-closeout`):
// the org-level `connectors.ci_feeders` toggle is enforced at the
// feed-upsert seam — accepted while on (and instantly replayable), typed
// 403 while off; OTHER connector sources stay ungated.
#[tokio::test]
async fn connector_disabled_rejects_feed_upsert_with_typed_403() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("ci-gate-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let bearer = [("authorization", format!("Bearer {token}"))];
    let _org_id = me_org_id(&app, &token).await;

    let client = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/v1/clients",
            &bearer,
            Some(json!({ "name": "Gate Client" })),
        )
        .await,
    )
    .await;
    let client_id = client["id"].as_str().unwrap().to_string();

    let feed_body = |source: &str, key: &str| {
        json!({
            "source_system": source, "external_id": "G-1", "client_id": client_id,
            "tag": "SRV-G", "name": "Gate Server",
            "identity_keys": { "serial": "SN-GATE" },
            "observed_fields": { "ip_address": "10.9.0.1" },
            "idempotency_key": key
        })
    };

    // Sponsored state: while the connector is ON (default), the write lands
    // AND its idempotency key is claimed — a later replay must still replay.
    let first = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/v1/config-items/feed-upsert",
            &bearer,
            Some(feed_body("ninjaone", "gate-sync-1")),
        )
        .await,
    )
    .await;
    assert_eq!(first["created_item"], json!(true));

    // The operator toggles the connector off via the settings hub.
    let off = send(
        &app,
        axum::http::Method::PUT,
        "/api/v1/settings/org",
        &bearer,
        Some(json!({ "key": "connectors.ci_feeders", "value": false })),
    )
    .await;
    assert_eq!(off.status(), 200, "org owner can edit settings");
    assert_eq!(body_json(off).await["key"], json!("connectors.ci_feeders"));

    let rejected = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/config-items/feed-upsert",
        &bearer,
        Some(feed_body("ninjaone", "gate-sync-2")),
    )
    .await;
    assert_eq!(rejected.status(), 403, "disabled connector is typed 403");
    let detail = body_json(rejected).await;
    assert!(
        detail["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("connectors.ci_feeders"),
        "error copy names the org toggle: {detail}"
    );

    // A different connector family stays ON (per-connector, not global).
    let other = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/config-items/feed-upsert",
        &bearer,
        Some(feed_body("immybot", "gate-sync-3")),
    )
    .await;
    assert_eq!(
        other.status(),
        200,
        "unrelated connector source stays ungated"
    );

    // Turning it back on un-gates the seam.
    let re_enable = send(
        &app,
        axum::http::Method::PUT,
        "/api/v1/settings/org",
        &bearer,
        Some(json!({ "key": "connectors.ci_feeders", "value": true })),
    )
    .await;
    assert_eq!(re_enable.status(), 200);
    let allowed = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/config-items/feed-upsert",
        &bearer,
        Some(feed_body("ninjaone", "gate-sync-2")),
    )
    .await;
    assert_eq!(allowed.status(), 200, "re-enabled connector accepts writes");
}
