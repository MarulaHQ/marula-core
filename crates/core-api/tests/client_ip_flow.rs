//! Trusted-proxy client-IP resolution, driven through the REAL router on a
//! REAL socket with axum connect info (Wave 47 lane A10,
//! `p47-trusted-proxy-client-ip`).
//!
//! Why this file exists alongside `client_ip`'s unit tests: the unit tests
//! prove the resolver matrix, but only a live connection can prove the
//! wiring — that the binary's serving path hands the DIRECT PEER to the
//! resolver and publishes the answer as a `ClientIp` extension every
//! handler reads. The observable surface is the durable per-IP half of the
//! refresh gate (`GateSubject::IpOnly` — refresh is the one throttled
//! surface whose key set no longer has a per-actor member once a session
//! resolves, so the per-IP key is the half these cases can see in the
//! status codes — still the outer bound).
//!
//! Every case sends its own randomly-suffixed client block: the per-IP
//! counters are DURABLE and DB-wide, so a fixed IP would collide with
//! parallel runs (the same discipline `auth_hardening_flow.rs` follows).
//! Addresses that only need to be private to a case come from the shared
//! fixture (`support/caller.rs`, Wave 54 follow-up F7); this file keeps its
//! own paired addresses because its subject is WHICH BUCKET a caller lands in,
//! and two requests deliberately sharing one is the thing under test.
//!
//! The per-IP keys stay noise-only. These tests assert throttle *key
//! quality* (which bucket a caller lands in), never that the per-IP key
//! blocks an account: per-actor keys remain the lock.

use core_api::{client_ip::TrustedProxies, config::Config, state::AppState};
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

// --- harness ----------------------------------------------------------------

async fn test_state(admin_url: &str) -> AppState {
    let pool: PgPool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(10))
        .connect(admin_url)
        .await
        .expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("config");
    config.database_url = admin_url.to_string();
    // Per-instance salt so durable per-IP hashes (especially the bare
    // loopback peer 127.0.0.1) never collide across parallel test binaries
    // or repeated runs against the shared test database.
    config.jwt_secret = format!("client-ip-flow-{}", Uuid::now_v7());
    // Pinned, not inherited: the loopback peer of this test's own socket is
    // TRUSTED and RFC1918 hops are trusted, which is the posture under test.
    // An operator's ambient TRUSTED_PROXIES must not shape the assertions.
    config.trusted_proxies = TrustedProxies::default();
    AppState {
        plugin_pool: Default::default(),
        pool,
        config: Arc::new(config),
        oidc: core_api::auth::oidc_client::OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(
            &Config::from_env_allow_dev_fallback().expect("config"),
        )),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(admin_url)
            .expect("lazy reporting pool"),
    }
}

/// Boots the router the way `core-api`'s binary does: with connect info, so
/// every request carries its direct peer.
async fn serve(admin_url: &str) -> String {
    let app = core_api::router(test_state(admin_url).await);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("serve");
    });
    format!("http://{addr}")
}

/// A unique public /24 per test invocation (outside RFC1918 and Cloudflare
/// ranges), so durable per-IP counters cannot bleed between parallel tests
/// or repeated runs against the same database.
fn unique_block(test_case: u8) -> (u8, u8) {
    let third = (Uuid::now_v7().as_u128() % 250 + 1) as u8;
    (18u8.saturating_add(test_case), third)
}

/// One valid IPv4 address inside a test's block, chosen by its last octet.
fn client_ip(block: (u8, u8), last_octet: u8) -> String {
    format!("198.{}.{}.{last_octet}", block.0, block.1)
}

/// One refresh attempt with an arbitrary `X-Forwarded-For` chain (leftmost
/// hop first, exactly as the header is written) and optional
/// `CF-Connecting-IP`. The token is garbage on purpose: an unresolvable
/// session id still records a failure on the per-IP key, which is what the
/// gate curve is observed through.
async fn refresh_attempt(
    http: &reqwest::Client,
    base: &str,
    xff: Option<&str>,
    cf_connecting_ip: Option<&str>,
) -> reqwest::StatusCode {
    let mut request = http.post(format!("{base}/api/auth/refresh")).json(&json!({
        "refresh_token": "0".repeat(64),
        "session_id": Uuid::now_v7().to_string(),
    }));
    if let Some(chain) = xff {
        request = request.header("x-forwarded-for", chain);
    }
    if let Some(ip) = cf_connecting_ip {
        request = request.header("cf-connecting-ip", ip);
    }
    request.send().await.expect("refresh request").status()
}

// ---------------------------------------------------------------------------
// The right-to-left walk: the rightmost hop a trusted proxy chain leaves
// behind is the client, and everything to its LEFT is attacker-pinnable.
// ---------------------------------------------------------------------------

/// The header's leftmost entry is whatever the caller typed; the real
/// client sits to the right of it, and the inner load balancer's own
/// address sits rightmost (trusted, so it is skipped).
#[tokio::test]
async fn the_per_ip_gate_follows_the_rightmost_untrusted_hop_not_the_pinned_first_hop() {
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let base = serve(&admin_url).await;
    let http = reqwest::Client::new();
    let block = unique_block(10);
    let client = client_ip(block, 7);
    let balancer = "10.0.0.9";

    // Attempt 1: the caller pinned a fake first hop; the real client is the
    // middle entry and the trusted balancer appended its own address.
    let first = refresh_attempt(
        &http,
        &base,
        Some(&format!("1.2.3.4, {client}, {balancer}")),
        None,
    )
    .await;
    assert_eq!(first, reqwest::StatusCode::UNAUTHORIZED);

    // Attempt 2: a DIFFERENT fake first hop, same client and same balancer.
    // The bucket must still be the client's — a resolver that took the
    // first hop (the old behavior) would key this one elsewhere and answer
    // 401 instead of the engaged 429.
    let second = refresh_attempt(
        &http,
        &base,
        Some(&format!("9.9.9.9, {client}, {balancer}")),
        None,
    )
    .await;
    assert_eq!(second, reqwest::StatusCode::TOO_MANY_REQUESTS);
}

/// Trusted hops are skipped, not consulted: the same real client behind a
/// different trusted proxy lands in the SAME bucket.
#[tokio::test]
async fn trusted_hops_are_skipped_so_the_surviving_client_key_is_stable() {
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let base = serve(&admin_url).await;
    let http = reqwest::Client::new();
    let block = unique_block(11);
    let client = client_ip(block, 21);

    let first = refresh_attempt(
        &http,
        &base,
        Some(&format!("1.2.3.4, {client}, 10.0.0.9")),
        None,
    )
    .await;
    assert_eq!(first, reqwest::StatusCode::UNAUTHORIZED);

    // Same client, DIFFERENT trusted proxy (private range, so still
    // trusted) ⇒ same key ⇒ the gate is already engaged.
    let second = refresh_attempt(
        &http,
        &base,
        Some(&format!("1.2.3.4, {client}, 192.168.1.8")),
        None,
    )
    .await;
    assert_eq!(second, reqwest::StatusCode::TOO_MANY_REQUESTS);
}

/// Distinct real clients behind the same trusted proxy keep distinct
/// buckets — the walk does not collapse a whole edge onto one key.
#[tokio::test]
async fn distinct_clients_behind_one_trusted_proxy_get_distinct_buckets() {
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let base = serve(&admin_url).await;
    let http = reqwest::Client::new();
    let block = unique_block(12);
    let first_client = client_ip(block, 31);
    let second_client = client_ip(block, 32);

    let first = refresh_attempt(
        &http,
        &base,
        Some(&format!("{first_client}, 10.0.0.9")),
        None,
    )
    .await;
    assert_eq!(first, reqwest::StatusCode::UNAUTHORIZED);

    // A different client ⇒ a different bucket ⇒ not throttled yet.
    let second = refresh_attempt(
        &http,
        &base,
        Some(&format!("{second_client}, 10.0.0.9")),
        None,
    )
    .await;
    assert_eq!(second, reqwest::StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// The resolver as a TABLE over real chains, through a real socket.
//
// Each row sends two refresh attempts whose `X-Forwarded-For` chains differ in
// the part a caller types and agree (or deliberately disagree) on the part a
// proxy appended. The per-IP gate is the readout: two chains that resolve to
// the same client land in one bucket, so the second attempt meets the engaged
// lock; chains that resolve to different clients do not.
//
// The socket peer of this harness is loopback, which the default trust set
// covers, so every row is the production shape — a trusted proxy in front of
// the API — rather than the peer-less `oneshot` posture.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_trusted_walk_resolves_each_real_chain_shape() {
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let base = serve(&admin_url).await;
    let http = reqwest::Client::new();

    // Placeholders are filled per row so each row gets its own address block
    // (the per-IP counters are durable and DB-wide).
    struct Row {
        name: &'static str,
        /// Last octets of the two client addresses this row may use.
        clients: (u8, u8),
        first: String,
        second: String,
        /// Do the two chains resolve to the same client?
        same_bucket: bool,
    }

    let rows = vec![
        Row {
            name: "a pinned prefix cannot move the bucket when the client and the proxy agree",
            clients: (51, 52),
            first: "1.1.1.1, {c1}, 10.0.0.9".to_owned(),
            second: "9.9.9.9, {c1}, 10.0.0.9".to_owned(),
            same_bucket: true,
        },
        Row {
            name: "a chain of trusted proxies collapses onto the client behind them",
            clients: (53, 54),
            first: "{c1}, 10.0.0.9, 192.168.1.2".to_owned(),
            second: "{c1}, 10.0.1.1, 192.168.7.7".to_owned(),
            same_bucket: true,
        },
        Row {
            name: "two distinct clients behind one proxy stay distinct",
            clients: (55, 56),
            first: "{c1}, 10.0.0.9".to_owned(),
            second: "{c2}, 10.0.0.9".to_owned(),
            same_bucket: false,
        },
        Row {
            name: "a port suffix on the appended hop does not change the client",
            clients: (57, 58),
            first: "{c1}:41234, 10.0.0.9".to_owned(),
            second: "{c1}, 10.0.0.9".to_owned(),
            same_bucket: true,
        },
        Row {
            name: "an unparseable entry in the client's half is skipped, not believed",
            clients: (59, 60),
            first: "not-an-address, {c1}, 10.0.0.9".to_owned(),
            second: "{c1}, 10.0.0.9".to_owned(),
            same_bucket: true,
        },
        Row {
            name: "a caller padding the chain past the cap still resolves to the appended client",
            clients: (61, 62),
            first: format!("{}, {{c1}}, 10.0.0.9", "198.18.0.9, ".repeat(80).trim_end()),
            second: "{c1}, 10.0.0.9".to_owned(),
            same_bucket: true,
        },
    ];

    let mut failures = Vec::new();
    for (index, row) in rows.into_iter().enumerate() {
        let block = unique_block(20 + index as u8);
        let (first_octet, second_octet) = row.clients;
        let c1 = client_ip(block, first_octet);
        let c2 = client_ip(block, second_octet);
        let fill = |chain: &str| chain.replace("{c1}", &c1).replace("{c2}", &c2);

        let first_status = refresh_attempt(&http, &base, Some(&fill(&row.first)), None).await;
        let second_status = refresh_attempt(&http, &base, Some(&fill(&row.second)), None).await;
        let expected = if row.same_bucket {
            reqwest::StatusCode::TOO_MANY_REQUESTS
        } else {
            reqwest::StatusCode::UNAUTHORIZED
        };
        if first_status != reqwest::StatusCode::UNAUTHORIZED || second_status != expected {
            failures.push(format!(
                "{}: first {first_status} (want 401), second {second_status} (want {expected})",
                row.name
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "chain shapes resolved to the wrong bucket:\n{}",
        failures.join("\n")
    );
}

// ---------------------------------------------------------------------------
// No proxy header: the peer address itself is the answer, so every caller
// on the same socket shares one bucket.
//
// This is the INTENTIONAL shared-bucket case (Wave 54 follow-up F7). Every
// other suite stamps a caller of its own and never relies on this; here the
// sharing IS the assertion — two headerless requests from one peer must land
// on one key. `caller_fixture::headerless_oneshot_requests_share_the_unknown_bucket`
// pins the sibling fact for the harness posture, where there is no peer at
// all. Do not "fix" this case by giving each attempt its own address: that
// would delete the evidence that the peer is the key.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn without_a_proxy_header_the_peer_address_is_the_client() {
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let base = serve(&admin_url).await;
    let http = reqwest::Client::new();

    let first = refresh_attempt(&http, &base, None, None).await;
    assert_eq!(first, reqwest::StatusCode::UNAUTHORIZED);

    // Second caller from the same peer: the peer IS the key, so the gate
    // engages. A header-blind resolver would have kept a shared "unknown"
    // bucket here — the peer address is strictly more honest.
    let second = refresh_attempt(&http, &base, None, None).await;
    assert_eq!(second, reqwest::StatusCode::TOO_MANY_REQUESTS);
}

// ---------------------------------------------------------------------------
// Cloudflare: `auto` believes `CF-Connecting-IP` only from a peer that is
// BOTH trusted and inside a known Cloudflare range. A loopback test peer is
// neither, so the header must be ignored and the XFF walk must win.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cf_connecting_ip_is_not_believed_from_a_non_cloudflare_peer() {
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let base = serve(&admin_url).await;
    let http = reqwest::Client::new();
    let block = unique_block(13);
    let walked_client = client_ip(block, 41);
    let cf_claimed = client_ip(block, 42);

    let first = refresh_attempt(
        &http,
        &base,
        Some(&format!("{walked_client}, 10.0.0.9")),
        Some(&cf_claimed),
    )
    .await;
    assert_eq!(first, reqwest::StatusCode::UNAUTHORIZED);

    // Same CF claim, different walked client: if the header had been
    // believed, both attempts would share the claimed IP and this one would
    // be throttled. It is not — the XFF walk decided.
    let second = refresh_attempt(
        &http,
        &base,
        Some(&format!("{}, 10.0.0.9", client_ip(block, 43))),
        Some(&cf_claimed),
    )
    .await;
    assert_eq!(second, reqwest::StatusCode::UNAUTHORIZED);
}
