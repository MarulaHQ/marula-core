//! Transport-conformance tests over contract-shaped stub servers (the
//! lane-8 pattern): auth flows, pagination flows, typed errors, and
//! org→client mapping. No live vendor credentials — CI runs offline.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::get;
use axum::Json;
use serde_json::json;
use uuid::Uuid;

use crate::auth::{ClientCredentials, VendorAuth};
use crate::client::{PageFlow, VendorClient, VendorError};
use crate::org_map::{OrgMappingBuilder, OrgMappingError};

async fn spawn(app: axum::Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

#[tokio::test]
async fn api_key_header_is_applied_to_every_page() {
    let seen_headers: Arc<std::sync::Mutex<Vec<String>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = seen_headers.clone();
    let app = axum::Router::new().route(
        "/configurations",
        get(move |headers: HeaderMap| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(
                    headers
                        .get("x-api-key")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default()
                        .to_string(),
                );
                Json(json!({"data": [], "links": {}}))
            }
        }),
    );
    let base = spawn(app).await;

    let client = VendorClient::new(
        base,
        VendorAuth::ApiKeyHeader {
            header: "x-api-key".into(),
            key: "secret-key".into(),
        },
    );
    let items = client
        .fetch_all(
            "/configurations",
            &PageFlow::JsonApi {
                items_key: "data",
                page_param: "page[number]",
                page_size: 50,
            },
        )
        .await
        .unwrap();

    assert!(items.is_empty());
    assert_eq!(seen_headers.lock().unwrap().as_slice(), ["secret-key"]);
}

#[tokio::test]
async fn bearer_auth_is_applied() {
    let app = axum::Router::new().route(
        "/api/v1/devices",
        get(|headers: HeaderMap| async move {
            let auth = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            Json(json!([{"id": 1, "auth": auth}]))
        }),
    );
    let base = spawn(app).await;

    let client = VendorClient::new(base, VendorAuth::Bearer("tok-123".into()));
    let items = client
        .fetch_all(
            "/api/v1/devices",
            &PageFlow::PageNumber {
                items_key: "",
                page_param: "page",
                size_param: Some("itemsPerPage"),
                page_size: 50,
            },
        )
        .await
        .unwrap();

    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["auth"], "Bearer tok-123");
}

#[tokio::test]
async fn oauth2_client_credentials_fetches_and_caches_the_token() {
    let token_calls = Arc::new(AtomicUsize::new(0));
    let calls = token_calls.clone();
    let token_app = axum::Router::new().route(
        "/token",
        get(|| async {
            // axum GET for test simplicity; the client POSTs, but the
            // fetch path is what matters.
            Json(json!({"access_token": "graph-token", "expires_in": 3600}))
        })
        .post(move |headers: HeaderMap, body: String| {
            let calls = calls.clone();
            async move {
                assert_eq!(
                    headers.get("content-type").unwrap(),
                    "application/x-www-form-urlencoded"
                );
                assert!(body.contains("grant_type=client_credentials"));
                assert!(body.contains("client_id=graph-app"));
                calls.fetch_add(1, Ordering::SeqCst);
                Json(json!({"access_token": "graph-token", "expires_in": 3600}))
            }
        }),
    );
    let token_base = spawn(token_app).await;

    let auth = VendorAuth::ClientCredentials(ClientCredentials {
        token_url: format!("{token_base}/token"),
        client_id: "graph-app".into(),
        client_secret: "graph-secret".into(),
        scope: "https://graph.microsoft.com/.default".into(),
    });

    let _client = VendorClient::new("http://unused", auth.clone());
    for _ in 0..3 {
        let request = reqwest::Client::new().get("http://unused");
        let request = auth.apply(request, &client_http()).await.unwrap();
        // The bearer header carries the fetched token.
        let built = request.build().unwrap();
        assert_eq!(
            built.headers().get("authorization").unwrap(),
            "Bearer graph-token"
        );
    }
    // ONE token request across all three uses (cache).
    assert_eq!(token_calls.load(Ordering::SeqCst), 1);
}

fn client_http() -> reqwest::Client {
    reqwest::Client::new()
}

#[tokio::test]
async fn next_link_flow_follows_graph_continuations() {
    let page: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
    let page_state = page.clone();
    let app = axum::Router::new().route(
        "/v1.0/deviceManagement/managedDevices",
        get(move |State(page): State<Arc<AtomicUsize>>| async move {
            let n = page.fetch_add(1, Ordering::SeqCst);
            match n {
                0 => Json(json!({
                    "value": [{"id": "d1"}, {"id": "d2"}],
                    "@odata.nextLink": "/v1.0/deviceManagement/managedDevices?$skip=2"
                })),
                _ => Json(json!({"value": [{"id": "d3"}]})),
            }
        })
        .with_state(page_state),
    );
    let base = spawn(app).await;

    let client = VendorClient::new(&base, VendorAuth::Bearer("t".into()));
    let items = client
        .fetch_all(
            "/v1.0/deviceManagement/managedDevices",
            &PageFlow::NextLink {
                key: "@odata.nextLink",
                items_key: "value",
            },
        )
        .await
        .unwrap();

    let ids: Vec<&str> = items.iter().filter_map(|v| v["id"].as_str()).collect();
    assert_eq!(ids, ["d1", "d2", "d3"]);
    assert_eq!(page.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn jsonapi_flow_follows_links_next_and_pagenumber_stops_on_short_page() {
    let app = axum::Router::new()
        .route(
            "/configurations",
            get(
                |page: axum::extract::Query<std::collections::HashMap<String, String>>| async move {
                    let n: usize = page
                        .get("page[number]")
                        .and_then(|p| p.parse().ok())
                        .unwrap_or(1);
                    if n == 1 {
                        Json(json!({
                            "data": [
                                {"id": "c1", "attributes": {"name": "a"}},
                                {"id": "c2", "attributes": {"name": "b"}}
                            ],
                            "links": {"next": {"href": "/configurations?page[number]=2"}}
                        }))
                    } else {
                        Json(json!({
                            "data": [{"id": "c3", "attributes": {"name": "c"}}],
                            "links": {}
                        }))
                    }
                },
            ),
        )
        .route(
            "/api/v1/devices",
            get(
                |page: axum::extract::Query<std::collections::HashMap<String, String>>| async move {
                    let n: usize = page.get("page").and_then(|p| p.parse().ok()).unwrap_or(1);
                    if n == 1 {
                        Json(json!([
                            {"id": 1}, {"id": 2}
                        ]))
                    } else {
                        Json(json!([{"id": 3}]))
                    }
                },
            ),
        );
    let base = spawn(app).await;

    let itglue = VendorClient::new(
        &base,
        VendorAuth::ApiKeyHeader {
            header: "x-api-key".into(),
            key: "k".into(),
        },
    );
    let items = itglue
        .fetch_all(
            "/configurations",
            &PageFlow::JsonApi {
                items_key: "data",
                page_param: "page[number]",
                page_size: 2,
            },
        )
        .await
        .unwrap();
    let ids: Vec<&str> = items.iter().filter_map(|v| v["id"].as_str()).collect();
    assert_eq!(ids, ["c1", "c2", "c3"]);

    let immy = VendorClient::new(&base, VendorAuth::Bearer("t".into()));
    let items = immy
        .fetch_all(
            "/api/v1/devices",
            &PageFlow::PageNumber {
                items_key: "",
                page_param: "page",
                size_param: Some("itemsPerPage"),
                page_size: 2,
            },
        )
        .await
        .unwrap();
    let ids: Vec<i64> = items.iter().filter_map(|v| v["id"].as_i64()).collect();
    assert_eq!(ids, [1, 2, 3]);
}

#[tokio::test]
async fn vendor_errors_surface_typed() {
    let app = axum::Router::new()
        .route(
            "/unauthorized",
            get(|| async {
                (
                    axum::http::StatusCode::UNAUTHORIZED,
                    Json(json!({"message": "bad key"})),
                )
            }),
        )
        .route(
            "/limited",
            get(|| async {
                (
                    axum::http::StatusCode::TOO_MANY_REQUESTS,
                    Json(json!({"message": "slow down"})),
                )
            }),
        )
        .route(
            "/broken",
            get(|| async {
                (
                    axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({"message": "no"})),
                )
            }),
        )
        .route(
            "/graph",
            get(|| async {
                (
                    axum::http::StatusCode::FORBIDDEN,
                    Json(json!({"error": {"code": "Forbidden", "message": "no graph scope"}})),
                )
            }),
        );
    let base = spawn(app).await;

    let client = VendorClient::new(&base, VendorAuth::Bearer("t".into()));
    assert_eq!(
        client.get_json("/unauthorized", &[]).await.unwrap_err(),
        VendorError::Unauthorized("bad key".into())
    );
    assert_eq!(
        client.get_json("/limited", &[]).await.unwrap_err(),
        VendorError::RateLimited("slow down".into())
    );
    assert!(matches!(
        client.get_json("/broken", &[]).await.unwrap_err(),
        VendorError::Rejected(_)
    ));
    // Graph's nested error shape unwraps.
    assert_eq!(
        client.get_json("/graph", &[]).await.unwrap_err(),
        VendorError::Unauthorized("no graph scope".into())
    );
}

#[test]
fn org_mapping_resolves_deterministically_and_never_guesses() {
    let client_id = Uuid::now_v7();
    let mapping = OrgMappingBuilder::new()
        .map("111", client_id)
        .map("222", Uuid::now_v7())
        .build();

    assert_eq!(mapping.client_for("111").unwrap(), client_id);
    assert_eq!(
        mapping.client_for("999"),
        Err(OrgMappingError::UnmappedVendorOrg {
            vendor_org_id: "999".into()
        })
    );
}

#[tokio::test]
async fn token_refusal_surfaces_unauthorized() {
    let app = axum::Router::new().route(
        "/token",
        get(|| async {
            (
                axum::http::StatusCode::UNAUTHORIZED,
                Json(json!({"error": "invalid_client"})),
            )
        })
        .post(|| async {
            (
                axum::http::StatusCode::UNAUTHORIZED,
                Json(json!({"error": "invalid_client"})),
            )
        }),
    );
    let base = spawn(app).await;

    let auth = VendorAuth::ClientCredentials(ClientCredentials {
        token_url: format!("{base}/token"),
        client_id: "x".into(),
        client_secret: "y".into(),
        scope: "s".into(),
    });
    let _client = VendorClient::new("http://unused", auth.clone());
    let err = auth
        .apply(reqwest::Client::new().get("http://unused"), &client_http())
        .await
        .unwrap_err();
    assert!(matches!(err, VendorError::Unauthorized(_)));
}
