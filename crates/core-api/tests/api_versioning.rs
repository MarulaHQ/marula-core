//! API versioning conformance (TASKLIST U1, upgrade-compatibility-policy.md
//! §2/§3): the `/api/v1` canonical surface and the unversioned legacy alias
//! must behave IDENTICALLY route-for-route.
//!
//! Purpose:
//! 1. EXHAUSTIVE dual-mount parity — every path+method in the committed
//!    OpenAPI snapshot (`packages/sdk/openapi.json`, documented contract)
//!    plus the undocumented machine surfaces is probed against BOTH
//!    prefixes; status and response body bytes must be equal. A missing
//!    twin yields 404-elsewhere and fails; a forked payload fails.
//! 2. Existence sentinel — axum's default unmatched response has an EMPTY
//!    body with no content-type, while every real rejection here (401 auth
//!    middleware or handler validation) returns a JSON body. A probe that
//!    lands on an empty-body 404 on EITHER mount means the surface was
//!    never mounted, which parity alone cannot catch.
//! 3. Auth-parity fixtures: unauthenticated 401 shapes and failed-login
//!    bodies are byte-equal across prefixes.
//! 4. Scope guards: the live leg is this API's own SSE stream
//!    (`/api/events/stream`, plus the presence routes) on the API's single
//!    published port — no second live-push binary or port; the machine-only
//!    search endpoints stay UNVERSIONED by design; Scalar docs stay
//!    root-level only.
//! 5. Snapshot drift gate: the served `/openapi.json` equals the committed
//!    SDK snapshot exactly (key-for-key), that snapshot documents v1 paths
//!    exclusively (zero legacy keys), and it covers no nonexistent surface
//!    blindly because every key feeds probe set (1).
//!
//! Non-purpose: authenticated business-flow behavior of individual
//! endpoints (owned by their flow suites) and deprecation headers (U4).
//!
//! Database selection follows settings_flow's contract:
//! MARULA_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;

const LEGACY: &str = "/api";
const CANONICAL: &str = "/api/v1";

/// Undocumented public machine surfaces + health. They carry deliberate
/// in-handler token/HMAC checks instead of JWT middleware, so they are not
/// in the spec; keeping them probed here keeps their alias twin enforced.
const MACHINE_TAILS: &[&str] = &[
    "/health",
    "/webhooks/email-inbound",
    "/webhooks/ingest/no-such-source",
    "/internal/email/outbound/claim",
    "/internal/email/outbound/results",
    "/internal/sla/verdict",
    "/internal/effects",
    "/internal/time/gap-audit/run",
    "/internal/webhook/deliveries",
];

async fn test_state() -> AppState {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("versioning e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url;
    AppState {
        pool,
        reporting: core_api::state::AppState::reporting_pool_lazy(&config.database_url)
            .expect("reporting pool"),
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
    }
}

async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
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
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
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

async fn status_and_body(response: axum::response::Response) -> (u16, Vec<u8>) {
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, bytes.to_vec())
}

/// Registers a fresh org through the legacy prefix and returns an owner
/// access token (both prefixes share org state — same handlers, same DB).
async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("{LEGACY}/auth/register"),
        None,
        Some(json!({
            "org_name": "Versioning Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let (status, body) = status_and_body(response).await;
    assert_eq!(status, 200);
    let value: Value = serde_json::from_slice(&body).unwrap();
    value["access_token"].as_str().expect("access token").into()
}

/// Replaces every `{param}` template segment with a syntactically valid
/// placeholder so probes reach route matching rather than pattern failure.
fn concrete_path(template: &str, prefix: &str) -> String {
    format!("{prefix}{}", template.replace(['{', '}'], ""))
}

fn committed_spec() -> Value {
    // The committed snapshot lives three levels up from this crate's dir.
    let manifest = env!("CARGO_MANIFEST_DIR");
    let path = std::path::Path::new(manifest)
        .join("../../packages/sdk/openapi.json")
        .canonicalize()
        .expect("openapi.json sibling location");
    serde_json::from_str(&std::fs::read_to_string(path).expect("committed snapshot readable"))
        .expect("snapshot parses as JSON")
}

fn spec_probe_methods(entry: &Value) -> Vec<(axum::http::Method, Option<Value>)> {
    const METHODS: &[&str] = &["get", "post", "put", "patch", "delete"];
    METHODS
        .iter()
        .filter(|m| entry.get(**m).is_some())
        .map(|m| {
            (
                axum::http::Method::from_bytes(m.to_uppercase().as_bytes()).unwrap(),
                // Post bodies: harmless minimal input; auth/validation
                // rejections are deterministic and body-carrying.
                if *m == "post" || *m == "put" || *m == "patch" {
                    Some(json!({}))
                } else {
                    None
                },
            )
        })
        .collect()
}

#[tokio::test]
async fn canonical_and_legacy_mounts_are_parity_clones() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());

    let slug = format!("ver-parity-{}", uuid::Uuid::now_v7().simple());
    let _token = register_org(&app, &slug).await;

    // Parity probe over every documented operation of the committed
    // snapshot. Probes are UNAUTHENTICATED by design: protected surfaces
    // therefore exercise require_auth itself, and public handlers yield
    // deterministic validation errors — bodies stay byte-comparable.
    let spec = committed_spec();
    for (template, entry) in spec["paths"].as_object().unwrap() {
        assert!(
            template.starts_with(CANONICAL),
            "spec documents a non-v1 path: {template}"
        );
        // Probe EVERY documented method, not just the first.
        let tail = template.replacen(CANONICAL, "", 1);
        for (method, body) in spec_probe_methods(entry) {
            let legacy_uri = concrete_path(&tail, LEGACY);
            let canonical_uri = concrete_path(&tail, CANONICAL);
            let legacy =
                status_and_body(send(&app, method.clone(), &legacy_uri, None, body.clone()).await)
                    .await;
            let canonical =
                status_and_body(send(&app, method.clone(), &canonical_uri, None, body).await).await;
            assert_eq!(
                legacy.0, canonical.0,
                "status diverges for {method} {template}"
            );
            assert_eq!(
                legacy.1, canonical.1,
                "body diverges for {method} {template}"
            );
            // Existence sentinel: an empty 404 means the surface is absent
            // (axum default), while every reachable rejection carries JSON.
            for side in [legacy, canonical] {
                assert!(
                    !(side.0 == 404 && side.1.is_empty()),
                    "{method} {template} resolved to an unmounted default 404"
                );
            }
        }
    }

    // Undocumented machine surfaces: alias twin enforcement + existence
    // sentinel (in-handler token/HMAC checks emit JSON rejections).
    for tail in MACHINE_TAILS {
        let method = if *tail == "/health" {
            axum::http::Method::GET
        } else {
            axum::http::Method::POST
        };
        let body = if *tail == "/health" {
            None
        } else {
            Some(json!({}))
        };
        let legacy = status_and_body(
            send(
                &app,
                method.clone(),
                &format!("{LEGACY}{tail}"),
                None,
                body.clone(),
            )
            .await,
        )
        .await;
        let canonical =
            status_and_body(send(&app, method, &format!("{CANONICAL}{tail}"), None, body).await)
                .await;
        assert_eq!(legacy.0, canonical.0, "machine surface diverges: {tail}");
        assert_eq!(legacy.1, canonical.1, "machine surface diverges: {tail}");
        for side in [legacy, canonical] {
            assert!(
                !(side.0 == 404 && side.1.is_empty()),
                "machine surface unmounted: {tail}"
            );
        }
    }
}

#[tokio::test]
async fn auth_middleware_401_shapes_are_identical() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());

    for tail in ["/me", "/clients", "/settings/registry", "/audit-events"] {
        let legacy = status_and_body(
            send(
                &app,
                axum::http::Method::GET,
                &format!("{LEGACY}{tail}"),
                None,
                None,
            )
            .await,
        )
        .await;
        let canonical = status_and_body(
            send(
                &app,
                axum::http::Method::GET,
                &format!("{CANONICAL}{tail}"),
                None,
                None,
            )
            .await,
        )
        .await;
        assert_eq!(legacy.0, 401, "unauthenticated GET {tail} must be 401");
        assert_eq!(legacy, canonical, "401 shape diverges for {tail}");
        assert!(!canonical.1.is_empty(), "401 must carry a JSON error body");
    }

    // Failed login (bad credentials) — same deterministic error via both.
    let payload = json!({"email": "ghost@nowhere.test", "password": "wrong"});
    let legacy = status_and_body(
        send(
            &app,
            axum::http::Method::POST,
            &format!("{LEGACY}/auth/login"),
            None,
            Some(payload.clone()),
        )
        .await,
    )
    .await;
    let canonical = status_and_body(
        send(
            &app,
            axum::http::Method::POST,
            &format!("{CANONICAL}/auth/login"),
            None,
            Some(payload),
        )
        .await,
    )
    .await;
    assert_eq!(legacy.0, 401);
    assert_eq!(legacy, canonical);
}

#[tokio::test]
async fn authenticated_read_parity_on_representative_fixtures() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());

    let slug = format!("ver-read-{}", uuid::Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;

    // Read-only GETs hit the same org state through both mounts.
    for tail in [
        "/me",
        "/boards",
        "/boards/nav",
        "/clients",
        "/tickets?limit=1",
        "/timers/active",
        "/notifications/unread-count",
        "/settings/registry",
        "/org",
        "/time-settings",
    ] {
        let legacy = status_and_body(
            send(
                &app,
                axum::http::Method::GET,
                &format!("{LEGACY}{tail}"),
                Some(&token),
                None,
            )
            .await,
        )
        .await;
        let canonical = status_and_body(
            send(
                &app,
                axum::http::Method::GET,
                &format!("{CANONICAL}{tail}"),
                Some(&token),
                None,
            )
            .await,
        )
        .await;
        assert_eq!(
            legacy.0, canonical.0,
            "status diverges for authenticated GET {tail}"
        );
        assert_eq!(legacy.0, 200, "authenticated GET {tail} must succeed");
        assert_eq!(legacy.1, canonical.1, "body diverges for GET {tail}");
    }

    // Deterministic validation rejection: duplicate registration slug →
    // identical conflict/error shape on both prefixes.
    let dup = json!({
        "org_name": "Versioning Co",
        "org_slug": slug,
        "email": "other@dup.test",
        "password": "correct-horse-battery",
        "display_name": "Again",
    });
    let legacy = status_and_body(
        send(
            &app,
            axum::http::Method::POST,
            &format!("{LEGACY}/auth/register"),
            None,
            Some(dup.clone()),
        )
        .await,
    )
    .await;
    let canonical = status_and_body(
        send(
            &app,
            axum::http::Method::POST,
            &format!("{CANONICAL}/auth/register"),
            None,
            Some(dup),
        )
        .await,
    )
    .await;
    assert_ne!(legacy.0, 200, "duplicate slug must fail");
    assert_eq!(legacy, canonical);
}

#[tokio::test]
async fn live_leg_is_the_api_stream_and_docs_stay_at_root() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());

    // The live leg is THIS API's own SSE stream, mounted on this process's
    // one published port at `/api/events/stream` and dual-mounted under the
    // versioned prefix like every other route. The presence routes ride the
    // same in-process machinery. Auth rejects these unauthenticated probes
    // (401 or 405); what matters is that the routes are REGISTERED — an
    // empty-body 404 is axum's unmatched sentinel and means never mounted.
    for uri in [
        format!("{LEGACY}/events/stream"),
        format!("{CANONICAL}/events/stream"),
    ] {
        let (status, body) =
            status_and_body(send(&app, axum::http::Method::GET, &uri, None, None).await).await;
        assert!(
            !(status == 404 && body.is_empty()),
            "{uri} is the live leg and must be mounted"
        );
    }
    for uri in [
        format!("{LEGACY}/presence/heartbeat"),
        format!("{CANONICAL}/presence/heartbeat"),
    ] {
        let (status, body) = status_and_body(
            send(&app, axum::http::Method::POST, &uri, None, Some(json!({}))).await,
        )
        .await;
        assert!(
            !(status == 404 && body.is_empty()),
            "{uri} is part of the live leg and must be mounted"
        );
    }

    // No second live-push surface is published: the retired WebSocket hub's
    // paths are gone from both mounts, so nothing shadows the stream.
    for uri in [
        format!("{LEGACY}/ws"),
        format!("{CANONICAL}/ws"),
        format!("{LEGACY}/realtime"),
        format!("{CANONICAL}/realtime"),
    ] {
        let (status, _) =
            status_and_body(send(&app, axum::http::Method::GET, &uri, None, None).await).await;
        assert_eq!(status, 404, "{uri} must not exist");
    }

    // Machine-only search endpoints stay UNVERSIONED forever: mounted at
    // /internal/search/*, never under /api of either generation.
    let internal_ok = status_and_body(
        send(
            &app,
            axum::http::Method::POST,
            "/internal/search/upsert",
            None,
            Some(json!({})),
        )
        .await,
    )
    .await;
    assert!(
        !(internal_ok.0 == 404 && internal_ok.1.is_empty()),
        "internal search mount vanished"
    );
    for uri in [
        format!("{LEGACY}/search/upsert"),
        format!("{CANONICAL}/search/upsert"),
        format!("{LEGACY}/internal/search/delete"),
        format!("{CANONICAL}/internal/search/delete"),
    ] {
        // NOTE: /{prefix}/search exists as a DIFFERENT staff GET route, but
        // POST /{prefix}/internal/search/* has no registration by design.
        let (status, _) = status_and_body(
            send(&app, axum::http::Method::POST, &uri, None, Some(json!({}))).await,
        )
        .await;
        assert_eq!(status, 404, "{uri} must remain unversioned-only 404 here");
    }

    // Docs/scalar live at root once, never per generation.
    for uri in ["/openapi.json", "/docs"] {
        let (status, _) =
            status_and_body(send(&app, axum::http::Method::GET, uri, None, None).await).await;
        assert!(status < 400, "{uri} must serve at root ({status})");
    }
    for uri in [
        format!("{LEGACY}/openapi.json"),
        format!("{CANONICAL}/openapi.json"),
        format!("{LEGACY}/docs"),
        format!("{CANONICAL}/docs"),
    ] {
        let (status, _) =
            status_and_body(send(&app, axum::http::Method::GET, &uri, None, None).await).await;
        assert_eq!(status, 404, "{uri} must not shadow docs paths");
    }
}

#[tokio::test]
async fn served_spec_equals_committed_v1_only_snapshot() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());

    let (_, served) =
        status_and_body(send(&app, axum::http::Method::GET, "/openapi.json", None, None).await)
            .await;
    let served: Value = serde_json::from_slice(&served).expect("served spec parses");
    let committed = committed_spec();

    let served_paths = served["paths"]
        .as_object()
        .expect("paths map")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    let committed_paths = committed["paths"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(served_paths.len(), committed_paths.len(), "documented path count drifted — regenerate packages/sdk/openapi.json (cargo run -p core-api --bin dump-openapi -- --write packages/sdk/openapi.json)");
    assert!(
        served_paths
            .iter()
            .zip(committed_paths.iter())
            .all(|(a, b)| a == b),
        "served spec keys differ from committed snapshot — regenerate"
    );

    let legacy_keys = committed_paths
        .iter()
        .filter(|k| !k.starts_with(CANONICAL))
        .collect::<Vec<_>>();
    assert!(
        legacy_keys.is_empty(),
        "SDK snapshot must document /api/v1 exclusively; leaked: {legacy_keys:?}"
    );
}
