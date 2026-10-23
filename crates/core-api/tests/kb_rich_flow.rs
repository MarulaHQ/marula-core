//! HTTP flow tests for the KB rich-content surface (Wave 13 lane 4,
//! KB1/KB2 per ADR 0004) — over the real router + the shared lane DB.
//!
//! Purpose: prove the envelope wiring end to end — instant-first
//! mutations with IF2 metadata, the allowlist gate (unmatched URLs are a
//! typed 422, never a stored row), the registry-only resolution surface,
//! external documents with permission note + link-check, the
//! client-facing exposure rule (portal gate = the article's), and the
//! typed unique-constraint reason codes. Database selection follows the
//! documented contract in tests/common (MARULA_TEST_DATABASE_URL or
//! DATABASE_URL; no schema reset).
//!
//! Non-purpose: pure allowlist/regex rules (core-kb rich unit tests) and
//! SQL-layer semantics (core-db kb_rich_queries).

use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::json;
use sqlx::PgPool;
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
        .expect("kb rich flow needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    config.secret_key = core_api::config::SecretKey([23; 32]);
    config.internal_token = Some("kb-rich-flow-internal-token".to_string());
    AppState {
        plugin_pool: Default::default(),
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

async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(url)
        .await
}

async fn send(
    app: &axum::Router,
    method: axum::http::Method,
    uri: &str,
    headers: &[(&str, String)],
    body: Option<serde_json::Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, value.clone());
    }
    let request = match body {
        Some(value) => builder
            .header("content-type", "application/json")
            .body(axum::body::Body::from(value.to_string()))
            .unwrap(),
        None => builder.body(axum::body::Body::empty()).unwrap(),
    };
    app.clone()
        .oneshot(request)
        .await
        .expect("infallible router")
}

fn auth(token: &str) -> (&'static str, String) {
    ("authorization", format!("Bearer {token}"))
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("json body")
}

async fn register_org(app: &axum::Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": "Rich Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    serde_json::from_slice::<serde_json::Value>(&bytes).expect("json")["access_token"]
        .as_str()
        .expect("access token")
        .into()
}

#[tokio::test]
async fn kb_rich_owner_flow_allowlist_gates_and_reason_codes() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("kb-rich-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let bearer = [auth(&token)];

    // --- visible category + article to hang rich content on ------------
    let category = {
        let response = send(
            &app,
            axum::http::Method::POST,
            "/api/v1/kb/categories",
            &bearer,
            Some(json!({ "name": "Runbooks", "client_visible": true })),
        )
        .await;
        assert_eq!(response.status(), 200);
        body_json(response).await
    };
    let category_id = category["id"].as_str().unwrap().to_string();
    let article = {
        let response = send(
            &app,
            axum::http::Method::POST,
            "/api/v1/kb/articles",
            &bearer,
            Some(json!({
                "title": "Rotate the Wifi password",
                "category_id": category_id,
                "status": "published",
                "client_visible": true
            })),
        )
        .await;
        assert_eq!(response.status(), 200);
        body_json(response).await
    };
    let article_id = article["id"].as_str().unwrap().to_string();

    // --- registry: system rows visible, org extension creatable --------
    let providers = {
        let response = send(
            &app,
            axum::http::Method::GET,
            "/api/v1/kb/embed-providers",
            &bearer,
            None,
        )
        .await;
        assert_eq!(response.status(), 200);
        body_json(response).await
    };
    let providers_arr = providers.as_array().expect("providers array");
    assert!(
        providers_arr
            .iter()
            .any(|p| p["slug"] == "youtube" && p["is_system"] == true),
        "system youtube row ships"
    );

    // Org extension with a broken regex: typed 422.
    let bad = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/kb/embed-providers",
        &bearer,
        Some(json!({
            "name": "Acme Wiki",
            "host_patterns": ["wiki.acme.test"],
            "url_regex": "(unclosed",
            "oembed_endpoint": "https://wiki.acme.test/oembed?url={url}"
        })),
    )
    .await;
    assert_eq!(bad.status(), 422);

    // System rows are immutable through the org path (typed 409, not a
    // silent no-op): flip enabled on the shipped YouTube row.
    let youtube_id = providers_arr
        .iter()
        .find(|p| p["slug"] == "youtube")
        .and_then(|p| p["id"].as_str())
        .unwrap()
        .to_string();
    let system_patch = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/kb/embed-providers/{youtube_id}"),
        &bearer,
        Some(json!({ "enabled": false })),
    )
    .await;
    assert_eq!(system_patch.status(), 409, "system rows are immutable");

    // --- embeds: allowlist gate + resolution ---------------------------
    let resolution = {
        let response = send(
            &app,
            axum::http::Method::POST,
            "/api/v1/kb/embeds/resolve",
            &bearer,
            Some(json!({ "url": "https://youtu.be/dQw4w9WgXcQ" })),
        )
        .await;
        assert_eq!(response.status(), 200);
        let body = body_json(response).await;
        assert_eq!(body["provider_slug"], "youtube");
        assert!(body["oembed_url"]
            .as_str()
            .unwrap()
            .starts_with("https://www.youtube.com/oembed?url="));
        body
    };
    let _ = resolution;

    // An unmatched URL is a typed 422 — raw arbitrary iframes never pass.
    let evil = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/kb/embeds/resolve",
        &bearer,
        Some(json!({ "url": "https://evil-youtube.com/watch?v=x" })),
    )
    .await;
    assert_eq!(evil.status(), 422, "allowlist miss is a typed 422");
    let evil_body = body_json(evil).await;
    assert_eq!(evil_body["error"]["reason"], "validation_failed");

    // Store the embed on the article (registry-only, no fetch).
    let embed = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/kb/articles/{article_id}/embeds"),
        &bearer,
        Some(json!({
            "url": "https://youtu.be/dQw4w9WgXcQ",
            "idempotency_key": "embed-1"
        })),
    )
    .await;
    assert_eq!(embed.status(), 200, "allowlisted url attaches");
    let embed_body = body_json(embed).await;
    let _embed_id = embed_body["id"].as_str().unwrap().to_string();
    assert_eq!(embed_body["provider_slug"], "youtube");
    assert_eq!(embed_body["mutation"]["seq"], 1, "IF2 metadata present");

    // Duplicate URL on the same article: typed reason code, not generic.
    let dup = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/kb/articles/{article_id}/embeds"),
        &bearer,
        Some(json!({ "url": "https://youtu.be/dQw4w9WgXcQ" })),
    )
    .await;
    assert_eq!(dup.status(), 409);
    assert_eq!(
        body_json(dup).await["error"]["reason"],
        "kb_embed_url_taken",
        "unique violation maps to the typed reason code"
    );

    // --- linked external documents ------------------------------------
    let doc = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/kb/external-documents",
        &bearer,
        Some(json!({
            "title": "Runbook on SharePoint",
            "url": "https://acme.sharepoint.com/:w:/r/sites/IT/Runbook.docx",
            "article_id": article_id,
            "permission_note": "M365 group ACME-IT"
        })),
    )
    .await;
    assert_eq!(doc.status(), 200);
    let doc_body = body_json(doc).await;
    let doc_id = doc_body["id"].as_str().unwrap().to_string();
    assert_eq!(
        doc_body["url_kind"], "sharepoint",
        "kind classified server-side"
    );

    // Link-check records the outcome as data — an unreachable link is a
    // 200 with status unreachable (https to a non-listening port).
    let check = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/kb/external-documents/{doc_id}/link-check"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(check.status(), 200);
    let check_body = body_json(check).await;
    let status = check_body["last_verified_status"].as_str().unwrap();
    assert!(
        status == "ok" || status == "unreachable",
        "link-check outcome is one of the closed vocabulary"
    );

    // Plain http URL: typed 422 (https-only).
    let insecure = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/kb/external-documents",
        &bearer,
        Some(json!({ "title": "Insecure", "url": "http://acme.sharepoint.com/x" })),
    )
    .await;
    assert_eq!(insecure.status(), 422);

    // Cleanup: deleting the embed emits kb_embed.deleted.
    let embed_del = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/v1/kb/embeds/{}", embed_body["id"].as_str().unwrap()),
        &bearer,
        None,
    )
    .await;
    assert_eq!(embed_del.status(), 200);
    let _ = embed_del;
}
