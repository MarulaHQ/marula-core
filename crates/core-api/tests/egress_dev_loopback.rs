//! F64 — the dev-gated loopback allowance for the calendar-sync E2E feed.
//!
//! The calendar E2E serves an iCal feed on loopback and the egress guard refuses
//! it (`egress_private_address`), correctly: a loopback feed is the only thing a
//! test can serve offline. These cases drive the same guard the calendar poll
//! uses (`core_api::outbound`) against a real loopback listener, so "admitted"
//! and "refused" are measured as connections rather than inferred.
//!
//! The refusal half is asserted on the typed denial, not on `is_err()`, so a
//! future relaxation that widens the allowance beyond loopback fails here
//! instead of silently shipping.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use core_api::outbound::{guard, guard_under};
use core_domain::egress::{AddressPolicy, EgressDenial};

/// A loopback HTTP server that answers one iCal body and counts connections.
///
/// The count is the assertion: "admitted" means the socket was reached, and
/// "refused" means it was never dialled.
fn feed_listener() -> (std::net::SocketAddr, Arc<AtomicUsize>) {
    use std::io::{Read, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
    let addr = listener.local_addr().expect("listener address");
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&hits);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            seen.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/calendar\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
            );
        }
    });
    (addr, hits)
}

/// The E2E's case: under the dev allowance a loopback iCal feed is fetched,
/// once — and over the same guard the poll uses, with the client pinned and
/// redirects still refused.
#[tokio::test]
async fn the_dev_allowance_admits_a_loopback_feed() {
    let (addr, hits) = feed_listener();
    let url = format!("http://{addr}/feed.ics");

    // First the refusal, so the acceptance is a difference and not luck: the
    // same URL is `egress_private_address` with no allowance.
    assert_eq!(
        guard(&url).await.err(),
        Some(EgressDenial::PrivateAddress),
        "the shipped guard refuses the loopback feed"
    );

    let guarded = guard_under(&url, AddressPolicy::DevLoopback)
        .await
        .expect("the dev allowance admits a loopback feed");

    let response = guarded
        .client(Duration::from_secs(5))
        .expect("the client carries the pin")
        .get(guarded.url())
        .send()
        .await
        .expect("the loopback feed answers");
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "the feed was fetched exactly once"
    );
}

/// The other half of the ruling: loopback is the WHOLE allowance. Every private
/// range, link-local and the metadata endpoint stays refused, with the typed
/// denial that says the address rule fired rather than the scheme or the port.
#[tokio::test]
async fn the_allowance_admits_nothing_beyond_loopback() {
    for url in [
        "http://10.0.0.1/feed.ics",
        "http://172.16.4.4/feed.ics",
        "http://192.168.1.1/feed.ics",
        "http://100.64.0.1/feed.ics",
        "http://169.254.169.254/latest/meta-data/",
        "http://[fd00:ec2::254]/latest/meta-data/",
        "http://[fe80::1]/feed.ics",
        "http://0.0.0.0/feed.ics",
    ] {
        assert_eq!(
            guard_under(url, AddressPolicy::DevLoopback).await.err(),
            Some(EgressDenial::PrivateAddress),
            "{url} must stay refused even with the allowance"
        );
    }
}
