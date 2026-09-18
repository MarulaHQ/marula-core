//! Instant-first write-path contract tests (TASKLIST IF1–IF3), driven
//! through the REAL core-api router against a real Postgres.
//!
//! Purpose: pin the binding contract of `docs/instant-first.md` over HTTP:
//!
//! * IF1 idempotent autosave writes — the same idempotency key with the
//!   same payload replays the byte-identical result and never double-
//!   applies; the same key with a different payload is a typed 409; the
//!   If-Match-style version gate rejects stale writes with a typed 409
//!   naming `expected`/`provided`.
//! * IF2 read-your-write response — every mutation result carries the
//!   additive `mutation` metadata: the applied snapshot, `updated_at`, a
//!   monotonic per-entity `seq` (mirrored as `version`), and the prior +
//!   new values that make undo a core capability.
//! * IF3 typed error envelope — not-found, authz, RLS-sparse, validation,
//!   and version/conflict classes come back as machine-readable `reason`
//!   codes (plus the legacy `code`/`message` keys, unchanged).
//!
//! Non-purpose: seq allocation internals under concurrency (the counter's
//! row lock is exercised by core-db), offline replay buffering (client
//! side), and the undo read model (IF7).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn test_state() -> (AppState, Router) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("instant-first e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    core_db::assert_rls_coverage(&pool)
        .await
        .expect("RLS coverage holds with instant-first surfaces");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    let state = AppState {
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&reporting_url)
            .expect("lazy reporting pool"),
    };
    let app = core_api::routes::build_router(state.clone());
    (state, app)
}

async fn connect_admin(url: &str) -> Result<PgPool, sqlx::Error> {
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
    bearer: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder()
        .method(method.clone())
        .uri(uri);
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
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
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!("body_json parse failed ({e}); last SEND lines identify the endpoint")
    })
}

struct Org {
    token: String,
    org_id: Uuid,
}

async fn register_org(app: &Router, slug: String) -> Org {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Instant Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let tokens = body_json(response).await;
    let token = tokens["access_token"].as_str().unwrap().to_string();
    let me =
        body_json(send(app, axum::http::Method::GET, "/api/me", Some(&token), None).await).await;
    Org {
        token,
        org_id: me["org_id"].as_str().unwrap().parse().unwrap(),
    }
}

/// Inserts a same-org user with NO role (hence no permissions) directly
/// through the admin pool and logs them in for a bearer token.
async fn add_roleless_user(app: &Router, state: &AppState, org: &Org, label: &str) -> String {
    let email = format!("{label}@instant.test");
    core_db::users::create_user(
        &state.pool,
        Uuid::now_v7(),
        org.org_id,
        &email,
        label,
        &core_api::auth::password::hash("correct-horse-battery").unwrap(),
        "UTC",
    )
    .await
    .expect("seed roleless user");
    let login = body_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/auth/login",
            None,
            Some(json!({"email": email, "password": "correct-horse-battery"})),
        )
        .await,
    )
    .await;
    login["access_token"].as_str().unwrap().to_string()
}

async fn default_board(app: &Router, org: &Org) -> Uuid {
    let nav = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/boards/nav",
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    nav[0]["id"]
        .as_str()
        .unwrap()
        .parse()
        .expect("default board id")
}

async fn create_ticket(app: &Router, org: &Org, board: Uuid, title: &str, extra: Value) -> Value {
    let mut body = json!({
        "board_id": board,
        "title": title,
        "priority": "p3",
    });
    if let (Some(dst), Some(src)) = (body.as_object_mut(), extra.as_object()) {
        for (k, v) in src {
            dst.insert(k.clone(), v.clone());
        }
    }
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/tickets",
        Some(&org.token),
        Some(body),
    )
    .await;
    assert_eq!(response.status(), 200, "ticket create must succeed");
    body_json(response).await
}

// ===========================================================================
// IF2 — read-your-write response shape + seq monotonicity
// ===========================================================================

#[tokio::test]
async fn mutation_response_carries_mutation_meta_and_seq_is_monotonic() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("if-meta-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;

    // Create: seq starts at 1, before is null, after mirrors the entity.
    let created = create_ticket(&app, &org, board, "Autosaved title", json!({})).await;
    assert_eq!(
        created["title"], "Autosaved title",
        "entity fields stay top-level"
    );
    assert_eq!(
        created["custom_fields"],
        json!([]),
        "legacy payload keys intact"
    );
    let ticket_id: Uuid = created["id"].as_str().unwrap().parse().unwrap();
    let mutation = &created["mutation"];
    assert_eq!(mutation["seq"], 1, "first write on a fresh entity is seq 1");
    assert_eq!(mutation["version"], 1, "version mirrors seq");
    assert!(
        mutation["updated_at"].as_str().is_some(),
        "updated_at present"
    );
    assert!(mutation["before"].is_null(), "create has no prior value");
    assert_eq!(mutation["after"]["title"], "Autosaved title");
    assert_eq!(mutation["idempotency_key"], Value::Null, "no key supplied");
    // Additivity is exact: the envelope adds ONE object with these keys.
    let mut meta_keys: Vec<&str> = mutation
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    meta_keys.sort();
    assert_eq!(
        meta_keys,
        vec![
            "after",
            "before",
            "idempotency_key",
            "seq",
            "updated_at",
            "version"
        ]
    );

    // Patch: seq advances monotonically; before/after carry prior + new.
    let patched = body_json(
        send(
            &app,
            axum::http::Method::PATCH,
            &format!("/api/tickets/{ticket_id}"),
            Some(&org.token),
            Some(json!({ "title": "Second autosave" })),
        )
        .await,
    )
    .await;
    assert_eq!(patched["title"], "Second autosave");
    let seq2 = patched["mutation"]["seq"].as_i64().unwrap();
    assert_eq!(seq2, 2, "seq strictly increases per applied write");
    assert_eq!(patched["mutation"]["before"]["title"], "Autosaved title");
    assert_eq!(patched["mutation"]["after"]["title"], "Second autosave");
    assert_eq!(
        patched["mutation"]["after"]["id"], created["mutation"]["after"]["id"],
        "after is the full entity snapshot"
    );

    // Third write with an explicit version echo succeeds and advances again.
    let patched2 = body_json(
        send(
            &app,
            axum::http::Method::PATCH,
            &format!("/api/tickets/{ticket_id}"),
            Some(&org.token),
            Some(json!({ "title": "Third autosave", "version": seq2 })),
        )
        .await,
    )
    .await;
    assert_eq!(patched2["mutation"]["seq"], 3);
    assert_eq!(patched2["mutation"]["version"], 3);
    assert_eq!(patched2["mutation"]["before"]["title"], "Second autosave");
}

#[tokio::test]
async fn comment_and_status_mutations_carry_the_envelope() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("if-comment-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, "Comment target", json!({})).await;
    let ticket_id: Uuid = ticket["id"].as_str().unwrap().parse().unwrap();
    let current_status: Uuid = ticket["status_id"].as_str().unwrap().parse().unwrap();

    let comment = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/tickets/{ticket_id}/comments"),
            Some(&org.token),
            Some(json!({ "body": "note body", "kind": "internal" })),
        )
        .await,
    )
    .await;
    assert_eq!(
        comment["body"], "note body",
        "comment fields stay top-level"
    );
    assert_eq!(
        comment["mutation"]["seq"], 1,
        "per-entity seq: the comment's own"
    );
    assert!(comment["mutation"]["before"].is_null());
    assert_eq!(comment["mutation"]["after"]["body"], "note body");

    let target_status_id = next_status_id(&app, &org, board, current_status).await;

    let status = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/tickets/{ticket_id}/status"),
            Some(&org.token),
            Some(json!({ "status_id": target_status_id })),
        )
        .await,
    )
    .await;
    assert_eq!(status["id"], ticket["id"], "ticket fields stay top-level");
    assert_eq!(
        status["mutation"]["seq"], 2,
        "the status change is the ticket's second write"
    );
    assert_eq!(
        status["mutation"]["before"]["status_id"], ticket["status_id"],
        "prior value carries the old status id"
    );
}

/// The board's first status other than the ticket's current one (the
/// default seed board exposes the standard lifecycle; with no explicit
/// transition matrix the semantic-category floor governs).
async fn next_status_id(app: &Router, org: &Org, board: Uuid, current: Uuid) -> Uuid {
    let statuses = body_json(
        send(
            app,
            axum::http::Method::GET,
            &format!("/api/boards/{board}/statuses"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    statuses
        .as_array()
        .unwrap()
        .iter()
        .find(|s| {
            s["id"]
                .as_str()
                .map(|id| id != current.to_string())
                .unwrap_or(false)
        })
        .map(|s| s["id"].as_str().unwrap().parse().unwrap())
        .expect("board has another status")
}

#[tokio::test]
async fn checklist_mutations_carry_the_envelope_and_replay() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("if-checklist-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, "Checklist target", json!({})).await;
    let ticket_id: Uuid = ticket["id"].as_str().unwrap().parse().unwrap();

    // Create an ad-hoc item: the checklist view stays top-level (additive
    // envelope) and `mutation.after` is the created item.
    let added = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/tickets/{ticket_id}/checklist"),
            Some(&org.token),
            Some(json!({ "text": "Verify power", "idempotency_key": "cl-add-1" })),
        )
        .await,
    )
    .await;
    assert_eq!(
        added["items"].as_array().map(Vec::len),
        Some(1),
        "view fields stay top-level"
    );
    let mutation = added["mutation"].clone();
    assert!(mutation.is_object(), "add carries the mutation envelope");
    assert!(mutation["before"].is_null(), "create has no prior value");
    assert_eq!(mutation["after"]["text"], "Verify power");
    assert_eq!(mutation["after"]["checked"], false);
    assert_eq!(
        added["items"][0]["id"], mutation["after"]["id"],
        "the applied item is in the returned view"
    );
    let item_id: Uuid = mutation["after"]["id"].as_str().unwrap().parse().unwrap();
    let seq1 = mutation["seq"].as_i64().expect("seq present");

    // Same key + same payload replays the identical stored result.
    let replayed = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/tickets/{ticket_id}/checklist"),
            Some(&org.token),
            Some(json!({ "text": "Verify power", "idempotency_key": "cl-add-1" })),
        )
        .await,
    )
    .await;
    assert_eq!(
        replayed, added,
        "replay returns the same stored result without double-apply"
    );

    // Toggle with the version echo: before/after carry the item's prior and
    // new checked state, and the item seq advances monotonically.
    let toggled = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/tickets/{ticket_id}/checklist/{item_id}"),
            Some(&org.token),
            Some(json!({ "checked": true, "version": seq1 })),
        )
        .await,
    )
    .await;
    assert_eq!(toggled["mutation"]["before"]["checked"], false);
    assert_eq!(toggled["mutation"]["after"]["checked"], true);
    assert_eq!(
        toggled["mutation"]["after"]["id"], mutation["after"]["id"],
        "per-entity seq family is the item itself"
    );
    assert!(
        toggled["mutation"]["seq"].as_i64().unwrap() > seq1,
        "seq strictly increases per applied write"
    );
}

// ===========================================================================
// IF1 — idempotency replay + reuse rejection
// ===========================================================================

#[tokio::test]
async fn same_key_same_payload_replays_identical_result_without_double_apply() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("if-replay-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let key = Uuid::now_v7().to_string();

    let first = create_ticket(
        &app,
        &org,
        board,
        "Debounced write",
        json!({ "idempotency_key": key }),
    )
    .await;
    assert_eq!(first["mutation"]["idempotency_key"], key);

    // The retry (debounce/retry episode): identical key + payload.
    let retry = create_ticket(
        &app,
        &org,
        board,
        "Debounced write",
        json!({ "idempotency_key": key }),
    )
    .await;
    assert_eq!(
        first, retry,
        "replay must be byte-identical to the stored result"
    );

    // And nothing double-applied.
    let list = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/tickets?limit=200",
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    let matches = list
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["title"] == "Debounced write")
        .count();
    assert_eq!(matches, 1, "the retried create must not double-apply");

    // Update replay: same key + same patch body returns the SAME result,
    // even though the entity moved on in between.
    let ticket_id: Uuid = first["id"].as_str().unwrap().parse().unwrap();
    let patch_body =
        json!({ "title": "Patched once", "idempotency_key": Uuid::now_v7().to_string() });
    let patch1 = body_json(
        send(
            &app,
            axum::http::Method::PATCH,
            &format!("/api/tickets/{ticket_id}"),
            Some(&org.token),
            Some(patch_body.clone()),
        )
        .await,
    )
    .await;
    // A different write lands in between (entity moves on).
    let _ = body_json(
        send(
            &app,
            axum::http::Method::PATCH,
            &format!("/api/tickets/{ticket_id}"),
            Some(&org.token),
            Some(json!({ "priority": "p4" })),
        )
        .await,
    )
    .await;
    let patch_replay = body_json(
        send(
            &app,
            axum::http::Method::PATCH,
            &format!("/api/tickets/{ticket_id}"),
            Some(&org.token),
            Some(patch_body),
        )
        .await,
    )
    .await;
    assert_eq!(
        patch1, patch_replay,
        "update replay returns the stored result, not a re-application"
    );
    let final_ticket = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/tickets/{ticket_id}"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        final_ticket["priority"], "p4",
        "the intermediate write survived the replay"
    );
}

#[tokio::test]
async fn same_key_different_payload_is_typed_conflict() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("if-reuse-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let key = Uuid::now_v7().to_string();

    let _ = create_ticket(
        &app,
        &org,
        board,
        "Original payload",
        json!({ "idempotency_key": key }),
    )
    .await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/tickets",
        Some(&org.token),
        Some(json!({
            "board_id": board,
            "title": "Different payload, same key",
            "priority": "p3",
            "idempotency_key": key,
        })),
    )
    .await;
    assert_eq!(response.status(), 409);
    let body = body_json(response).await;
    assert_eq!(body["error"]["code"], 409, "legacy code key intact");
    assert_eq!(
        body["error"]["reason"], "idempotency_key_reused",
        "typed machine-readable reason"
    );
    assert!(
        body["error"]["message"].as_str().is_some(),
        "message stays for humans"
    );

    // A key claimed against one entity family cannot replay on another.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/clients",
        Some(&org.token),
        Some(json!({ "name": "Cross-family", "idempotency_key": key })),
    )
    .await;
    assert_eq!(response.status(), 409);
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "idempotency_key_reused");
}

// ===========================================================================
// IF1 — If-Match-style version gate
// ===========================================================================

#[tokio::test]
async fn stale_version_is_typed_409_naming_expected_and_provided() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("if-version-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, "Versioned", json!({})).await;
    let ticket_id: Uuid = ticket["id"].as_str().unwrap().parse().unwrap();
    let current = ticket["mutation"]["version"].as_i64().unwrap();

    // A concurrent write moves the entity forward.
    let moved = body_json(
        send(
            &app,
            axum::http::Method::PATCH,
            &format!("/api/tickets/{ticket_id}"),
            Some(&org.token),
            Some(json!({ "title": "Moved by someone else" })),
        )
        .await,
    )
    .await;
    assert_eq!(moved["mutation"]["version"], current + 1);

    // The stale writer presents the version it last saw: rejected BEFORE
    // anything changes, with a typed machine-readable body.
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/tickets/{ticket_id}"),
        Some(&org.token),
        Some(json!({ "title": "Stale write", "version": current })),
    )
    .await;
    assert_eq!(response.status(), 409);
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "version_conflict");
    assert_eq!(body["error"]["expected"], current + 1);
    assert_eq!(body["error"]["provided"], current);
    assert_eq!(body["error"]["code"], 409);

    // The rejected write changed nothing.
    let after = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/tickets/{ticket_id}"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(after["title"], "Moved by someone else");

    // A version presented for a fresh entity can only ever be 0.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/tickets",
        Some(&org.token),
        Some(json!({
            "board_id": board,
            "title": "Create with bogus version",
            "priority": "p3",
            "version": 7,
        })),
    )
    .await;
    assert_eq!(response.status(), 409);
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "version_conflict");
    assert_eq!(body["error"]["expected"], 0);
    assert_eq!(body["error"]["provided"], 7);
}

// ===========================================================================
// IF3 — typed envelope codes for every error class
// ===========================================================================

#[tokio::test]
async fn envelope_codes_for_each_error_class() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("if-envelope-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;

    // A second org holds the ticket the first org will probe: the write
    // target is invisible under the caller's row-level scope.
    let other_org = register_org(&app, format!("if-envelope-b-{}", Uuid::now_v7())).await;
    let other_board = default_board(&app, &other_org).await;
    let foreign = create_ticket(&app, &other_org, other_board, "Not yours", json!({})).await;
    let foreign_ticket_id: Uuid = foreign["id"].as_str().unwrap().parse().unwrap();

    // unauthorized: no bearer at all.
    let response = send(&app, axum::http::Method::GET, "/api/tickets", None, None).await;
    assert_eq!(response.status(), 401);
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "unauthorized");
    assert_eq!(body["error"]["code"], 401);

    // forbidden: a same-org user with no permissions.
    let roleless = add_roleless_user(&app, &state, &org, "roleless").await;
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/tickets",
        Some(&roleless),
        Some(json!({
            "board_id": board,
            "title": "Should not land",
            "priority": "p3",
        })),
    )
    .await;
    assert_eq!(response.status(), 403);
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "forbidden");

    // validation_failed: 422 with a machine code, not prose parsing.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/tickets",
        Some(&org.token),
        Some(json!({
            "board_id": board,
            "title": "   ",
            "priority": "p3",
        })),
    )
    .await;
    assert_eq!(response.status(), 422);
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "validation_failed");

    // not_found: a read miss keeps the classic code.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/tickets/{}", Uuid::now_v7()),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 404);
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "not_found");

    // rls_sparse: a MUTATION whose target is invisible under the caller's
    // row-level scope (here: another org's ticket) is a distinct write-path
    // code so clients reconcile by resync, not by re-POSTing.
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/tickets/{foreign_ticket_id}"),
        Some(&org.token),
        Some(json!({ "title": "Cross-org probe" })),
    )
    .await;
    assert_eq!(response.status(), 404);
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "rls_sparse");
    assert_eq!(body["error"]["code"], 404);
}

#[tokio::test]
async fn write_trail_records_prior_and_new_values() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("if-trail-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, "Trail", json!({})).await;
    let ticket_id: Uuid = ticket["id"].as_str().unwrap().parse().unwrap();

    let _ = body_json(
        send(
            &app,
            axum::http::Method::PATCH,
            &format!("/api/tickets/{ticket_id}"),
            Some(&org.token),
            Some(json!({ "title": "Trail v2" })),
        )
        .await,
    )
    .await;

    // The append-only trail (undo substrate) holds create + update with
    // the exact before/after snapshots the responses advertised.
    let rows: Vec<(i64, String, Option<Value>, Value)> = sqlx::query_as(
        "SELECT seq, op, before, after FROM entity_writes \
         WHERE org_id = $1 AND entity = 'ticket' AND entity_id = $2 ORDER BY seq",
    )
    .bind(org.org_id)
    .bind(ticket_id)
    .fetch_all(&state.pool)
    .await
    .expect("trail rows");
    assert_eq!(rows.len(), 2, "create + update trail rows");
    assert_eq!(rows[0].0, 1);
    assert_eq!(rows[0].1, "create");
    assert!(rows[0].2.is_none(), "create has no prior value");
    assert_eq!(rows[1].0, 2);
    assert_eq!(rows[1].1, "update");
    assert_eq!(rows[1].2.as_ref().unwrap()["title"], "Trail");
    assert_eq!(rows[1].3["title"], "Trail v2");

    // The seq counter matches the highest applied write.
    let seq: i64 = sqlx::query_scalar(
        "SELECT seq FROM entity_write_seqs WHERE org_id = $1 AND entity = 'ticket' AND entity_id = $2",
    )
    .bind(org.org_id)
    .bind(ticket_id)
    .fetch_one(&state.pool)
    .await
    .expect("counter row");
    assert_eq!(seq, 2);
}
