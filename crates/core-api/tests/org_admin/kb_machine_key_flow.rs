//! The docs connector reaches KB articles with a machine key (G9 follow-up).
//!
//! `dms-sync` pulls the article index, then creates and updates one article
//! per record. The handler gates were already right (`kb:create` / `kb:edit`,
//! which `rag:write` resolves onto); the auth middleware's path allowlist
//! refused the request before any handler ran, so the connector reported
//! "articles unavailable this run" instead of syncing.
//!
//! This suite proves the narrowing, not just the opening:
//!   * list, create and update work with a scoped key;
//!   * delete, category administration and unrelated subtrees still answer
//!     401 — the allowlist did not widen into `/api/kb`;
//!   * a read-scoped key gets a typed 403 on the write (an allowance is not a
//!     grant: the handler still decides).

use crate::common::{body_json, register_org_token, send, send_headers, test_state};
use axum::Router;
use serde_json::{json, Value};
use uuid::Uuid;

async fn mint_key(app: &Router, owner: &str, scopes: Value) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/api-keys",
        Some(owner),
        Some(json!({ "name": format!("docs-{}", Uuid::now_v7().simple()), "scopes": scopes })),
    )
    .await;
    assert_eq!(response.status(), 201, "the scope set must be mintable");
    body_json(response).await["raw_key"]
        .as_str()
        .expect("raw key")
        .to_string()
}

async fn as_key(
    app: &Router,
    method: axum::http::Method,
    uri: &str,
    key: &str,
    body: Option<Value>,
) -> axum::response::Response {
    send_headers(app, method, uri, &[("x-api-key", key.to_string())], body).await
}

#[tokio::test]
async fn a_machine_key_syncs_articles_and_nothing_else_under_kb() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("kb-key-{}", Uuid::now_v7().simple());
    let owner = register_org_token(&app, "KB Key Co", &slug).await;

    // `rag:write` is the mintable name for the KB write grants; the read half
    // is separate by design, and the connector needs both.
    let key = mint_key(&app, &owner, json!(["rag:read", "rag:write"])).await;

    // --- list, create, update ----------------------------------------------
    let response = as_key(
        &app,
        axum::http::Method::GET,
        "/api/v1/kb/articles",
        &key,
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "the connector pulls the index first"
    );

    let response = as_key(
        &app,
        axum::http::Method::POST,
        "/api/v1/kb/articles",
        &key,
        Some(json!({
            "title": "Rotation policy",
            "slug": "rotation-policy",
            "body": "Rotate credentials every quarter.",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "a key creates an article");
    let created = body_json(response).await;
    let id = created["id"].as_str().expect("article id").to_string();
    assert_eq!(created["slug"], "rotation-policy");

    let response = as_key(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/kb/articles/{id}"),
        &key,
        Some(json!({ "body": "Rotate credentials every 90 days." })),
    )
    .await;
    assert_eq!(response.status(), 200, "a key updates that article");

    // The legacy alias is the same handler stack, so the allowlist must accept
    // it identically.
    let response = as_key(
        &app,
        axum::http::Method::GET,
        "/api/kb/articles",
        &key,
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "legacy mount, same allowance");

    // --- the parts deliberately left closed --------------------------------
    let response = as_key(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/v1/kb/articles/{id}"),
        &key,
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        401,
        "deletion is not on the allowlist even though rag:write carries the grant"
    );

    let response = as_key(
        &app,
        axum::http::Method::POST,
        "/api/v1/kb/categories",
        &key,
        Some(json!({ "name": "Runbooks" })),
    )
    .await;
    assert_eq!(
        response.status(),
        401,
        "category administration stays closed"
    );

    let response = as_key(
        &app,
        axum::http::Method::GET,
        "/api/v1/kb/articles/export",
        &key,
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        401,
        "a child of the article path that is not an id is refused"
    );

    let response = as_key(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/users/{}/cost-rate", Uuid::now_v7()),
        &key,
        None,
    )
    .await;
    assert_eq!(response.status(), 401, "unrelated subtrees stay closed");

    // --- an allowance is not a grant ---------------------------------------
    let reader = mint_key(&app, &owner, json!(["rag:read"])).await;
    let response = as_key(
        &app,
        axum::http::Method::POST,
        "/api/v1/kb/articles",
        &reader,
        Some(json!({ "title": "Nope", "body": "reader key" })),
    )
    .await;
    assert_eq!(
        response.status(),
        403,
        "a read key reaches the handler and is refused by the matrix, not by the path gate"
    );

    // The article the write key created is still there and readable.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/kb/articles/{id}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        body_json(response).await["body"],
        "Rotate credentials every 90 days."
    );
}

#[tokio::test]
async fn the_kb_allowance_is_by_method_not_by_prefix() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("kb-key-method-{}", Uuid::now_v7().simple());
    let owner = register_org_token(&app, "KB Method Co", &slug).await;
    let key = mint_key(&app, &owner, json!(["rag:write"])).await;

    // POST on the collection is allowed; PUT on it is not a route at all, so
    // the middleware must not answer it with the allowance either.
    let response = as_key(
        &app,
        axum::http::Method::PUT,
        "/api/v1/kb/articles",
        &key,
        Some(json!({ "title": "x" })),
    )
    .await;
    assert_eq!(response.status(), 401, "PUT is not an allowed method");

    // A two-segment tail under the collection is not an article id.
    let response = as_key(
        &app,
        axum::http::Method::PATCH,
        "/api/v1/kb/articles/abc/embeds",
        &key,
        Some(json!({})),
    )
    .await;
    assert_eq!(response.status(), 401, "nested article paths stay closed");
}
