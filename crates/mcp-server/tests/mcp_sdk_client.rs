//! Conformance against the SDK's own client. The raw JSON-RPC test proves the
//! wire shape; this one proves the official `rmcp` client, which negotiates the
//! modern revision, can discover and use every surface through the same
//! session rule: tools, resource templates, resource reads and prompts.

use axum::{extract::Path, response::IntoResponse, routing::get, Json, Router};
use rmcp::{
    model::{GetPromptRequestParams, ReadResourceRequestParams, ResourceContents},
    transport::streamable_http_client::StreamableHttpClientTransportConfig,
    transport::StreamableHttpClientTransport,
    ServiceExt,
};
use serde_json::{json, Map};

const TOKEN: &str = "session-token";
const TICKET: &str = "018f0a5e-0000-7000-8000-0000000000aa";

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

/// What the org granted and what this credential holds. Every case here is a
/// staff session, so `subject` is `user` and the permission gate is the only
/// dimension.
struct Caller {
    grants: &'static [&'static str],
    permissions: &'static [&'static str],
}

fn person(grants: &'static [&'static str], permissions: &'static [&'static str]) -> Caller {
    Caller {
        grants,
        permissions,
    }
}

fn stub_core_api(caller: Caller) -> Router {
    let Caller {
        grants,
        permissions,
    } = caller;
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
                    "permissions": permissions,
                    "subject": "user",
                    "mcp_capabilities": grants
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
}

/// Starts the stub api and the adapter. Returns the adapter's base URL.
async fn adapter(caller: Caller) -> String {
    let stub_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let stub_addr = stub_listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(stub_listener, stub_core_api(caller))
            .await
            .unwrap()
    });

    let server = mcp_server::MarulaServer::new(
        &format!("http://{stub_addr}"),
        &mcp_server::openapi_spec(),
        &mcp_server::mcp_catalog_spec(),
        None,
    );
    let app = mcp_server::http::router(server);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn transport_config(base: &str, token: Option<&str>) -> StreamableHttpClientTransportConfig {
    let mut config = StreamableHttpClientTransportConfig::with_uri(format!("{base}/mcp"));
    if let Some(token) = token {
        config = config.auth_header(token);
    }
    config
}

#[tokio::test]
async fn the_official_client_lists_and_calls_within_the_session() {
    let base = adapter(person(&["clients:read"], &["clients:view"])).await;
    let transport =
        StreamableHttpClientTransport::from_config(transport_config(&base, Some(TOKEN)));
    let client = ().serve(transport).await.expect("official client connects");
    let tools = client.list_all_tools().await.expect("tools/list");
    let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    assert!(names.contains(&"clients_list"), "{names:?}");
    assert!(
        !names.contains(&"tickets_list"),
        "ungranted family: {names:?}"
    );

    let result = client
        .call_tool(rmcp::model::CallToolRequestParams::new("clients_list"))
        .await
        .expect("tools/call");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let text = result
        .content
        .first()
        .and_then(|block| block.as_text())
        .map(|text| text.text.clone())
        .expect("text content");
    assert!(text.contains("\"clients\""), "{text}");
    client.cancel().await.ok();
}

#[tokio::test]
async fn the_official_client_reads_a_resource_and_a_prompt() {
    let base = adapter(person(&["tickets:read"], &["tickets:view"])).await;
    let transport =
        StreamableHttpClientTransport::from_config(transport_config(&base, Some(TOKEN)));
    let client = ().serve(transport).await.expect("official client connects");

    let templates = client
        .list_all_resource_templates()
        .await
        .expect("resources/templates/list");
    let uris: Vec<&str> = templates
        .iter()
        .map(|template| template.uri_template.as_str())
        .collect();
    assert_eq!(uris, vec!["marula://tickets/{id}"]);

    let read = client
        .read_resource(ReadResourceRequestParams::new(format!(
            "marula://tickets/{TICKET}"
        )))
        .await
        .expect("resources/read");
    let text = read
        .contents
        .first()
        .and_then(|contents| match contents {
            ResourceContents::TextResourceContents { text, .. } => Some(text.clone()),
            _ => None,
        })
        .expect("text resource contents");
    assert!(text.contains("Printer offline"), "{text}");

    // An ungranted family is refused on the same surface.
    assert!(
        client
            .read_resource(ReadResourceRequestParams::new(
                "marula://clients/018f0a5e-0000-7000-8000-000000000002"
            ))
            .await
            .is_err(),
        "the grant decides a resource read too"
    );

    let prompts = client.list_all_prompts().await.expect("prompts/list");
    let names: Vec<&str> = prompts.iter().map(|prompt| prompt.name.as_str()).collect();
    assert_eq!(names, vec!["triage_ticket"]);

    let mut arguments = Map::new();
    arguments.insert("ticket_id".into(), json!(TICKET));
    let read = client
        .get_prompt(GetPromptRequestParams::new("triage_ticket").with_arguments(arguments))
        .await
        .expect("prompts/get");
    assert_eq!(read.messages.len(), 2);
    client.cancel().await.ok();
}

#[tokio::test]
async fn the_official_client_without_a_session_sees_no_tools() {
    let base = adapter(person(&["clients:read"], &["clients:view"])).await;
    let transport = StreamableHttpClientTransport::from_config(transport_config(&base, None));
    let client = ().serve(transport).await.expect("client connects");
    let tools = client.list_all_tools().await.expect("tools/list");
    assert!(tools.is_empty(), "{:?}", tools.len());
    let prompts = client.list_all_prompts().await.expect("prompts/list");
    assert!(prompts.is_empty(), "{:?}", prompts.len());
    client.cancel().await.ok();
}
