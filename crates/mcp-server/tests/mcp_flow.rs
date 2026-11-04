//! End-to-end contract for the adapter: a caller's own session is the only
//! credential, the org's grants decide which tools exist, and both protocol
//! eras get an answer.

use axum::{response::IntoResponse, routing::get, Json, Router};
use mcp_server::MarulaServer;
use serde_json::{json, Value};
use tower::ServiceExt;

const TOKEN: &str = "session-token";

fn bearer_ok(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == format!("Bearer {TOKEN}"))
}

/// A stub of the core-api surface the adapter reads and proxies onto. It
/// grants exactly one family, so a second family proves the deny path.
fn stub_core_api() -> Router {
    Router::new()
        .route(
            "/api/v1/me",
            get(|headers: axum::http::HeaderMap| async move {
                if !bearer_ok(&headers) {
                    return axum::http::StatusCode::UNAUTHORIZED.into_response();
                }
                Json(json!({
                    "user_id": "018f0a5e-0000-7000-8000-000000000001",
                    "org_id": "018f0a5e-0000-7000-8000-000000000002",
                    "email": "tech@example.test",
                    "display_name": "Tech",
                    "timezone": "UTC",
                    "permissions": ["clients:view"],
                    "mcp_capabilities": ["clients:read"]
                }))
                .into_response()
            }),
        )
        .route(
            "/api/clients",
            get(|headers: axum::http::HeaderMap| async move {
                if !bearer_ok(&headers) {
                    return axum::http::StatusCode::UNAUTHORIZED.into_response();
                }
                Json(json!({ "clients": [], "next_cursor": null })).into_response()
            }),
        )
}

async fn app() -> Router {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, stub_core_api()).await.unwrap() });
    let base = format!("http://{addr}");
    let server = MarulaServer::new(
        &base,
        &mcp_server::openapi_spec(),
        &mcp_server::mcp_catalog_spec(),
        None,
    );
    mcp_server::http::router(server)
}

async fn rpc(app: &Router, token: Option<&str>, body: Value) -> Value {
    let mut request = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("host", "127.0.0.1");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    // SEP-2243: a request carrying per-request protocol metadata must also
    // carry the version header, or the server refuses it.
    let encoded = serde_json::to_string(&body).unwrap();
    if encoded.contains("io.modelcontextprotocol/protocolVersion") {
        request = request.header("MCP-Protocol-Version", "2026-07-28");
        if let Some(method) = body.get("method").and_then(Value::as_str) {
            request = request.header("Mcp-Method", method);
        }
        if let Some(name) = body
            .get("params")
            .and_then(|params| params.get("name"))
            .and_then(Value::as_str)
        {
            request = request.header("Mcp-Name", name);
        }
    }
    let response = app
        .clone()
        .oneshot(request.body(axum::body::Body::from(encoded)).unwrap())
        .await
        .expect("router answers");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&bytes);
    serde_json::from_str(&text).unwrap_or_else(|_| {
        panic!("answer was not JSON ({status}): {text}");
    })
}

fn modern_meta(method: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": method,
        "params": { "_meta": {
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientInfo": { "name": "test", "version": "1" },
            "io.modelcontextprotocol/clientCapabilities": {}
        }}
    })
}

#[tokio::test]
async fn a_modern_client_discovers_and_lists_its_grants() {
    let app = app().await;
    let discovered = rpc(&app, Some(TOKEN), modern_meta("server/discover")).await;
    let result = &discovered["result"];
    assert!(
        result["supportedVersions"]
            .as_array()
            .is_some_and(|versions| versions.iter().any(|v| v == "2026-07-28")),
        "server advertises the revision it implements: {result}"
    );

    let listed = rpc(&app, Some(TOKEN), modern_meta("tools/list")).await;
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(names.contains(&"clients_list"), "granted family appears");
    assert!(
        !names.contains(&"tickets_list"),
        "an ungranted family does not appear: {names:?}"
    );
}

#[tokio::test]
async fn a_legacy_client_still_gets_the_same_catalog() {
    let app = app().await;
    let initialized = rpc(
        &app,
        Some(TOKEN),
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "legacy-test", "version": "1" }
            }
        }),
    )
    .await;
    let version = initialized["result"]["protocolVersion"]
        .as_str()
        .expect("a negotiated version");
    assert!(
        ["2025-11-25", "2026-07-28"].contains(&version),
        "negotiated a version the server implements, got {version}"
    );

    let listed = rpc(
        &app,
        Some(TOKEN),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }),
    )
    .await;
    assert!(listed["result"]["tools"]
        .as_array()
        .is_some_and(|tools| !tools.is_empty()));
}

#[tokio::test]
async fn a_session_without_a_token_sees_no_tools() {
    let app = app().await;
    let listed = rpc(&app, None, modern_meta("tools/list")).await;
    assert_eq!(
        listed["result"]["tools"].as_array().map(Vec::len),
        Some(0),
        "no session means no grants means no tools"
    );
}

#[tokio::test]
async fn a_call_forwards_the_callers_session_and_refuses_an_ungranted_family() {
    let app = app().await;
    let called = rpc(
        &app,
        Some(TOKEN),
        json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "clients_list", "arguments": {},
                "_meta": {
                    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                    "io.modelcontextprotocol/clientInfo": { "name": "test", "version": "1" },
                    "io.modelcontextprotocol/clientCapabilities": {}
                }
            }
        }),
    )
    .await;
    assert_eq!(called["result"]["isError"], json!(false), "{called}");
    let text = called["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    assert!(text.contains("\"clients\""), "proxied payload: {text}");

    let refused = rpc(
        &app,
        Some(TOKEN),
        json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "tickets_list", "arguments": {},
                "_meta": {
                    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                    "io.modelcontextprotocol/clientInfo": { "name": "test", "version": "1" },
                    "io.modelcontextprotocol/clientCapabilities": {}
                }
            }
        }),
    )
    .await;
    assert_eq!(refused["result"]["isError"], json!(true), "{refused}");
    let text = refused["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    assert!(text.contains("tickets:read"), "names the grant: {text}");
}
