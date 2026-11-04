//! Conformance against the SDK's own client. The raw JSON-RPC test proves the
//! wire shape; this one proves the official `rmcp` client, which negotiates the
//! modern revision, can discover and call through the same session rule.

use axum::{response::IntoResponse, routing::get, Json, Router};
use rmcp::{
    transport::streamable_http_client::StreamableHttpClientTransportConfig,
    transport::StreamableHttpClientTransport, ServiceExt,
};
use serde_json::json;

const TOKEN: &str = "session-token";

fn bearer_ok(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == format!("Bearer {TOKEN}"))
}

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

/// Starts the stub api and the adapter. Returns the adapter's base URL.
async fn adapter() -> String {
    let stub_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let stub_addr = stub_listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(stub_listener, stub_core_api()).await.unwrap() });

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
    let base = adapter().await;
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
async fn the_official_client_without_a_session_sees_no_tools() {
    let base = adapter().await;
    let transport = StreamableHttpClientTransport::from_config(transport_config(&base, None));
    let client = ().serve(transport).await.expect("client connects");
    let tools = client.list_all_tools().await.expect("tools/list");
    assert!(tools.is_empty(), "{:?}", tools.len());
    client.cancel().await.ok();
}
