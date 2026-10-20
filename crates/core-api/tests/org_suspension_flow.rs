//! HTTP flow tests for the tenant suspension notice flag (the private build tracker Wave 47
//! lane A7, `p47-org-suspension-notice`, migration 0136) over the REAL
//! core-api router.
//!
//! Purpose: prove the surface end to end, and — more importantly — that the
//! flag does not become a gate:
//!   * the state moves not-flagged → flagged (reason + notice interval) →
//!     cleared, and re-flagging an already-flagged org keeps the original
//!     `suspended_at` (idempotent set: a retry cannot restart the countdown).
//!   * a FLAGGED ORG KEEPS WORKING: its profile still reads and still writes.
//!     This is the load-bearing rule (the private build tracker A7 (C)) and the assertion that
//!     fails loudly if a later lane adds enforcement to the flag.
//!   * the write body is validated: an unknown or missing reason is a typed
//!     422, not a raw constraint violation.
//!   * a path org that is not the caller's own is a 404 for BOTH read and
//!     write — the RLS/`NotFound` posture.
//!   * a non-admin human MEMBER reads their own state but is refused on the
//!     write (`admin:edit` is the human manage gate).
//!   * a service account needs `org_suspension:read` to read, and can never
//!     write: there is no machine manage grant by design.
//!   * the state change emits `org.suspension_changed` (the registered
//!     contract event) plus an audit row.
//!
//! Non-purpose: no enforcement test, because there is no enforcement to test.
//!
//! Database selection follows the documented contract: MARULA_TEST_DATABASE_URL
//! or DATABASE_URL (no schema reset).

use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
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
        .expect("suspension flow needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    config.secret_key = core_api::config::SecretKey([43; 32]);
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

async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(url)
        .await
}

async fn body_json(response: axum::response::Response) -> Value {
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
    body: Option<Value>,
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

fn bearer(token: &str) -> Vec<(&'static str, String)> {
    vec![("authorization", format!("Bearer {token}"))]
}

async fn register_org(app: &axum::Router, state: &AppState, slug: &str) -> (String, Uuid) {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": "Notice Co",
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

#[tokio::test]
async fn suspension_notice_lifecycle_and_no_enforcement() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());

    let slug_a = format!("susp-a-{}", Uuid::now_v7().simple());
    let (owner_a, org_a) = register_org(&app, &state, &slug_a).await;
    let slug_b = format!("susp-b-{}", Uuid::now_v7().simple());
    let (owner_b, org_b) = register_org(&app, &state, &slug_b).await;

    let status_a = format!("/api/v1/orgs/{org_a}/suspension-status");
    let profile_a = "/api/org";

    // --- initial state: not flagged, both links always present --------------
    let response = send(
        &app,
        axum::http::Method::GET,
        &status_a,
        &bearer(&owner_a),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let view = body_json(response).await;
    assert_eq!(view["org_id"], json!(org_a.to_string()));
    assert!(view["suspended_at"].is_null());
    assert!(view["reason"].is_null());
    assert!(view["notify_after"].is_null());
    assert_eq!(view["data_export_url"], json!("/api/v1/org/exports"));
    assert_eq!(
        view["docs_url"],
        json!("https://getmarula.com/docs/how-to/export-data.html")
    );

    // --- a human MEMBER reads their own state but cannot write it -----------
    let invite = send(
        &app,
        axum::http::Method::POST,
        "/api/org/users",
        &bearer(&owner_a),
        Some(json!({
            "email": format!("tech@{slug_a}.test"),
            "display_name": "Tech",
            "role": "technician",
        })),
    )
    .await;
    assert_eq!(invite.status(), 200, "invite must succeed");
    let setup_token = body_json(invite).await["invite"]["setup_token"]
        .as_str()
        .expect("setup token")
        .to_string();
    let accept = send(
        &app,
        axum::http::Method::POST,
        "/api/org/users/accept-invite",
        &[],
        Some(json!({ "token": setup_token, "password": "another-correct-horse" })),
    )
    .await;
    assert_eq!(accept.status(), 200);
    let tech = body_json(accept).await["access_token"]
        .as_str()
        .expect("tech token")
        .to_string();

    let response = send(
        &app,
        axum::http::Method::GET,
        &status_a,
        &bearer(&tech),
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "a member reads their own state");
    let refused = send(
        &app,
        axum::http::Method::PATCH,
        &status_a,
        &bearer(&tech),
        Some(json!({ "suspended": true, "reason": "abuse" })),
    )
    .await;
    assert_eq!(refused.status(), 403, "a non-admin cannot flag the org");

    // --- validation: the reason is required and closed ----------------------
    for body in [
        json!({ "suspended": true }),
        json!({ "suspended": true, "reason": "late_payment" }),
    ] {
        let response = send(
            &app,
            axum::http::Method::PATCH,
            &status_a,
            &bearer(&owner_a),
            Some(body.clone()),
        )
        .await;
        assert_eq!(
            response.status(),
            422,
            "a bad reason body must be typed validation: {body}"
        );
    }

    // --- flag it: reason + a one-week notice interval -----------------------
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &status_a,
        &bearer(&owner_a),
        Some(json!({
            "suspended": true,
            "reason": "billing_lapsed",
            "notify_after": 604800,
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "owner flags the org");
    let flagged = body_json(response).await;
    assert!(flagged["suspended_at"].is_string());
    assert_eq!(flagged["reason"], json!("billing_lapsed"));
    assert_eq!(flagged["notify_after"], json!(604800));

    // --- idempotent set: a retry keeps the original timestamp ---------------
    let first_at = flagged["suspended_at"].clone();
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &status_a,
        &bearer(&owner_a),
        Some(json!({
            "suspended": true,
            "reason": "abuse",
            "notify_after": 86400,
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let again = body_json(response).await;
    assert_eq!(
        again["suspended_at"], first_at,
        "re-flagging must not restart the countdown"
    );
    assert_eq!(again["reason"], json!("abuse"), "the reason is refreshed");
    assert_eq!(again["notify_after"], json!(86400));

    // --- THE LOAD-BEARING RULE: a flagged org keeps working -----------------
    let read = send(
        &app,
        axum::http::Method::GET,
        profile_a,
        &bearer(&owner_a),
        None,
    )
    .await;
    assert_eq!(
        read.status(),
        200,
        "a flagged org's own reads are untouched — the flag is a notice, not a block"
    );
    let usage = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/orgs/{org_a}/usage"),
        &bearer(&owner_a),
        None,
    )
    .await;
    assert_eq!(usage.status(), 200, "monitoring reads keep working");
    let write = send(
        &app,
        axum::http::Method::PATCH,
        profile_a,
        &bearer(&owner_a),
        Some(json!({ "name": "Notice Co (renamed while flagged)" })),
    )
    .await;
    assert_eq!(
        write.status(),
        200,
        "a flagged org's writes are untouched — nothing in the core gates on the flag"
    );

    // --- the change emitted the registered contract event ------------------
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox_events \
          WHERE org_id = $1 AND event_type = 'org.suspension_changed'",
    )
    .bind(org_a)
    .fetch_one(&state.pool)
    .await
    .expect("outbox count");
    assert!(
        events >= 2,
        "flag + re-flag each emit org.suspension_changed, saw {events}"
    );
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_events \
          WHERE org_id = $1 AND action = 'org.suspension_flagged'",
    )
    .bind(org_a)
    .fetch_one(&state.pool)
    .await
    .expect("audit count");
    assert_eq!(audits, 2, "both flags are audited");

    // --- cross-org is a 404 for BOTH read and write -------------------------
    let status_b = format!("/api/v1/orgs/{org_b}/suspension-status");
    let response = send(
        &app,
        axum::http::Method::GET,
        &status_b,
        &bearer(&owner_a),
        None,
    )
    .await;
    assert_eq!(response.status(), 404, "another org's state is a 404");
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &status_b,
        &bearer(&owner_a),
        Some(json!({ "suspended": false })),
    )
    .await;
    assert_eq!(response.status(), 404, "writing another org is a 404");
    let _ = (owner_b, org_b);

    // --- service accounts: the read grant, a wrong scope, no write path ----
    let key_resp = send(
        &app,
        axum::http::Method::POST,
        "/api/api-keys",
        &bearer(&owner_a),
        Some(json!({ "name": "banner", "scopes": ["org_suspension:read"] })),
    )
    .await;
    assert_eq!(key_resp.status(), 201, "the read scope is creatable");
    let read_key = body_json(key_resp).await["raw_key"]
        .as_str()
        .expect("raw key")
        .to_string();
    let response = send(
        &app,
        axum::http::Method::GET,
        &status_a,
        &[("x-api-key", read_key.clone())],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "a read key reads the notice state");
    let refused = send(
        &app,
        axum::http::Method::PATCH,
        &status_a,
        &[("x-api-key", read_key.clone())],
        // A real body, so the refusal below is the PERMISSION gate and not
        // the `Json` extractor rejecting an empty PATCH before the handler.
        Some(json!({ "suspended": true, "reason": "billing_lapsed" })),
    )
    .await;
    assert_eq!(
        refused.status(),
        403,
        "there is no machine manage grant — setting the flag is an admin action"
    );

    let wrong = send(
        &app,
        axum::http::Method::POST,
        "/api/api-keys",
        &bearer(&owner_a),
        Some(json!({ "name": "wrong", "scopes": ["tickets:read"] })),
    )
    .await;
    let wrong_key = body_json(wrong).await["raw_key"]
        .as_str()
        .expect("raw key")
        .to_string();
    let response = send(
        &app,
        axum::http::Method::GET,
        &status_a,
        &[("x-api-key", wrong_key)],
        None,
    )
    .await;
    assert_eq!(response.status(), 403, "a key without the scope is refused");

    // --- clear ---------------------------------------------------------------
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &status_a,
        &bearer(&owner_a),
        Some(json!({ "suspended": false })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let cleared = body_json(response).await;
    assert!(cleared["suspended_at"].is_null());
    assert!(cleared["reason"].is_null());
    assert!(cleared["notify_after"].is_null());
    assert_eq!(
        cleared["data_export_url"],
        json!("/api/v1/org/exports"),
        "the links survive a clear"
    );

    let response = send(
        &app,
        axum::http::Method::GET,
        &status_a,
        &bearer(&owner_a),
        None,
    )
    .await;
    let view = body_json(response).await;
    assert!(view["suspended_at"].is_null());
    assert!(view["reason"].is_null());
}
