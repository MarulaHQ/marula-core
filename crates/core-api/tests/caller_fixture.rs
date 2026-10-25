//! The shared caller-identity fixture's contract (Wave 54 follow-up F7).
//!
//! These cases are DB-free on purpose: the fixture is what the auth flow
//! suites rely on, so its guarantee is proved in the ordinary quick suite
//! rather than in the group gate. The one database-shaped fact in here — that
//! headerless `oneshot` requests share one bucket — is asserted against the
//! resolver itself, which is pure.

#[path = "support/caller.rs"]
mod caller;

use caller::{fresh_caller_ip, CallerIp};
use core_api::client_ip::{TrustedProxies, UNKNOWN};
use std::collections::HashSet;
use std::net::Ipv4Addr;

/// The collision that started this: a case's second call must not answer with
/// the first call's address, or a durable lock armed by one case refuses
/// another case's sign-in.
///
/// Serial, like every other case here that mints: the counter is per process,
/// so a sibling test running on another thread takes slots between two calls
/// the assertions below want to compare.
#[test]
#[serial_test::serial]
fn every_call_mints_a_distinct_address() {
    let mut seen = HashSet::new();
    for call in 0..4096 {
        let ip = fresh_caller_ip();
        assert!(
            seen.insert(ip.to_string()),
            "call {call} reissued {ip} — two cases would share one caller bucket"
        );
    }
}

/// Threads in one test binary run in parallel, so the counter has to be the
/// thing that separates them, not luck.
#[test]
#[serial_test::serial]
fn parallel_callers_draw_disjoint_addresses() {
    let handles: Vec<_> = (0..8)
        .map(|_| {
            std::thread::spawn(|| {
                (0..128)
                    .map(|_| fresh_caller_ip().to_string())
                    .collect::<Vec<_>>()
            })
        })
        .collect();

    let mut seen = HashSet::new();
    for handle in handles {
        for ip in handle.join().expect("minting thread") {
            assert!(seen.insert(ip.clone()), "{ip} was handed to two callers");
        }
    }
    assert_eq!(seen.len(), 8 * 128);
}

/// The address has to parse, live in the RFC 2544 benchmarking block, and be
/// untrusted — the resolver only follows a forwarded hop when it is not
/// trusted, and a private or loopback address would be skipped instead.
#[test]
#[serial_test::serial]
fn the_address_is_a_valid_untrusted_ipv4_in_the_benchmark_block() {
    let ip: Ipv4Addr = fresh_caller_ip()
        .as_str()
        .parse()
        .expect("a caller address must be a valid IPv4");
    let octets = ip.octets();
    assert_eq!(octets[0], 198, "{ip} is outside 198.18.0.0/15");
    assert!(
        octets[1] == 18 || octets[1] == 19,
        "{ip} is outside 198.18.0.0/15"
    );
    assert!(
        !ip.is_private() && !ip.is_loopback() && !ip.is_link_local(),
        "{ip} would be skipped by the trusted-proxy walk"
    );
    assert_eq!(
        TrustedProxies::default().resolve(Some(ip.into()), &Default::default()),
        ip.to_string(),
        "an untrusted peer is its own client, so the bucket is the minted address"
    );
}

/// The Wave 54 defect in one assertion: the old helper read the leading bytes
/// of a v7 UUID, which are the millisecond timestamp, so every call inside one
/// run produced the same address. The replacement is driven by its call
/// counter, so back-to-back calls differ and the counter is visible in the
/// address instead of the clock.
#[test]
#[serial_test::serial]
fn the_address_comes_from_the_call_counter_not_the_clock() {
    let first = fresh_caller_ip();
    let second = fresh_caller_ip();
    assert_ne!(
        first.as_str(),
        second.as_str(),
        "two calls must never share an address, whatever the millisecond"
    );

    let slot = |ip: &CallerIp| -> u32 {
        let octets = ip.as_str().parse::<Ipv4Addr>().expect("IPv4").octets();
        (u32::from(octets[1] - 18) << 16) | (u32::from(octets[2]) << 8) | u32::from(octets[3])
    };
    assert_eq!(
        (slot(&first) + 1) % 131_072,
        slot(&second),
        "consecutive calls take consecutive slots of the block"
    );
}

/// Stamping is what the flow suites use. A request that already names its
/// caller keeps that name: a case exercising one stable identity across
/// several requests must not be quietly re-keyed.
#[test]
#[serial_test::serial]
fn stamping_fills_a_headerless_request_and_leaves_an_explicit_one_alone() {
    let request = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri("/api/portal/login")
        .body(())
        .expect("request");
    let ip = fresh_caller_ip();
    let stamped = ip.stamp(request);
    assert_eq!(
        stamped
            .headers()
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok()),
        Some(ip.as_str()),
        "a headerless request must carry the minted caller"
    );
    assert_eq!(
        ip.forwarded_for(),
        ("x-forwarded-for", ip.as_str()),
        "the tuple form is the header pair the send helpers take"
    );

    let pinned = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri("/api/portal/login")
        .header("x-forwarded-for", "203.0.113.7")
        .body(())
        .expect("request");
    let untouched = fresh_caller_ip().stamp(pinned);
    assert_eq!(
        untouched
            .headers()
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok()),
        Some("203.0.113.7"),
        "an explicit caller survives the stamp"
    );
}

/// The intentional shared bucket, pinned here so the fixture's reason cannot
/// be lost: a `oneshot` request with no socket peer and no proxy header
/// resolves to one `unknown` key, which every headerless send in a binary
/// shares. This is a resolver fact, not a test fixture — the suites stamp a
/// caller instead of relying on it.
#[test]
fn headerless_oneshot_requests_share_the_unknown_bucket() {
    let resolver = TrustedProxies::default();
    let first = resolver.resolve(None, &Default::default());
    let second = resolver.resolve(None, &Default::default());
    assert_eq!(first, UNKNOWN);
    assert_eq!(
        first, second,
        "two callers with nothing to attribute share this one key"
    );
}
