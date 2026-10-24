//! Guarded outbound fetch for the product edges (Wave 54 lane S1a).
//!
//! ## One control, four call sites
//!
//! The decision lives in `core_domain::egress`; this module is the only place a
//! non-plugin caller turns that decision into a request. Before Wave 54 the KB
//! link-check, the KB oEmbed fetch, the calendar iCal poll and the outbound
//! webhook each built a client and connected to whatever URL their row held, so
//! a stored `http://127.0.0.1:…` or `http://169.254.169.254/…` was a request the
//! process made on the row's behalf. `plugin_http.rs` already screened with the
//! same guard; these four edges now do too.
//!
//! ## Order, and what each step stops
//!
//! 1. **`screen`** refuses a URL that is not https, carries userinfo, names a
//!    non-default port, or has a host that is not publicly routable. A private
//!    or metadata address spelled as a literal, as `2130706433`, or as a name
//!    on the denied-suffix list never reaches a socket.
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
/// The address set is empty for an IP-literal target: [`egress::screen`] already
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

/// Screens `raw`, resolves its host once, and judges every answer.
///
/// Every failure is a typed [`EgressDenial`]; the caller maps it onto the
/// vocabulary its own surface already returns.
pub async fn guard(raw: &str) -> Result<Guarded, EgressDenial> {
    let target = egress::screen(raw)?;
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
    async fn a_url_outside_the_https_default_port_is_refused() {
        assert!(guard("https://example.com:8443/hook").await.is_err());
        assert!(guard("http://example.com/hook").await.is_err());
        assert!(guard("https://user:pw@example.com/hook").await.is_err());
    }
}
