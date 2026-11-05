//! End-to-end contract for the adapter: a caller's own session is the only
//! credential, the org's grants decide which tools, resources and prompts
//! exist, and both protocol eras get an answer.

use axum::{extract::Path, response::IntoResponse, routing::get, Json, Router};
use mcp_server::MarulaServer;
use serde_json::{json, Value};
use tower::ServiceExt;

const TOKEN: &str = "session-token";
const TICKET: &str = "018f0a5e-0000-7000-8000-0000000000aa";

/// The caller `/api/v1/me` describes: what the org granted, and what this
/// credential itself holds.
#[derive(Clone, Copy)]
struct Caller {
    grants: &'static [&'static str],
    permissions: &'static [&'static str],
    subject: &'static str,
}

/// A staff session: role-derived permissions.
fn person(grants: &'static [&'static str], permissions: &'static [&'static str]) -> Caller {
    Caller {
        grants,
        permissions,
        subject: "user",
    }
}

/// A scoped API key: scope-derived permissions, `subject` agent.
fn key(grants: &'static [&'static str], permissions: &'static [&'static str]) -> Caller {
    Caller {
        grants,
        permissions,
        subject: "agent",
    }
}

fn bearer_ok(headers: &axum::http::HeaderMap) -> bool {
    let bearer = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == format!("Bearer {TOKEN}"));
    let key = headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == TOKEN);
    bearer || key
}

/// A stub of the core-api surface the adapter reads and proxies onto. It grants
/// exactly the capabilities it is given, so an absent grant proves the deny
/// path for all three surfaces.
fn stub_core_api(caller: Caller) -> Router {
    Router::new()
        .route(
            "/api/v1/me",
            get(move |headers: axum::http::HeaderMap| async move {
                if !bearer_ok(&headers) {
                    return axum::http::StatusCode::UNAUTHORIZED.into_response();
                }
                Json(json!({
                    "user_id": "018f0a5e-0000-7000-8000-000000000001",
                    "org_id": "018f0a5e-0000-7000-8000-000000000002",
                    "email": "tech@example.test",
                    "display_name": "Tech",
                    "timezone": "UTC",
                    "permissions": caller.permissions,
                    "subject": caller.subject,
                    "mcp_capabilities": caller.grants
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
        .route(
            "/api/clients/{id}",
            get(
                |Path(id): Path<String>, headers: axum::http::HeaderMap| async move {
                    if !bearer_ok(&headers) {
                        return axum::http::StatusCode::UNAUTHORIZED.into_response();
                    }
                    Json(json!({ "id": id, "name": "Acme" })).into_response()
                },
            ),
        )
        .route(
            "/api/tickets/{id}",
            get(
                |Path(id): Path<String>, headers: axum::http::HeaderMap| async move {
                    if !bearer_ok(&headers) {
                        return axum::http::StatusCode::UNAUTHORIZED.into_response();
                    }
                    Json(json!({ "id": id, "summary": "Printer offline" })).into_response()
                },
            ),
        )
        .route(
            "/api/documents/{id}",
            get(
                |Path(id): Path<String>, headers: axum::http::HeaderMap| async move {
                    if !bearer_ok(&headers) {
                        return axum::http::StatusCode::UNAUTHORIZED.into_response();
                    }
                    Json(json!({ "id": id, "title": "Runbook" })).into_response()
                },
            ),
        )
}

async fn app(caller: Caller) -> Router {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, stub_core_api(caller)).await.unwrap() });
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
        // The header names the target: a tool or prompt by name, a resource by
        // URI (SEP-2243).
        if let Some(name) = body
            .get("params")
            .and_then(|params| params.get("name").or_else(|| params.get("uri")))
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
    modern(method, json!({}))
}

fn modern(method: &str, params: Value) -> Value {
    let mut params = params;
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": { "name": "test", "version": "1" },
        "io.modelcontextprotocol/clientCapabilities": {}
    });
    json!({ "jsonrpc": "2.0", "id": 7, "method": method, "params": params })
}

#[tokio::test]
async fn a_modern_client_discovers_and_lists_its_grants() {
    let app = app(person(&["clients:read"], &["clients:view"])).await;
    let discovered = rpc(&app, Some(TOKEN), modern_meta("server/discover")).await;
    let result = &discovered["result"];
    assert!(
        result["supportedVersions"]
            .as_array()
            .is_some_and(|versions| versions.iter().any(|v| v == "2026-07-28")),
        "server advertises the revision it implements: {result}"
    );
    assert!(
        result["capabilities"]["resources"].is_object()
            && result["capabilities"]["prompts"].is_object(),
        "resources and prompts are advertised: {result}"
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
    let app = app(person(&["clients:read"], &["clients:view"])).await;
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
async fn a_bound_key_sees_only_the_tools_its_own_permissions_open() {
    // The org granted both families; this key was minted with `clients:read`
    // alone, so every ticket tool would be refused by the handler behind it.
    let bound_app = app(key(&["clients:read", "tickets:read"], &["clients:view"])).await;
    let listed = rpc(&bound_app, Some(TOKEN), modern_meta("tools/list")).await;
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(names.contains(&"clients_list"), "{names:?}");
    assert!(
        !names.contains(&"tickets_list"),
        "a tool the key's own scopes cannot call must not be advertised: {names:?}"
    );

    // The same org, a key that carries the ticket scope: the family returns.
    let permitted_app = app(key(
        &["clients:read", "tickets:read"],
        &["clients:view", "tickets:view"],
    ))
    .await;
    let listed = rpc(&permitted_app, Some(TOKEN), modern_meta("tools/list")).await;
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(
        names.contains(&"tickets_list") && names.contains(&"clients_list"),
        "{names:?}"
    );
}

#[tokio::test]
async fn a_key_is_not_shown_a_family_its_credential_cannot_reach() {
    // `notifications` is self-scoped and org-granted, so a person sees it. An
    // `x-api-key` cannot reach that path at all: core-api refuses the credential
    // before a handler runs, so the adapter does not advertise the tool.
    let person_view = app(person(&["notifications:read"], &["clients:view"])).await;
    let listed = rpc(&person_view, Some(TOKEN), modern_meta("tools/list")).await;
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(names.contains(&"notifications_list"), "{names:?}");

    let key_view = app(key(&["notifications:read"], &["clients:view"])).await;
    let listed = rpc(&key_view, Some(TOKEN), modern_meta("tools/list")).await;
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(
        !names.contains(&"notifications_list"),
        "a key cannot reach this path, so it must not be offered: {names:?}"
    );
}

#[tokio::test]
async fn a_session_without_a_token_sees_no_tools() {
    let app = app(person(&["clients:read"], &["clients:view"])).await;
    let listed = rpc(&app, None, modern_meta("tools/list")).await;
    assert_eq!(
        listed["result"]["tools"].as_array().map(Vec::len),
        Some(0),
        "no session means no grants means no tools"
    );
    let templates = rpc(&app, None, modern_meta("resources/templates/list")).await;
    assert_eq!(
        templates["result"]["resourceTemplates"]
            .as_array()
            .map(Vec::len),
        Some(0),
        "and no resources"
    );
    let prompts = rpc(&app, None, modern_meta("prompts/list")).await;
    assert_eq!(
        prompts["result"]["prompts"].as_array().map(Vec::len),
        Some(0),
        "and no prompts"
    );
}

#[tokio::test]
async fn a_call_forwards_the_callers_session_and_refuses_an_ungranted_family() {
    let app = app(person(&["clients:read"], &["clients:view"])).await;
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

#[tokio::test]
async fn resources_are_templates_scoped_by_grant() {
    let app = app(person(
        &["tickets:read", "documents:read"],
        &["tickets:view", "documents:view"],
    ))
    .await;
    let listed = rpc(&app, Some(TOKEN), modern_meta("resources/list")).await;
    assert_eq!(
        listed["result"]["resources"].as_array().map(Vec::len),
        Some(0),
        "records are not enumerated; templates carry discovery"
    );

    let templates = rpc(&app, Some(TOKEN), modern_meta("resources/templates/list")).await;
    let templates = templates["result"]["resourceTemplates"]
        .as_array()
        .expect("templates array");
    let uris: Vec<&str> = templates
        .iter()
        .filter_map(|template| template["uriTemplate"].as_str())
        .collect();
    assert_eq!(
        uris,
        vec!["marula://tickets/{id}", "marula://documents/{id}"]
    );
    assert!(templates
        .iter()
        .all(|template| template["mimeType"] == "application/json"));
}

#[tokio::test]
async fn a_resource_read_proxies_the_same_record_the_tool_would() {
    let app = app(person(&["tickets:read"], &["tickets:view"])).await;
    let read = rpc(
        &app,
        Some(TOKEN),
        modern(
            "resources/read",
            json!({ "uri": format!("marula://tickets/{TICKET}") }),
        ),
    )
    .await;
    assert!(read["error"].is_null(), "{read}");
    let contents = &read["result"]["contents"][0];
    assert_eq!(contents["uri"], format!("marula://tickets/{TICKET}"));
    assert_eq!(contents["mimeType"], "application/json");
    let text = contents["text"].as_str().expect("text contents");
    assert!(text.contains("Printer offline"), "proxied payload: {text}");
    assert!(text.contains(TICKET), "same record the caller asked for");
}

#[tokio::test]
async fn a_resource_read_refuses_an_ungranted_family_and_a_reshaped_uri() {
    let app = app(person(&["tickets:read"], &["tickets:view"])).await;
    let refused = rpc(
        &app,
        Some(TOKEN),
        modern(
            "resources/read",
            json!({ "uri": "marula://clients/018f0a5e-0000-7000-8000-000000000002" }),
        ),
    )
    .await;
    let message = refused["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("clients:read"), "{refused}");

    for uri in [
        "marula://tickets/../../api/v1/audit-events",
        "marula://tickets/a/b",
        "marula://tickets/abc%3Fadmin=1",
        "file:///etc/passwd",
        "marula://invoices/abc",
    ] {
        let refused = rpc(
            &app,
            Some(TOKEN),
            modern("resources/read", json!({ "uri": uri })),
        )
        .await;
        assert!(
            !refused["error"].is_null(),
            "{uri} must be refused, got {refused}"
        );
    }
}

#[tokio::test]
async fn prompts_are_listed_and_served_only_when_the_first_read_is_granted() {
    let app = app(person(&["tickets:read"], &["tickets:view"])).await;
    let listed = rpc(&app, Some(TOKEN), modern_meta("prompts/list")).await;
    let names: Vec<&str> = listed["result"]["prompts"]
        .as_array()
        .expect("prompts array")
        .iter()
        .filter_map(|prompt| prompt["name"].as_str())
        .collect();
    assert_eq!(names, vec!["triage_ticket"]);
    let argument = &listed["result"]["prompts"][0]["arguments"][0];
    assert_eq!(argument["name"], "ticket_id");
    assert_eq!(argument["required"], json!(true));

    let served = rpc(
        &app,
        Some(TOKEN),
        modern(
            "prompts/get",
            json!({ "name": "triage_ticket", "arguments": { "ticket_id": TICKET } }),
        ),
    )
    .await;
    let messages = served["result"]["messages"]
        .as_array()
        .expect("messages array");
    assert_eq!(messages.len(), 2, "text plus the record link: {served}");
    let text = messages[0]["content"]["text"]
        .as_str()
        .expect("text content");
    assert!(
        text.contains(&format!("marula://tickets/{TICKET}")),
        "the workflow names the record it starts from: {text}"
    );
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[1]["content"]["type"], "resource_link");
    assert_eq!(
        messages[1]["content"]["uri"],
        format!("marula://tickets/{TICKET}")
    );

    // A prompt whose first read is not granted is neither listed nor served.
    let refused = rpc(
        &app,
        Some(TOKEN),
        modern(
            "prompts/get",
            json!({ "name": "client_qbr_pack", "arguments": { "client_id": TICKET } }),
        ),
    )
    .await;
    let message = refused["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("clients:read"), "{refused}");

    // The record id is validated before it reaches a proxied path.
    let refused = rpc(
        &app,
        Some(TOKEN),
        modern(
            "prompts/get",
            json!({ "name": "triage_ticket", "arguments": { "ticket_id": "../../audit" } }),
        ),
    )
    .await;
    assert!(!refused["error"].is_null(), "{refused}");
}
