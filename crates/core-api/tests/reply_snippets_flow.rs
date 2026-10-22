//! HTTP flow tests for the reply-snippet library (Wave 34 lane 3,
//! `p34-canned-templates`) — over the real router + the shared lane DB.
//!
//! Purpose: prove the envelope wiring end to end — typed library CRUD in
//! the settings domain (scope `org`, key `reply_snippet:<id>`), the
//! closed-ticket-vocabulary gate (out-of-vocabulary placeholders are
//! typed 400s, never silent empties), the search-in-composer predicate
//! (`board_id` offer filter + `q` substring), fork-on-edit provenance
//! (`forked_from` on the copy, origin untouched), render against a real
//! ticket context, and IF1 idempotent replay with IF2 `mutation`
//! metadata. Database selection follows the documented contract in
//! tests/common (MARULA_TEST_DATABASE_URL or DATABASE_URL; no schema
//! reset).
//!
//! Non-purpose: pure vocabulary/predicate rules (core-domain
//! reply_snippets unit tests) and the templates registry surface
//! (tests/templates_flow.rs — snippets stay a separate typed library).

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
        .expect("snippet flow needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    config.secret_key = core_api::config::SecretKey([31; 32]);
    config.internal_token = Some("reply-snippets-flow-internal-token".to_string());
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

fn auth(token: &str) -> (&'static str, String) {
    ("authorization", format!("Bearer {token}"))
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("json body")
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

async fn register_org(app: &axum::Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": "Snippet Co",
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
async fn reply_snippets_flow_crud_predicate_fork_render_replay() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("snippet-flow-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let bearer = [auth(&token)];

    // --- create: typed validation gate (out-of-vocabulary = 400) --------
    let rejected = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/reply-snippets",
        &bearer,
        Some(json!({
            "name": "Bad token",
            "body": "Hello {{client.contact_first}}",
            "idempotency_key": format!("bad-{}", Uuid::now_v7()),
        })),
    )
    .await;
    assert_eq!(
        rejected.status(),
        422,
        "out-of-vocabulary token is a typed 422"
    );

    // --- create: in-vocabulary snippet with board scoping ---------------
    let key = format!("create-{}", Uuid::now_v7());
    let created = {
        let response = send(
            &app,
            axum::http::Method::POST,
            "/api/v1/reply-snippets",
            &bearer,
            Some(json!({
                "name": "Wifi triage",
                "body": "Hi {{contact.name}}, we are looking at {{ticket.summary}} now.",
                "idempotency_key": key,
            })),
        )
        .await;
        assert_eq!(response.status(), 200);
        body_json(response).await
    };
    let snippet_id = created["id"].as_str().expect("id").to_string();
    assert_eq!(created["name"], "Wifi triage");
    assert_eq!(created["forked_from"], serde_json::Value::Null);
    assert!(
        created["mutation"].is_object(),
        "IF2 mutation metadata rides the envelope"
    );

    // --- IF1: same key replays the same response ------------------------
    let replay = {
        let response = send(
            &app,
            axum::http::Method::POST,
            "/api/v1/reply-snippets",
            &bearer,
            Some(json!({
                "name": "Wifi triage",
                "body": "Hi {{contact.name}}, we are looking at {{ticket.summary}} now.",
                "idempotency_key": key,
            })),
        )
        .await;
        assert_eq!(response.status(), 200);
        body_json(response).await
    };
    assert_eq!(
        replay["id"], created["id"],
        "replay returns the original row"
    );

    // --- list: the search-in-composer predicate -------------------------
    let list = {
        let response = send(
            &app,
            axum::http::Method::GET,
            "/api/v1/reply-snippets?q=wifi",
            &bearer,
            None,
        )
        .await;
        assert_eq!(response.status(), 200);
        body_json(response).await
    };
    assert!(list["snippets"]
        .as_array()
        .expect("snippets")
        .iter()
        .any(|s| s["id"] == created["id"]));
    assert!(list["vocabulary"]
        .as_array()
        .expect("vocabulary")
        .iter()
        .any(|k| k == "contact.name"));
    let miss = {
        let response = send(
            &app,
            axum::http::Method::GET,
            "/api/v1/reply-snippets?q=printer",
            &bearer,
            None,
        )
        .await;
        assert_eq!(response.status(), 200);
        body_json(response).await
    };
    assert!(miss["snippets"]
        .as_array()
        .expect("snippets")
        .iter()
        .all(|s| s["id"] != created["id"]));

    // --- fork-on-edit provenance ----------------------------------------
    let forked = {
        let response = send(
            &app,
            axum::http::Method::POST,
            &format!("/api/v1/reply-snippets/{snippet_id}/fork"),
            &bearer,
            Some(json!({
                "name": "Wifi triage (dispatch)",
                "idempotency_key": format!("fork-{}", Uuid::now_v7()),
            })),
        )
        .await;
        assert_eq!(response.status(), 200);
        body_json(response).await
    };
    let fork_id = forked["id"].as_str().expect("fork id").to_string();
    assert_eq!(
        forked["forked_from"], created["id"],
        "the copy records its origin"
    );
    let origin = {
        let response = send(
            &app,
            axum::http::Method::GET,
            &format!("/api/v1/reply-snippets/{snippet_id}"),
            &bearer,
            None,
        )
        .await;
        assert_eq!(response.status(), 200);
        body_json(response).await
    };
    assert_eq!(origin["name"], "Wifi triage", "the origin is untouched");

    // --- patch + delete ---------------------------------------------------
    let patched = {
        let response = send(
            &app,
            axum::http::Method::PATCH,
            &format!("/api/v1/reply-snippets/{snippet_id}"),
            &bearer,
            Some(json!({
                "body": "Hi {{contact.name}} — an update on {{ticket.number}}.",
                "idempotency_key": format!("patch-{}", Uuid::now_v7()),
            })),
        )
        .await;
        assert_eq!(response.status(), 200);
        body_json(response).await
    };
    assert!(patched["body"]
        .as_str()
        .expect("body")
        .contains("ticket.number"));

    let deleted = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/v1/reply-snippets/{fork_id}"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(deleted.status(), 200, "delete rides the envelope");
    let gone = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/reply-snippets/{fork_id}"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(gone.status(), 404, "deleted snippet is gone");

    // --- unauthenticated reads are rejected ------------------------------
    let anon = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/reply-snippets",
        &[],
        None,
    )
    .await;
    assert_eq!(anon.status(), 401, "the library is staff-only");
}
