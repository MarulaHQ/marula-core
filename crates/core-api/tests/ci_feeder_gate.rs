//! CI7 feeder conformance gate (ADR 0003, Wave 12 lane 8) — the REAL
//! feeder adapters over the REAL core-api router and lane DB.
//!
//! The suite drives the same `MeridianFeederSink` a live transport
//! would (x-api-key machine key, public `/api/v1/config-items/feed-upsert`
//! seam). NinjaOne maps through its shipped normalizer; Auvik and IT Glue
//! records are built directly on the public feed model because those
//! mappers ship as marketplace connectors (ADR 0013) — the wire contract
//! under test is unchanged. The gate proves:
//!
//! 1. **Second sync updates its own link.** Each feeder's first sync
//!    creates the item + link; its second sync (changed content) lands
//!    on the SAME link id and the SAME item — no duplicate, and the
//!    other connectors' links are untouched.
//! 2. **No cross-connector adoption.** A source record never resolves
//!    through another connector's link: different sources carrying the
//!    same serial converge on ONE canonical item with ONE link per
//!    source (the core's CI3 adoption with evidence), while the same
//!    external id across different sources stays a distinct link.
//! 3. **Deterministic identity keys.** The same device fed with
//!    differently-formatted serial/MAC/hostname payloads resolves to
//!    the SAME item (no twin) — determinism of keys, not of luck.
//! 4. **Idempotent upsert.** The same record re-synced with unchanged
//!    content replays the stored result (IF1): no new write, no churn.
//! 5. **Never blind.** A record with NO identity signals still lands
//!    WITH its link (the core's feed_upsert guarantee, observed from
//!    the wire).
//! 6. **Scoped-machine-key posture.** `clients:write` keys ride
//!    x-api-key on the feeder seam; a read-scoped key gets a typed 403
//!    on the write.
//!
//! Database selection follows config_items_flow's contract:
//! MERIDIAN_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

use axum::Router;
use ci_feeders::{
    vendors, FeederSink, FeederSync, MeridianFeederConfig, MeridianFeederSink, SyncConfig,
};
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn test_state() -> AppState {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("ci feeder gate needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    config.secret_key = core_api::config::SecretKey([29; 32]);
    AppState {
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&reporting_url)
            .expect("lazy reporting pool"),
    }
}

async fn connect(url: &str) -> Result<sqlx::PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(url)
        .await
}

/// Boots the REAL router on a localhost port so the connector's REST
/// sink drives the full wire path (x-api-key auth middleware included).
async fn spawn_router(state: AppState) -> String {
    let app = core_api::routes::build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

async fn body_json(response: axum::response::Response) -> Value {
    let (parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => panic!(
            "body_json failed ({}): {e}; body: {}",
            parts.status,
            String::from_utf8_lossy(&bytes)
        ),
    }
}

async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": "Feeder Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    body_json(response).await["access_token"]
        .as_str()
        .expect("access token")
        .into()
}

async fn send(
    app: &Router,
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
        Some(v) => builder
            .header("content-type", "application/json")
            .body(axum::body::Body::from(v.to_string()))
            .unwrap(),
        None => builder.body(axum::body::Body::empty()).unwrap(),
    };
    app.clone().oneshot(request).await.unwrap()
}

/// Admin bootstrap: org token + clients:write machine key + one client,
/// with the REAL router live on loopback for the sink to hit.
struct FeederOrg {
    token: String,
    api_key: String,
    client_id: Uuid,
    base_url: String,
    app: Router,
}

async fn setup(prefix: &str) -> FeederOrg {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let base_url = spawn_router(state).await;
    let slug = format!("{prefix}-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let bearer = [("authorization", format!("Bearer {token}"))];

    let client = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/v1/clients",
            &bearer,
            Some(json!({ "name": "Acme Systems" })),
        )
        .await,
    )
    .await;
    let client_id: Uuid = client["id"].as_str().unwrap().parse().unwrap();

    let key = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/v1/api-keys",
            &bearer,
            Some(json!({"name": "feeder-sync", "scopes": ["clients:write"]})),
        )
        .await,
    )
    .await;
    let api_key = key["raw_key"].as_str().expect("raw key shown once").into();

    FeederOrg {
        token,
        api_key,
        client_id,
        base_url,
        app,
    }
}

fn sink_for(org: &FeederOrg, api_key: &str) -> MeridianFeederSink {
    MeridianFeederSink::new(MeridianFeederConfig {
        base_url: org.base_url.clone(),
        api_key: api_key.to_string(),
    })
}

/// The org's item count + per-item source-link count, read through the
/// admin JWT (the connector never sees this surface).
async fn org_inventory(org: &FeederOrg) -> (usize, Vec<Value>) {
    let response = send(
        &org.app,
        axum::http::Method::GET,
        "/api/v1/config-items",
        &[("authorization", format!("Bearer {}", org.token))],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let items = body_json(response)
        .await
        .as_array()
        .cloned()
        .unwrap_or_default();
    (items.len(), items)
}

async fn item_view(org: &FeederOrg, item_id: Uuid) -> Value {
    let response = send(
        &org.app,
        axum::http::Method::GET,
        &format!("/api/v1/config-items/{item_id}"),
        &[("authorization", format!("Bearer {}", org.token))],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "item {item_id} must exist");
    body_json(response).await
}

fn ninjaone_device(id: &str, serial: &str, ip: &str) -> Value {
    json!({
        "id": id, "uid": format!("uid-{id}"),
        "displayName": format!("WS-{id}"),
        "systemName": "acme-ws-01", "domain": "acme.local",
        "osName": "Windows 11 Pro", "serialNumber": serial,
        "macAddresses": ["AA-BB-CC-DD-EE-FF"],
        "externalIp": ip, "lastSeen": "2026-09-12T09:30:00Z",
        "deviceClass": "WORKSTATION"
    })
}

/// IT Glue-shaped record built on the public feed model (the vendor
/// mapper ships in the marketplace `it-glue` connector).
fn it_glue_configuration(
    id: &str,
    name: &str,
    hostname: Option<&str>,
    serial: Option<&str>,
    primary_ip: Option<&str>,
) -> ci_feeders::model::FeederRecord {
    let mut identity = ci_feeders::identity::IdentityKeysBuilder::new().source_guid(id);
    if let Some(serial) = serial {
        identity = identity.serial(serial);
    }
    if let Some(hostname) = hostname {
        identity = identity.hostname(hostname);
    }
    let mut observed = serde_json::Map::new();
    if let Some(ip) = primary_ip {
        observed.insert("primary_ip".into(), json!(ip));
    }
    ci_feeders::model::FeederRecord {
        source_system: "it_glue".into(),
        external_id: id.into(),
        source_guid: Some(id.into()),
        payload_ref: None,
        tag: name.into(),
        name: name.into(),
        identity_keys: identity.build(),
        observed_fields: Value::Object(observed),
    }
}

/// Auvik-shaped record built on the public feed model (the vendor mapper
/// ships in the marketplace `auvik` connector).
fn auvik_device(
    id: &str,
    name: &str,
    serial: Option<&str>,
    mac: Option<&str>,
    domain: Option<&str>,
    ip: Option<&str>,
) -> ci_feeders::model::FeederRecord {
    let mut identity = ci_feeders::identity::IdentityKeysBuilder::new().source_guid(id);
    if let Some(serial) = serial {
        identity = identity.serial(serial);
    }
    if let Some(mac) = mac {
        identity = identity.macs([mac]);
    }
    identity = identity.hostname(name);
    if let Some(domain) = domain {
        identity = identity.domain(domain);
    }
    let mut observed = serde_json::Map::new();
    if let Some(ip) = ip {
        observed.insert("ipAddresses".into(), json!([ip]));
    }
    ci_feeders::model::FeederRecord {
        source_system: "auvix".into(),
        external_id: id.into(),
        source_guid: Some(id.into()),
        payload_ref: None,
        tag: name.into(),
        name: name.into(),
        identity_keys: identity.build(),
        observed_fields: Value::Object(observed),
    }
}

// --- gate 1: second sync updates its own link, never a twin -------------------

#[tokio::test]
async fn second_sync_updates_its_own_link_and_never_a_twin() {
    let org = setup("ci-gate-1").await;
    let sink = sink_for(&org, &org.api_key);

    let ninja = vendors::map_ninjaone_device(&ninjaone_device("111", "SN-111", "203.0.113.1"))
        .expect("ninjaone maps");
    let it_glue = it_glue_configuration(
        "501",
        "Docs Config 501",
        Some("docs-501.acme.local"),
        None,
        Some("10.0.5.1"),
    );
    let auvix = auvik_device("auv-777", "acme-switch-01", None, None, None, Some("10.0.1.1"));

    // First sync per feeder: three distinct records with no shared
    // signals — three distinct canonical items, one link each.
    let report = FeederSync::run(
        &SyncConfig::new(org.client_id),
        &[ninja.clone(), it_glue.clone(), auvix.clone()],
        &sink,
    )
    .await;
    assert!(report.is_clean(), "first sync clean: {:?}", report.errors);
    assert_eq!(report.synced, 3);
    assert_eq!(report.created, 3, "three distinct items, never blind");

    let (count, _) = org_inventory(&org).await;
    assert_eq!(count, 3, "no feeder created a duplicate");

    // Second sync per feeder with CHANGED content (observed churn).
    let ninja_v2 =
        vendors::map_ninjaone_device(&ninjaone_device("111", "SN-111", "203.0.113.2")).unwrap();
    let it_glue_v2 = it_glue_configuration(
        "501",
        "Docs Config 501",
        Some("docs-501.acme.local"),
        None,
        Some("10.0.5.9"),
    );
    let auvix_v2 =
        auvik_device("auv-777", "acme-switch-01", None, None, None, Some("10.0.1.2"));

    let report2 = FeederSync::run(
        &SyncConfig::new(org.client_id),
        &[ninja_v2, it_glue_v2, auvix_v2],
        &sink,
    )
    .await;
    assert!(
        report2.is_clean(),
        "second sync clean: {:?}",
        report2.errors
    );
    assert_eq!(report2.created, 0, "a second sync never creates a twin");
    assert_eq!(
        report2.resolved_existing, 0,
        "same-source sync resolves its OWN link"
    );

    let (count, _) = org_inventory(&org).await;
    assert_eq!(count, 3, "still exactly three items");

    // Each item carries exactly ONE link — its own, refreshed.
    for (tag, source) in [
        ("WS-111", "ninjaone"),
        ("Docs Config 501", "it_glue"),
        ("acme-switch-01", "auvix"),
    ] {
        let (_, items) = org_inventory(&org).await;
        let item = items
            .iter()
            .find(|i| i["tag"] == json!(tag))
            .cloned()
            .expect("the source's item");
        let item_id: Uuid = item["id"].as_str().unwrap().parse().unwrap();
        let view = item_view(&org, item_id).await;
        let links = view["source_links"].as_array().unwrap();
        assert_eq!(links.len(), 1, "{tag}: exactly one link, no duplicates");
        assert_eq!(links[0]["source_system"], json!(source));
    }
}

// --- gate 2: no cross-connector adoption; deterministic convergence -----------

#[tokio::test]
async fn sources_converge_on_one_item_with_one_link_per_source() {
    let org = setup("ci-gate-2").await;
    let sink = sink_for(&org, &org.api_key);

    // IT Glue documents the device first.
    let it_glue = it_glue_configuration(
        "601",
        "Mail Server",
        Some("mail.acme.local"),
        Some("SN-ABC-123"),
        None,
    );
    let report = FeederSync::run(&SyncConfig::new(org.client_id), &[it_glue], &sink).await;
    assert_eq!(report.created, 1);
    let (_, items) = org_inventory(&org).await;
    let item_id: Uuid = items[0]["id"].as_str().unwrap().parse().unwrap();
    let it_glue_link_id: Uuid = item_view(&org, item_id).await["source_links"][0]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    // NinjaOne feeds the SAME physical device — the serial matches at
    // 0.95 (deterministic): the core ADOPTS the existing item and the
    // ninjaone record gets its OWN link on it. One canonical item, one
    // link per source, never one connector grabbing the other's link.
    let ninja =
        vendors::map_ninjaone_device(&ninjaone_device("900", " sn abc 123 ", "203.0.113.9"))
            .unwrap();
    let report2 = FeederSync::run(&SyncConfig::new(org.client_id), &[ninja], &sink).await;
    assert!(
        report2.is_clean(),
        "ninjaone sync clean: {:?}",
        report2.errors
    );
    assert_eq!(report2.created, 0, "no twin");
    assert_eq!(
        report2.resolved_existing, 1,
        "deterministic adoption with evidence"
    );
    assert_eq!(
        report2.auto_merged, 1,
        "serial 0.95 clears the 0.85 default"
    );

    let view = item_view(&org, item_id).await;
    let links = view["source_links"].as_array().unwrap();
    assert_eq!(links.len(), 2, "one link per source on ONE canonical item");
    assert!(
        links
            .iter()
            .any(|l| l["source_system"] == json!("ninjaone")),
        "ninjaone has its own link"
    );
    assert!(
        links.iter().any(|l| l["source_system"] == json!("it_glue")),
        "it_glue keeps its own link"
    );
    let ninja_link_id: Uuid = links
        .iter()
        .find(|l| l["source_system"] == json!("ninjaone"))
        .map(|l| l["id"].as_str().unwrap().parse().unwrap())
        .unwrap();
    assert_ne!(ninja_link_id, it_glue_link_id, "distinct links per source");

    // NinjaOne's second sync updates ONLY its own link — the IT Glue
    // link id stays byte-identical.
    let ninja_v2 =
        vendors::map_ninjaone_device(&ninjaone_device("900", "SN-ABC-123", "203.0.113.10"))
            .unwrap();
    let report3 = FeederSync::run(&SyncConfig::new(org.client_id), &[ninja_v2], &sink).await;
    assert!(report3.is_clean());
    assert_eq!(report3.created, 0);

    let view = item_view(&org, item_id).await;
    let links = view["source_links"].as_array().unwrap();
    assert_eq!(links.len(), 2, "still two links, no duplicates");
    let ninja_link_id_after: Uuid = links
        .iter()
        .find(|l| l["source_system"] == json!("ninjaone"))
        .map(|l| l["id"].as_str().unwrap().parse().unwrap())
        .unwrap();
    let it_glue_link_id_after: Uuid = links
        .iter()
        .find(|l| l["source_system"] == json!("it_glue"))
        .map(|l| l["id"].as_str().unwrap().parse().unwrap())
        .unwrap();
    assert_eq!(
        ninja_link_id, ninja_link_id_after,
        "ninjaone updates ITS link"
    );
    assert_eq!(
        it_glue_link_id, it_glue_link_id_after,
        "it_glue's link untouched"
    );

    // The same external id under a DIFFERENT source is a distinct link
    // (the CI2 unique key is (org, source_system, external_id)).
    let auvix_same_id = auvik_device("601", "auvik-probe", None, None, None, None);
    let report4 = FeederSync::run(&SyncConfig::new(org.client_id), &[auvix_same_id], &sink).await;
    assert!(report4.is_clean());
    assert_eq!(
        report4.created, 1,
        "different source, same external id: own item"
    );
}

// --- gate 3: identity keys deterministic under payload reformatting ----------

#[tokio::test]
async fn reformatted_payloads_resolve_to_the_same_item() {
    let org = setup("ci-gate-3").await;
    let sink = sink_for(&org, &org.api_key);

    let first = auvik_device(
        "auv-1",
        "acme-mail-01",
        Some("SN-8F3K2-Q"),
        Some("AABBCCDDEEFF"),
        Some("acme.local"),
        Some("10.0.0.1"),
    );
    let report = FeederSync::run(&SyncConfig::new(org.client_id), &[first], &sink).await;
    assert_eq!(report.created, 1);
    let (_, items) = org_inventory(&org).await;
    let item_id: Uuid = items[0]["id"].as_str().unwrap().parse().unwrap();

    // The SAME physical device re-fed under a NEW external id with every
    // formatting quirk flipped — the normalized keys still hit 0.95.
    let second = auvik_device(
        "auv-2",
        "ACME-MAIL-01.",
        Some(" sn8f3k2q "),
        Some("aa:bb:cc:dd:ee:ff"),
        Some("ACME.LOCAL."),
        Some("10.0.0.2"),
    );
    let report2 = FeederSync::run(&SyncConfig::new(org.client_id), &[second], &sink).await;
    assert!(report2.is_clean());
    assert_eq!(report2.created, 0, "no twin under reformatted identity");
    assert_eq!(report2.resolved_existing, 1);

    let (count, _) = org_inventory(&org).await;
    assert_eq!(count, 1, "deterministic keys, not luck");
    let view = item_view(&org, item_id).await;
    assert_eq!(
        view["source_links"].as_array().unwrap().len(),
        2,
        "both auvix records linked to the ONE item"
    );
}

// --- gate 4: idempotent upsert ------------------------------------------------

#[tokio::test]
async fn unchanged_replay_is_idempotent() {
    let org = setup("ci-gate-4").await;
    let sink = sink_for(&org, &org.api_key);

    let ninja =
        vendors::map_ninjaone_device(&ninjaone_device("222", "SN-222", "203.0.113.3")).unwrap();

    let first = sink
        .feed_upsert(&ninja, org.client_id)
        .await
        .expect("first");
    assert!(first.created_item);
    // The SAME record (byte-identical semantic content) again: the
    // deterministic idempotency key replays the stored result — same
    // item, no new write, and the link stays the one already bound.
    let second = sink
        .feed_upsert(&ninja, org.client_id)
        .await
        .expect("replay");
    assert_eq!(second.config_item_id, first.config_item_id);
    assert!(!second.created_item);

    let (count, _) = org_inventory(&org).await;
    assert_eq!(count, 1);
    let view = item_view(&org, first.config_item_id).await;
    let links = view["source_links"].as_array().unwrap();
    assert_eq!(links.len(), 1, "the replay never churned the link");
    let link_id: Uuid = links[0]["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(link_id, first.source_link_id);
}

// --- gate 5: never blind -------------------------------------------------------

#[tokio::test]
async fn signal_less_record_still_lands_with_its_link() {
    let org = setup("ci-gate-5").await;
    let sink = sink_for(&org, &org.api_key);

    // No serial, no MAC, no hostname — the feeder still never blind-
    // inserts: the core creates the item WITH its link in one write.
    let bare = it_glue_configuration("701", "Mystery appliance", None, None, None);
    let report = FeederSync::run(&SyncConfig::new(org.client_id), &[bare], &sink).await;
    assert!(report.is_clean());
    assert_eq!(report.created, 1);

    let (_, items) = org_inventory(&org).await;
    let item_id: Uuid = items[0]["id"].as_str().unwrap().parse().unwrap();
    let view = item_view(&org, item_id).await;
    let links = view["source_links"].as_array().unwrap();
    assert_eq!(links.len(), 1, "the link is always written");
    assert_eq!(links[0]["external_id"], json!("701"));

    // Re-sync updates that same link (no signals, but the CI2 key holds).
    let bare_v2 = it_glue_configuration("701", "Mystery appliance II", None, None, None);
    let report2 = FeederSync::run(&SyncConfig::new(org.client_id), &[bare_v2], &sink).await;
    assert_eq!(report2.created, 0);
    let view2 = item_view(&org, item_id).await;
    assert_eq!(view2["source_links"].as_array().unwrap().len(), 1);
}

// --- gate 6: scoped machine key posture ----------------------------------------

#[tokio::test]
async fn read_scoped_key_is_refused_the_feeder_write() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let base_url = spawn_router(state).await;
    let slug = format!("ci-gate-6-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let bearer = [("authorization", format!("Bearer {token}"))];

    let client = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/v1/clients",
            &bearer,
            Some(json!({ "name": "Acme" })),
        )
        .await,
    )
    .await;
    let client_id: Uuid = client["id"].as_str().unwrap().parse().unwrap();

    let key = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/v1/api-keys",
            &bearer,
            Some(json!({"name": "read-only", "scopes": ["clients:read"]})),
        )
        .await,
    )
    .await;
    let read_key = key["raw_key"].as_str().unwrap().to_string();

    let record =
        vendors::map_ninjaone_device(&ninjaone_device("333", "SN-333", "203.0.113.4")).unwrap();
    let sink = sink_for_by_base(&base_url, &read_key);
    let report = FeederSync::run(&SyncConfig::new(client_id), &[record], &sink).await;
    assert_eq!(report.synced, 0, "read-scoped keys cannot feed");
    assert_eq!(report.errors.len(), 1);
    assert!(
        report.errors[0]
            .error
            .to_lowercase()
            .contains("unauthorized")
            || report.errors[0].error.to_lowercase().contains("denied")
            || report.errors[0].error.to_lowercase().contains("forbidden"),
        "typed refusal, got: {}",
        report.errors[0].error
    );
}

fn sink_for_by_base(base_url: &str, api_key: &str) -> MeridianFeederSink {
    MeridianFeederSink::new(MeridianFeederConfig {
        base_url: base_url.to_string(),
        api_key: api_key.to_string(),
    })
}
