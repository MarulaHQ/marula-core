//! Guarded outbound fetch for the product edges (Wave 54 lane S1a).
//!
//! ## One control, five call sites
//!
//! The decision lives in `core_domain::egress`; this module is the only place a
//! non-plugin caller turns that decision into a request. Before Wave 54 the KB
//! link-check, the KB oEmbed fetch, the calendar iCal poll and the outbound
//! webhook each built a client and connected to whatever URL their row held, so
//! a stored `http://127.0.0.1:…` or `http://169.254.169.254/…` was a request the
//! process made on the row's behalf. `plugin_http.rs` already screened with the
//! same guard; these four edges now do too.
//!
//! Wave 55 added the fifth: the plugin distribution fetch, whose URL arrives in
//! a request body rather than in a row. It screens with the seam's
//! https-only/no-port shape through [`guard_with`], and inherits steps 2 to 4
//! unchanged.
//!
//! ## Order, and what each step stops
//!
//! 1. **`screen_fetch`** refuses a URL that carries userinfo or names a host
//!    that is not publicly routable. A private or metadata address spelled as a
//!    literal, as `2130706433`, or as a name on the denied-suffix list never
//!    reaches a socket. The scheme and the port are left to the product surface
//!    on purpose: a public `http` link and a calendar feed on `:8443` are not
//!    SSRF, and the plugin seam's https-only/no-port rule is the consent shape
//!    of an allowlist entry, not the address rule.
//! 2. **Resolve once** and judge every answer with [`egress::check_resolved`].
//!    A name that resolves to one public and one private address is refused:
//!    the private one is still reachable. An empty answer is a refusal.
//! 3. **Pin** the connection to exactly the judged addresses (`resolve_to_addrs`),
//!    so the address the guard approved is the address the transport dials. TLS
//!    still verifies the certificate against the hostname.
//! 4. **No redirects.** The client is built with `Policy::none`, so a `3xx`
//!    arrives as a response and the caller judges it with
//!    [`egress::is_redirect`]. A hop would be a second host nothing screened.
//!
//! Re-resolving is per attempt, never cached across attempts: a cached answer
//! is the window a DNS-rebinding attacker needs.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use core_domain::egress::{self, EgressDenial, EgressTarget};
use reqwest::redirect::Policy;

/// A URL the guard accepted, with the addresses its host resolved to.
///
/// The address set is empty for an IP-literal target: `screen_fetch` already
/// classified the literal, so there is nothing to look up and nothing to pin.
pub struct Guarded {
    target: EgressTarget,
    addrs: Vec<SocketAddr>,
}

impl Guarded {
    /// The canonical URL to fetch. This is the guard's own serialization, not
    /// the row's string, so a parser differential cannot authorize one URL and
    /// fetch another.
    pub fn url(&self) -> &str {
        self.target.as_str()
    }

    pub fn host(&self) -> &str {
        self.target.host()
    }

    /// A client pinned to the judged addresses with redirects refused.
    ///
    /// One client per fetch: the pin is per address set, and a client that
    /// outlived a re-resolution would be a client pinned to a stale answer. The
    /// edges that use this are not hot paths (a link check, an oEmbed preview, a
    /// 20-second calendar poll).
    pub fn client(&self, timeout: Duration) -> Result<reqwest::Client, EgressDenial> {
        let mut builder = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(Policy::none());
        if !self.target.is_ip_literal() {
            builder = builder.resolve_to_addrs(self.target.host(), &self.addrs);
        }
        // A client that cannot be built is a client that cannot carry the pin.
        builder.build().map_err(|_| EgressDenial::Unresolved)
    }
}

/// Screens `raw` with the product-fetch shape, resolves its host once, and
/// judges every answer.
///
/// Every failure is a typed [`EgressDenial`]; the caller maps it onto the
/// vocabulary its own surface already returns.
pub async fn guard(raw: &str) -> Result<Guarded, EgressDenial> {
    guard_with(raw, egress::screen_fetch).await
}

/// [`guard`] with the screening decision supplied by the caller.
///
/// Two surfaces ask the same address question and disagree only on the consent
/// shape: the product edges accept plain http and an explicit port, while the
/// plugin distribution seam is https-only on the default port. The
/// resolve-once, judge-every-answer, pin-the-result sequence is identical, so
/// the seam passes [`egress::screen`] here instead of growing a second guard —
/// which is how the distribution fetch ended up with a client of its own.
pub async fn guard_with<F>(raw: &str, screen: F) -> Result<Guarded, EgressDenial>
where
    F: FnOnce(&str) -> Result<EgressTarget, EgressDenial>,
{
    let target = screen(raw)?;
    if target.is_ip_literal() {
        return Ok(Guarded {
            target,
            addrs: Vec::new(),
        });
    }
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((target.host(), target.port()))
        .await
        .map_err(|_| EgressDenial::Unresolved)?
        .collect();
    judged(target, addrs)
}

/// Judges one resolution answer and pins it.
///
/// Split from the lookup so the decision is testable without a resolver: the
/// addresses are the input, and one private answer refuses the whole name
/// before a client exists. `check_resolved` also refuses an empty answer, so a
/// name that resolves to nothing is a refusal rather than a retry.
fn judged(target: EgressTarget, addrs: Vec<SocketAddr>) -> Result<Guarded, EgressDenial> {
    let ips: Vec<IpAddr> = addrs.iter().map(SocketAddr::ip).collect();
    egress::check_resolved(&ips)?;
    Ok(Guarded { target, addrs })
}

/// Whether this status is a redirect the caller must refuse rather than follow.
///
/// Re-exported so a call site reads the same predicate the transport enforces,
/// instead of writing `300..400` by hand.
pub fn is_redirect(status: u16) -> bool {
    egress::is_redirect(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The static half, with no DNS and no socket: the four call sites' tests
    /// prove the composed refusal, and these prove the classifier each of them
    /// relies on.
    #[tokio::test]
    async fn a_loopback_and_a_metadata_target_are_refused_before_a_lookup() {
        for url in [
            "http://127.0.0.1:8080/hook",
            "http://[::1]/hook",
            "https://169.254.169.254/latest/meta-data/",
            "https://metadata.google.internal/computeMetadata/v1/",
            "https://2130706433/hook",
            "https://localhost/hook",
        ] {
            assert!(
                guard(url).await.is_err(),
                "{url} must be refused by the screen, not looked up"
            );
        }
    }

    #[tokio::test]
    async fn a_public_literal_is_screened_without_a_lookup() {
        let guarded = guard("https://8.8.8.8/resolve")
            .await
            .expect("a public literal");
        assert_eq!(guarded.host(), "8.8.8.8");
        assert_eq!(guarded.url(), "https://8.8.8.8/resolve");
        // A literal needs no pin, and the client still refuses redirects.
        assert!(guarded.client(Duration::from_millis(100)).is_ok());
    }

    #[tokio::test]
    async fn a_scheme_outside_http_and_https_is_refused_before_a_lookup() {
        for url in [
            "ftp://example.com/hook",
            "file:///etc/passwd",
            "ws://example.com/hook",
        ] {
            assert!(guard(url).await.is_err(), "{url} must be refused");
        }
        assert!(guard("https://user:pw@example.com/hook").await.is_err());
    }

    /// A public plain-http target is a product fetch, not SSRF: the product
    /// screen allows it and the guard only judges the address. A literal needs
    /// no lookup, so this asserts the relaxed shape without touching the
    /// network.
    #[tokio::test]
    async fn a_public_plain_http_literal_is_accepted() {
        let guarded = guard("http://8.8.8.8/hook")
            .await
            .expect("a public http literal is a product fetch");
        assert_eq!(guarded.url(), "http://8.8.8.8/hook");
        assert!(guarded.client(Duration::from_millis(100)).is_ok());
    }

    /// The record's DNS half: a name the screen accepts is still refused when
    /// the resolver answers with an address the process must not reach.
    ///
    /// Driven through [`judged`], the step between the lookup and the client,
    /// so the answer is supplied rather than looked up: no test depends on a
    /// resolver, and none can be rescued by one. A mixed answer is refused
    /// because the private address is reachable through the name, and an empty
    /// answer is a refusal rather than a retry.
    #[test]
    fn a_name_that_resolves_to_a_private_address_is_refused_before_a_client_exists() {
        // Both shapes screen this name: the distribution seam (https, default
        // port) and the product fetch (http, explicit port).
        let seam = egress::screen("https://plugins.example.com/manifest.json")
            .expect("a public name screens for the distribution seam");
        let product = egress::screen_fetch("http://plugins.example.com:8443/hook")
            .expect("a public name screens for a product fetch");

        for answer in [
            "127.0.0.1:443",
            "127.0.0.53:53",
            "10.20.30.40:443",
            "172.16.4.4:443",
            "192.168.1.10:443",
            "169.254.169.254:443",
            "[::1]:443",
            "[fd00:ec2::254]:443",
            "[::ffff:127.0.0.1]:443",
        ] {
            let addr: SocketAddr = answer.parse().expect("a socket address");
            assert!(
                matches!(
                    judged(seam.clone(), vec![addr]),
                    Err(EgressDenial::PrivateAddress)
                ),
                "{answer} must be refused before a client exists"
            );
        }

        // One private answer poisons the whole name: the request would reach it.
        let mixed = vec![
            "93.184.216.34:443".parse().expect("public"),
            "127.0.0.1:443".parse().expect("loopback"),
        ];
        assert!(matches!(
            judged(seam, mixed),
            Err(EgressDenial::PrivateAddress)
        ));
        // ... and the same rule holds for the product-fetch shape.
        assert!(matches!(
            judged(
                product,
                vec!["169.254.169.254:8443".parse().expect("metadata")]
            ),
            Err(EgressDenial::PrivateAddress)
        ));
        // Nothing resolved is a refusal, not a name with no addresses.
        let empty = egress::screen("https://plugins.example.com/manifest.json").expect("screens");
        assert!(matches!(
            judged(empty, Vec::new()),
            Err(EgressDenial::Unresolved)
        ));
    }

    /// A `3xx` comes back as a response and the `Location` is never dialled.
    ///
    /// The listener counts the connections it accepts, so "not followed" is a
    /// measurement. Two loopback listeners on one machine have to be reached by
    /// pinning the client to the first one's address, which is exactly what the
    /// guard does for a resolved name; `judged` would refuse that pin, so the
    /// [`Guarded`] is built here with the same private fields the guard fills.
    /// This drives the transport policy (`Policy::none`), not the address rule.
    #[tokio::test]
    async fn a_redirect_is_answered_and_never_followed_to_a_second_host() {
        let (second_addr, second_connections) = counting_redirector(
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
        );
        let (first_addr, first_connections) = counting_redirector(&format!(
            "HTTP/1.1 302 Found\r\nLocation: http://{second_addr}/elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        ));

        let target = egress::screen_fetch(&format!(
            "http://plugins.example.com:{}/manifest.json",
            first_addr.port()
        ))
        .expect("a public name with an explicit port screens as a product fetch");
        let guarded = Guarded {
            target,
            addrs: vec![first_addr],
        };

        let response = guarded
            .client(Duration::from_secs(5))
            .expect("the client carries the pin")
            .get(guarded.url())
            .send()
            .await
            .expect("the pinned listener answers");
        assert_eq!(response.status().as_u16(), 302);
        assert!(is_redirect(response.status().as_u16()));

        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            first_connections.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the 302 was answered once"
        );
        assert_eq!(
            second_connections.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "the Location was never dialled"
        );
    }

    /// A loopback listener that answers every request with `response` and counts
    /// the connections it accepts. The count is the assertion.
    fn counting_redirector(
        response: &str,
    ) -> (SocketAddr, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::io::{Read, Write};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let count = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&count);
        let response = response.to_string();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                seen.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (addr, count)
    }
}
