//! Link-target checking for synced documents, and the guard that makes it
//! safe to run against customer content.
//!
//! ## Why this is a security surface, not a formatting check
//!
//! A link target in a document comes from the customer's documentation
//! platform. Nobody at Marula typed it and nobody reviewed it. Fetching it is
//! therefore a server-side request made on behalf of the opposite party: a
//! document can point at `http://127.0.0.1:…`, at the cloud instance metadata
//! address, or at a name that resolves to either, and a naive checker turns
//! the connector into an SSRF proxy. The rule below is the whole defence, and
//! it is deliberately the same decision `core-api`'s `outbound` guard makes for
//! the KB link-check (`crates/core-api/src/outbound.rs`,
//! `core-domain::egress::address_is_public`). It is duplicated here rather than
//! imported because a connector may not depend on a `crates/` module (R2); the
//! two lists are kept in step, and the ranges are the same ones.
//!
//! The rule, in order:
//!
//! 1. **Scheme.** Only `http` and `https`. `file:`, `ftp:`, `gopher:` and every
//!    other scheme is refused, and so is a URL carrying a username or password.
//! 2. **Length.** 2048 bytes, the same cap the core guard uses.
//! 3. **Host.** An IP literal is judged directly. A name is judged by its
//!    spelling first: a single label resolves through the host's search
//!    domains, so it is refused; the suffixes `.localhost`, `.local`,
//!    `.internal`, `.home.arpa`, `.in-addr.arpa`, `.ip6.arpa` and `.onion` are
//!    refused; the three metadata hostnames are refused by name.
//! 4. **Resolve once, judge every answer.** A name that returns one public and
//!    one private address is refused, because the private one is still
//!    reachable. An answer set that is empty is a refusal, not a retry.
//! 5. **Pin the connection** to exactly the addresses judged, so a DNS rebind
//!    between the check and the connect cannot swap in a private address.
//! 6. **No redirects.** A `3xx` is recorded as reachable without a hop; the
//!    target in the document is the only host this check may touch.
//! 7. **No body.** The probe sends `HEAD` and reads the status line only. A
//!    response body from customer content is content the connector has no
//!    reason to hold.
//!
//! Ranges refused as addresses: `0.0.0.0/8`, `10.0.0.0/8`, `100.64.0.0/10`
//! (CGNAT), `127.0.0.0/8`, `169.254.0.0/16` (link-local and cloud metadata),
//! `172.16.0.0/12`, `192.0.0.0/24`, `192.0.2.0/24` (RFC 5737 TEST-NET-1),
//! `192.168.0.0/16`, `198.18.0.0/15` (benchmarking), `198.51.100.0/24`
//! (TEST-NET-2), `203.0.113.0/24` (TEST-NET-3), everything from `224.0.0.0/4`
//! up, and on IPv6 the unspecified, loopback, IPv4-mapped, IPv4-compatible,
//! `fc00::/7`, `fe80::/10`, `ff00::/8`, `2001:db8::/32` and `100::/64` forms.
//!
//! A refusal is not a dead link. It is recorded as [`LinkVerdict::Refused`] and
//! counted separately, so "we would not fetch this" never reads as "the
//! customer's document is broken".

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use async_trait::async_trait;
use reqwest::redirect::Policy;
use serde::Serialize;
use url::{Host, Url};

/// The longest target this checker will consider, matching the core guard.
pub const MAX_URL_BYTES: usize = 2048;

/// The outcome of checking one link target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LinkVerdict {
    /// The target answered, or answered with a redirect that was not followed.
    Ok { status: u16 },
    /// The target answered `404`/`410`, or could not be expressed as a URL.
    /// This is the only verdict the report counts as a broken link.
    Dead { reason: String },
    /// The host resolved but the request failed (DNS, TLS, timeout, 5xx). The
    /// document may be fine and the network may not; the operator decides.
    Unreachable { reason: String },
    /// The guard refused the target before a socket existed. Never a dead link.
    Refused { reason: String },
    /// Checking was not attempted (disabled policy, or the target is not a URL
    /// this probe can speak to).
    Unchecked { reason: String },
}

impl LinkVerdict {
    pub const fn code(&self) -> &'static str {
        match self {
            LinkVerdict::Ok { .. } => "ok",
            LinkVerdict::Dead { .. } => "dead",
            LinkVerdict::Unreachable { .. } => "unreachable",
            LinkVerdict::Refused { .. } => "refused",
            LinkVerdict::Unchecked { .. } => "unchecked",
        }
    }

    pub fn is_dead(&self) -> bool {
        matches!(self, LinkVerdict::Dead { .. })
    }

    /// The reason a verdict carries, or `None` for a reachable target.
    pub fn reason(&self) -> Option<&str> {
        match self {
            LinkVerdict::Ok { .. } => None,
            LinkVerdict::Dead { reason }
            | LinkVerdict::Unreachable { reason }
            | LinkVerdict::Refused { reason }
            | LinkVerdict::Unchecked { reason } => Some(reason),
        }
    }
}

/// One link target the sync checked, as the run report carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BrokenLink {
    pub external_id: String,
    pub field: String,
    /// The target exactly as the document carried it — retained so an operator
    /// can open it and decide, rather than being told "a link is broken".
    pub target: String,
    pub verdict: &'static str,
    pub reason: String,
}

/// The probe seam. A none-mode or dry run uses [`NoLinkProbe`]; the shipped
/// runtime uses [`HttpLinkProbe`]; tests use their own.
#[async_trait]
pub trait LinkProbe: Send + Sync {
    async fn check(&self, target: &str) -> LinkVerdict;
}

/// None mode: attempts nothing and says so.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoLinkProbe;

#[async_trait]
impl LinkProbe for NoLinkProbe {
    async fn check(&self, _target: &str) -> LinkVerdict {
        LinkVerdict::Unchecked {
            reason: "link checking disabled (none mode)".into(),
        }
    }
}

/// The HTTP probe: guarded, pinned, bounded, status-only.
#[derive(Debug, Clone)]
pub struct HttpLinkProbe {
    timeout: Duration,
}

impl Default for HttpLinkProbe {
    fn default() -> Self {
        Self {
            // Bounded so one unreachable host cannot stall a sync run.
            timeout: Duration::from_secs(5),
        }
    }
}

impl HttpLinkProbe {
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }
}

#[async_trait]
impl LinkProbe for HttpLinkProbe {
    async fn check(&self, target: &str) -> LinkVerdict {
        let ticker = match screen(target) {
            Ok(url) => url,
            Err(reason) => return LinkVerdict::Refused { reason },
        };

        // A literal needs no lookup; a name is resolved once and every answer
        // is judged. The lookup result is what the connection is pinned to.
        let pinned = match ticker.host() {
            Some(Host::Ipv4(ip)) => {
                if !address_is_public(IpAddr::V4(ip)) {
                    return LinkVerdict::Refused {
                        reason: "address is not publicly routable".into(),
                    };
                }
                None
            }
            Some(Host::Ipv6(ip)) => {
                if !address_is_public(IpAddr::V6(ip)) {
                    return LinkVerdict::Refused {
                        reason: "address is not publicly routable".into(),
                    };
                }
                None
            }
            Some(Host::Domain(name)) => {
                let port = ticker.port_or_known_default().unwrap_or(80);
                let addrs: Vec<SocketAddr> = match tokio::net::lookup_host((name, port)).await {
                    Ok(found) => found.collect(),
                    Err(error) => {
                        return LinkVerdict::Unreachable {
                            reason: format!("dns: {error}"),
                        }
                    }
                };
                if addrs.is_empty() {
                    return LinkVerdict::Refused {
                        reason: "name resolved to no address".into(),
                    };
                }
                if let Some(bad) = addrs.iter().find(|a| !address_is_public(a.ip())) {
                    return LinkVerdict::Refused {
                        reason: format!("name resolves to a non-public address ({})", bad.ip()),
                    };
                }
                Some((name.to_string(), addrs))
            }
            None => {
                return LinkVerdict::Refused {
                    reason: "url names no host".into(),
                }
            }
        };

        let mut builder = reqwest::Client::builder()
            .timeout(self.timeout)
            .redirect(Policy::none())
            // No proxy: a proxy would make the request from an address this
            // guard never judged.
            .no_proxy();
        if let Some((host, addrs)) = &pinned {
            builder = builder.resolve_to_addrs(host, addrs);
        }
        let client = match builder.build() {
            Ok(client) => client,
            Err(error) => {
                return LinkVerdict::Unreachable {
                    reason: format!("client: {error}"),
                }
            }
        };

        let response = match client.head(ticker.as_str()).send().await {
            Ok(response) => response,
            Err(error) => {
                return LinkVerdict::Unreachable {
                    reason: request_reason(&error),
                }
            }
        };
        let status = response.status();
        // The body is never read: dropping the response closes the stream.
        drop(response);

        match status.as_u16() {
            404 | 410 => LinkVerdict::Dead {
                reason: format!("http_{}", status.as_u16()),
            },
            // A redirect is reachable. Following it would be a hop to a host
            // nothing screened.
            200..=399 => LinkVerdict::Ok {
                status: status.as_u16(),
            },
            // Some servers refuse HEAD on a page that exists.
            405 | 501 => LinkVerdict::Unchecked {
                reason: format!("http_{} (HEAD not supported)", status.as_u16()),
            },
            code => LinkVerdict::Unreachable {
                reason: format!("http_{code}"),
            },
        }
    }
}

fn request_reason(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "timeout".into()
    } else if error.is_connect() {
        "connect".into()
    } else {
        "transport".into()
    }
}

/// Validates and normalizes one target. Returns the parsed URL whose own
/// serialization is what the request uses, so a parser differential cannot
/// authorize one string and fetch another.
pub fn screen(raw: &str) -> Result<Url, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("empty target".into());
    }
    if raw.len() > MAX_URL_BYTES {
        return Err("target exceeds 2048 bytes".into());
    }
    let url = Url::parse(raw).map_err(|_| "malformed url".to_string())?;
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(format!("scheme {other} is not http or https")),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("url carries credentials".into());
    }
    match url.host() {
        None => Err("url names no host".into()),
        Some(Host::Domain(name)) => {
            if domain_is_denied(name) {
                Err(format!("host {name} is not publicly routable"))
            } else {
                Ok(url)
            }
        }
        Some(Host::Ipv4(ip)) => {
            if address_is_public(IpAddr::V4(ip)) {
                Ok(url)
            } else {
                Err("address is not publicly routable".into())
            }
        }
        Some(Host::Ipv6(ip)) => {
            if address_is_public(IpAddr::V6(ip)) {
                Ok(url)
            } else {
                Err("address is not publicly routable".into())
            }
        }
    }
}

fn domain_is_denied(name: &str) -> bool {
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    if name.is_empty() {
        return true;
    }
    // A single label has no public meaning: it resolves through the host's
    // search domains, which is how `metadata` becomes a link-local address.
    if !name.contains('.') {
        return true;
    }
    const DENIED_SUFFIXES: [&str; 7] = [
        ".localhost",
        ".local",
        ".internal",
        ".home.arpa",
        ".in-addr.arpa",
        ".ip6.arpa",
        ".onion",
    ];
    if DENIED_SUFFIXES.iter().any(|suffix| name.ends_with(suffix)) {
        return true;
    }
    matches!(
        name.as_str(),
        "metadata.google.internal" | "metadata.azure.internal" | "instance-data.ec2.internal"
    )
}

/// Whether an address is publicly routable. Mirrors
/// `core_domain::egress::address_is_public`; see the module comment for why the
/// list is repeated rather than imported.
pub fn address_is_public(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => ipv4_is_public(v4),
        IpAddr::V6(v6) => ipv6_is_public(v6),
    }
}

fn ipv4_is_public(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !(o[0] == 0                       // 0.0.0.0/8 "this network"
        || o[0] == 10                 // 10.0.0.0/8
        || (o[0] == 100 && (64..=127).contains(&o[1])) // 100.64.0.0/10 CGNAT
        || o[0] == 127                // 127.0.0.0/8 loopback
        || (o[0] == 169 && o[1] == 254) // 169.254.0.0/16 link-local, cloud metadata
        || (o[0] == 172 && (16..=31).contains(&o[1])) // 172.16.0.0/12
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0.0/24 IETF assignments
        || (o[0] == 192 && o[1] == 0 && o[2] == 2) // 192.0.2.0/24 TEST-NET-1
        || (o[0] == 192 && o[1] == 168) // 192.168.0.0/16
        || (o[0] == 198 && (o[1] == 18 || o[1] == 19)) // 198.18.0.0/15 benchmarking
        || (o[0] == 198 && o[1] == 51 && o[2] == 100) // 198.51.100.0/24 TEST-NET-2
        || (o[0] == 203 && o[1] == 0 && o[2] == 113) // 203.0.113.0/24 TEST-NET-3
        || o[0] >= 224) // multicast, reserved, broadcast
}

fn ipv6_is_public(ip: Ipv6Addr) -> bool {
    if ip.is_unspecified() || ip.is_loopback() {
        return false;
    }
    if let Some(v4) = ip.to_ipv4_mapped() {
        return ipv4_is_public(v4);
    }
    let s = ip.segments();
    if s[..6] == [0, 0, 0, 0, 0, 0] {
        let v4 = Ipv4Addr::new(
            (s[6] >> 8) as u8,
            (s[6] & 0xff) as u8,
            (s[7] >> 8) as u8,
            (s[7] & 0xff) as u8,
        );
        return ipv4_is_public(v4);
    }
    !((s[0] & 0xfe00) == 0xfc00      // fc00::/7 unique local
        || (s[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
        || (s[0] & 0xff00) == 0xff00 // ff00::/8 multicast
        || (s[0] == 0x2001 && s[1] == 0x0db8) // 2001:db8::/32 documentation
        || (s[0] == 0x0100 && s[1] == 0 && s[2] == 0 && s[3] == 0)) // 100::/64 discard
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_link_local_and_documentation_literals_are_refused() {
        for target in [
            "http://127.0.0.1:8080/admin",
            "http://127.1.2.3/",
            "http://[::1]/",
            "http://10.0.0.5/",
            "http://172.20.0.1/",
            "http://192.168.1.1/",
            "http://169.254.169.254/latest/meta-data/",
            // RFC 5737 documentation ranges, the brief's explicit case.
            "http://192.0.2.10/",
            "http://198.51.100.7/",
            "http://203.0.113.9/",
            "http://100.64.0.1/",
            "http://[fc00::1]/",
            "http://[fe80::1]/",
            "http://0.0.0.0/",
        ] {
            let verdict = screen(target);
            assert!(verdict.is_err(), "{target} must be refused");
        }
    }

    #[test]
    fn names_that_resolve_through_search_domains_or_local_suffixes_are_refused() {
        for target in [
            "http://localhost/",
            "http://metadata/",
            "http://printer.local/",
            "http://vault.internal/",
            "https://metadata.google.internal/computeMetadata/v1/",
            "http://x.onion/",
        ] {
            assert!(screen(target).is_err(), "{target} must be refused");
        }
        // Credentials in the URL and a non-HTTP scheme are refused too.
        assert!(screen("http://user:pass@example.com/").is_err());
        assert!(screen("file:///etc/passwd").is_err());
        assert!(screen("ftp://example.com/file").is_err());
        // A public target parses and would be fetched.
        assert!(screen("https://example.com/docs").is_ok());
    }

    #[tokio::test]
    async fn a_private_literal_is_refused_before_a_socket() {
        let probe = HttpLinkProbe::new(Duration::from_millis(200));
        let verdict = probe.check("http://127.0.0.1:9/never").await;
        assert!(
            matches!(verdict, LinkVerdict::Refused { .. }),
            "{verdict:?}"
        );
        let verdict = probe.check("http://203.0.113.9/never").await;
        assert!(matches!(verdict, LinkVerdict::Refused { .. }));
    }
}
