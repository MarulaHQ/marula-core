//! MCP adapter end-to-end (the private build tracker Phase 3, lane `p9-mcp-v1`): the REAL
//! committed OpenAPI snapshot drives the catalog; a stub core-api serves
//! the capability registry and one read endpoint; the MCP protocol runs
//! over the HTTP transport exactly as the server binary mounts it.
//!
//! Pins:
//! 1. The catalog derives mechanically from the snapshot: GET-only tools,
//!    capability slugs from the first path segment, sensitive prefixes
//!    (attachments/exports/api-keys/...) never enter the catalog.
//! 2. Deny-by-default capability filtering: only granted namespaces
//!    surface in `tools/list`; ungranted `tools/call` is a typed
//!    capability-denied error BEFORE any proxying.
//! 3. `tools/call` proxies to core-api with the caller's credentials
//!    (defense in depth: core-api re-checks scopes + RLS).

use axum::{Json, Router};
use mcp_server::catalog::build_catalog;
use mcp_server::http::router;
use mcp_server::protocol::{RequestId, RpcRequest};
use mcp_server::tools::{CoreApiBackend, ScopedBackend};
use mcp_server::Server;
use serde_json::{json, Value};
use tower::ServiceExt;

use axum::response::IntoResponse;

/// A tiny stub of the core-api surface the adapter proxies onto.
///
/// The proxied read routes REQUIRE `x-api-key` — exactly what real
/// core-api's require_auth does. This is the pin that keeps the proxy
/// authenticated: `tools/call` must carry the service key on every
/// read (mcp_flow regression: the proxy once sent none, and real
/// core-api answered 401 to every tool call).
fn stub_core_api() -> Router {
    const KEY: &str = "service-key";
    let proxied = |headers: axum::http::HeaderMap,
                   respond: axum::response::Response|
     -> axum::response::Response {
        let present = headers
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|k| k == KEY);
        if present {
            respond
        } else {
            axum::http::StatusCode::UNAUTHORIZED.into_response()
        }
    };
    Router::new()
        .route(
            "/api/v1/mcp/capabilities",
            axum::routing::get(move |headers: axum::http::HeaderMap| async move {
                let present = headers
                    .get("x-api-key")
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|k| k == KEY);
                if !present {
                    return axum::http::StatusCode::UNAUTHORIZED.into_response();
                }
                Json(json!({
                    "grants": [
                        { "capability": "clients:read", "mode": "proposed" },
                        { "capability": "tickets:read", "mode": "proposed" }
                    ]
                }))
                .into_response()
            }),
        )
        .route(
            "/api/clients/{id}",
            axum::routing::get(move |headers: axum::http::HeaderMap,
                                     path: axum::extract::Path<String>| async move {
                proxied(
                    headers,
                    Json(json!({ "id": path.0, "name": "Acme Manufacturing" })).into_response(),
                )
            }),
        )
        .route(
            "/api/clients",
            axum::routing::get(move |headers: axum::http::HeaderMap| async move {
                proxied(
                    headers,
                    Json(json!({ "clients": [], "next_cursor": null })).into_response(),
                )
            }),
        )
        .route(
            "/api/tickets",
            axum::routing::get(move |headers: axum::http::HeaderMap| async move {
                proxied(
                    headers,
                    Json(json!({ "tickets": [], "next_cursor": null })).into_response(),
                )
            }),
        )
}

async fn mcp_app() -> Router {
    let stub = stub_core_api();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, stub).await.unwrap() });

    let backend = CoreApiBackend::new(&format!("http://{addr}")).with_service_key("service-key");
    // The registry read itself must go through the stub.
    let granted = backend.granted_capabilities("service-key").await;
    assert!(
        granted.contains(&"clients".to_string()),
        "stub registry grants clients"
    );
    let granted_set: std::collections::HashSet<String> = granted.into_iter().collect();
    let scoped = ScopedBackend::new(backend, granted_set.clone());
    let spec = mcp_server::openapi_spec();
    let tools = build_catalog(&spec, &mcp_server::mcp_catalog_spec());
    let tools = mcp_server::catalog::filter_granted(tools, &granted_set);
    router(Server::new(scoped, tools))
}

async fn rpc(app: &Router, id: i64, method: &str, params: Value) -> Value {
    let request = RpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(RequestId::Number(id)),
        method: method.into(),
        params,
    };
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method(axum::http::Method::POST)
                .uri("/mcp")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    serde_json::to_string(&request).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "rpc must return 200");
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap()
}

// ===========================================================================
// Catalog derivation from the committed snapshot
// ===========================================================================

#[test]
fn catalog_is_get_only_and_excludes_sensitive_prefixes() {
    let spec = mcp_server::openapi_spec();
    let tools = build_catalog(&spec, &mcp_server::mcp_catalog_spec());
    assert!(
        tools.len() > 20,
        "the real snapshot yields a meaningful read catalog (got {})",
        tools.len()
    );
    assert!(tools.iter().all(|t| t.method == "GET"));
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(
        names.contains(&"tickets_list"),
        "canonical read tool present"
    );
    assert!(names.contains(&"clients_get_one"));
    assert!(
        !names.iter().any(|n| n.starts_with("attachments_")),
        "attachment downloads never enter the agent catalog"
    );
    assert!(
        !names.iter().any(|n| n.starts_with("api_keys_")),
        "key management never enters the agent catalog"
    );
    assert!(
        !names.iter().any(|n| n.starts_with("org_exports")),
        "export bundles never enter the agent catalog"
    );
    // Unique names, unique stable ordering.
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(names, sorted, "operationIds are unique and sorted");
}

#[test]
fn capabilities_map_from_path_namespaces() {
    let spec = mcp_server::openapi_spec();
    let tools = build_catalog(&spec, &mcp_server::mcp_catalog_spec());
    let tickets = tools.iter().find(|t| t.name == "tickets_list").unwrap();
    assert_eq!(tickets.capability, "tickets:read");
    let clients = tools.iter().find(|t| t.name == "clients_list").unwrap();
    assert_eq!(clients.capability, "clients:read");
}

// ===========================================================================
// Protocol flow over the HTTP transport
// ===========================================================================

#[tokio::test]
async fn initialize_and_tools_list_honor_deny_by_default() {
    let app = mcp_app().await;

    let hello = rpc(
        &app,
        1,
        "initialize",
        json!({ "protocolVersion": "2025-03-26" }),
    )
    .await;
    assert_eq!(hello["result"]["serverInfo"]["name"], "marula-mcp");
    assert_eq!(hello["result"]["protocolVersion"], "2025-03-26");

    // The stub registry grants clients + tickets only; every OTHER
    // namespace must be absent from tools/list (deny-by-default).
    let listed = rpc(&app, 2, "tools/list", json!({})).await;
    let tools = listed["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(!names.is_empty(), "granted tools are listed");
    assert!(
        names.iter().all(|n| {
            !(n.contains("invoices")
                || n.contains("documents")
                || n.contains("projects")
                || n.contains("reports")
                || n.contains("contracts")
                || n.contains("search")
                || n.contains("kb_"))
        }),
        "ungranted namespaces are hidden, got: {names:?}"
    );
    // Each listed tool carries a JSON-Schema input object.
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object");
    }
}

#[tokio::test]
async fn tools_call_proxies_granted_reads_and_denies_ungranted() {
    let app = mcp_app().await;

    let call = rpc(
        &app,
        3,
        "tools/call",
        json!({ "name": "clients_get_one", "arguments": { "id": "42" } }),
    )
    .await;
    let text = call["result"]["content"][0]["text"].as_str().unwrap();
    let body: Value = serde_json::from_str(text).unwrap();
    assert_eq!(body["name"], "Acme Manufacturing");
    assert_eq!(body["id"], "42");
    assert_eq!(call["result"]["isError"], false);

    // tickets_list IS granted in the stub registry — proxied fine.
    let call = rpc(
        &app,
        4,
        "tools/call",
        json!({ "name": "tickets_list", "arguments": { "limit": 5 } }),
    )
    .await;
    assert!(call["result"]["content"][0]["text"].is_string());

    // An ungranted namespace is not merely denied at call time — it was
    // never listed: tools/call on it is a typed method-not-found (the
    // scoped-backend capability denial is the defense-in-depth layer for
    // grants shrinking between list and call).
    let ungranted = rpc(
        &app,
        5,
        "tools/call",
        json!({ "name": "documents_list", "arguments": {} }),
    )
    .await;
    assert_eq!(
        ungranted["error"]["code"],
        mcp_server::protocol::CODE_METHOD_NOT_FOUND,
        "ungranted tools are unlisted: {ungranted}"
    );

    // Unknown tool names are a typed method-not-found too.
    let unknown = rpc(&app, 6, "tools/call", json!({ "name": "nope" })).await;
    assert_eq!(
        unknown["error"]["code"],
        mcp_server::protocol::CODE_METHOD_NOT_FOUND
    );
}

#[tokio::test]
async fn notifications_get_no_body_and_unknown_methods_error() {
    let app = mcp_app().await;
    let request = RpcRequest {
        jsonrpc: "2.0".into(),
        id: None,
        method: "notifications/initialized".into(),
        params: json!({}),
    };
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method(axum::http::Method::POST)
                .uri("/mcp")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    serde_json::to_string(&request).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 204, "notification ⇒ 204 no body");

    let bogus = rpc(&app, 7, "admin/secret", json!({})).await;
    assert_eq!(
        bogus["error"]["code"],
        mcp_server::protocol::CODE_METHOD_NOT_FOUND
    );
}
