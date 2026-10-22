//! The outbound-URL guard: one pure decision over a URL and an allowlist.
//!
//! Wave 53 lane 7 (`p53-connector-egress`) extracted this from two guards that
//! had drifted apart. `core-api`'s distribution seam carried a hand-rolled host
//! prefix screen, and `plugin-host`'s ABI carried the allowlist half for
//! `http_request`. They disagreed about credentials in the URL, about ports,
//! about every private range that is not a string prefix, and about which
//! scheme is acceptable. Both callers now ask this module, so a plugin request,
//! a distribution fetch and Wave 54's connector egress cannot drift again.
//!
//! Nothing here opens a socket, resolves a name or reads a clock. The two
//! questions a transport has that need I/O are pure functions over values the
//! transport supplies:
//!
//! * [`check_resolved`] judges the addresses a name resolved to, so a caller
//!   resolves once, checks every answer and connects to exactly those
//!   addresses;
//! * [`is_redirect`] and [`MAX_REDIRECTS`] state the redirect rule. Zero
//!   redirects: a redirect target is an unconsented host.
//!
//! The contract, in order:
//!
//! 1. [`screen`] parses the URL once with the WHATWG parser, refuses anything
//!    that is not `https`, refuses userinfo and a non-default port, and refuses
//!    a host that is private, loopback, link-local, metadata or otherwise
//!    unroutable — including the literal spellings the parser normalizes.
//! 2. [`authorize`] runs [`screen`] and then matches the result against a
//!    consented allowlist: exact host, exact port, path inside the scope's
//!    prefix on a segment boundary.
//! 3. The transport resolves the authorized host once, passes every answer to
//!    [`check_resolved`], pins the addresses it checked, and refuses any
//!    redirect.
//!
//! The verdict is [`EgressDenial`], never prose. A caller maps it onto its own
//! vocabulary: `plugin-host` onto `plugin_egress_*`, `core-api` onto a
//! validation error. This module names no plugin, no request type and no
//! database row, which is what lets Wave 54's connector egress consume it.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use url::{Host, Url};

/// Longest URL the guard accepts, in bytes. Matches the connector-endpoint cap
/// and the ABI's `http_request` URL cap.
pub const MAX_URL_BYTES: usize = 2048;

/// The only port an `https` target may name. `url` normalizes an explicit
/// `:443` away, so any port the parser still reports is a non-default one.
pub const DEFAULT_HTTPS_PORT: u16 = 443;

/// Redirects a transport using this guard may follow.
///
/// Zero, by ADR 0023 Decision 4: a redirect points at a host the install never
/// consented to, so following one would launder an allowlist decision through
/// the remote server.
pub const MAX_REDIRECTS: u32 = 0;

/// Why a URL, an allowlist entry or a resolved address set was refused.
///
/// Every variant is a distinct fact about the input, so an audit row can say
/// which rule fired without parsing a message. The set is `non_exhaustive`:
/// a later wave can add a reason without breaking a caller's match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EgressDenial {
    /// Not an absolute URL, or no host.
    MalformedUrl,
    /// Above [`MAX_URL_BYTES`].
    UrlTooLong,
    /// The scheme is not `https`.
    SchemeDenied,
    /// The URL carries a username or password.
    UserInfoDenied,
    /// The URL names a port other than [`DEFAULT_HTTPS_PORT`].
    PortDenied,
    /// No allowlist entry names this host and port.
    HostDenied,
    /// The host is allowlisted but the path is outside every scope's prefix.
    PathDenied,
    /// The host is a literal, or a name, that is not publicly routable.
    PrivateAddress,
    /// A redirect response. Zero are followed.
    RedirectDenied,
    /// A name resolved to no address at all.
    Unresolved,
    /// An allowlist entry is not a legal scope.
    ScopeMalformed,
    /// The allowlist is empty, so nothing could match.
    EmptyAllowlist,
}

impl EgressDenial {
    /// The stable string a caller may put in an audit row or an envelope.
    /// `core-domain` says `egress_*`; the plugin host prefixes the plugin
    /// vocabulary onto the same fact.
    pub const fn as_str(self) -> &'static str {
        match self {
            EgressDenial::MalformedUrl => "egress_malformed_url",
            EgressDenial::UrlTooLong => "egress_url_too_long",
            EgressDenial::SchemeDenied => "egress_scheme_denied",
            EgressDenial::UserInfoDenied => "egress_userinfo_denied",
            EgressDenial::PortDenied => "egress_port_denied",
            EgressDenial::HostDenied => "egress_host_denied",
            EgressDenial::PathDenied => "egress_path_denied",
            EgressDenial::PrivateAddress => "egress_private_address",
            EgressDenial::RedirectDenied => "egress_redirect_denied",
            EgressDenial::Unresolved => "egress_unresolved",
            EgressDenial::ScopeMalformed => "egress_scope_malformed",
            EgressDenial::EmptyAllowlist => "egress_empty_allowlist",
        }
    }
}

impl fmt::Display for EgressDenial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One consented scope: `https://host[:443][/path-prefix]`.
///
/// Parsed once and reusable, so a caller that caches a profile's scopes pays
/// for the parse once rather than per request. The fields are private because
/// a scope built by hand could carry a wildcard or a private host; the only way
/// in is [`EgressScope::parse`], which refuses both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressScope {
    host: String,
    port: u16,
    path_prefix: String,
}

impl EgressScope {
    /// Parses one allowlist entry. Refuses anything that could not be
    /// consented: a non-https scheme, credentials, a query or fragment, a
    /// non-default port, a wildcard, a private host, or a malformed URL.
    pub fn parse(raw: &str) -> Result<Self, EgressDenial> {
        if raw.is_empty() || raw.len() > MAX_URL_BYTES {
            return Err(EgressDenial::ScopeMalformed);
        }
        let parsed = Url::parse(raw).map_err(|_| EgressDenial::ScopeMalformed)?;
        if parsed.scheme() != "https" {
            return Err(EgressDenial::SchemeDenied);
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(EgressDenial::UserInfoDenied);
        }
        if parsed.query().is_some() || parsed.fragment().is_some() {
            return Err(EgressDenial::ScopeMalformed);
        }
        if parsed.port().is_some() {
            return Err(EgressDenial::PortDenied);
        }
        let host = parsed.host().ok_or(EgressDenial::ScopeMalformed)?;
        if let Host::Domain(name) = &host {
            if name.contains('*') {
                return Err(EgressDenial::ScopeMalformed);
            }
        }
        if host_is_denied(&host) {
            return Err(EgressDenial::PrivateAddress);
        }
        let host = canonical_host(&parsed).ok_or(EgressDenial::ScopeMalformed)?;
        let mut path_prefix = parsed.path().to_string();
        while path_prefix.len() > 1 && path_prefix.ends_with('/') {
            path_prefix.pop();
        }
        Ok(EgressScope {
            host,
            port: DEFAULT_HTTPS_PORT,
            path_prefix,
        })
    }

    /// The scope's host, lowercase and without a trailing dot.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The scope's port. Always [`DEFAULT_HTTPS_PORT`].
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The path prefix, without a trailing slash except for the root scope.
    pub fn path_prefix(&self) -> &str {
        &self.path_prefix
    }

    /// Whether this scope admits `target`. Host and port must match exactly;
    /// the path must be the prefix itself or continue past a `/` boundary, so
    /// a `/v1` scope never admits `/v10`.
    pub fn matches(&self, target: &EgressTarget) -> bool {
        self.host == target.host && self.port == target.port && self.path_allows(&target.path)
    }

    fn path_allows(&self, path: &str) -> bool {
        if self.path_prefix == "/" {
            return true;
        }
        if path == self.path_prefix {
            return true;
        }
        path.strip_prefix(&self.path_prefix)
            .is_some_and(|rest| rest.starts_with('/'))
    }
}

/// A URL that passed [`screen`], in the exact form the transport must fetch.
///
/// The transport uses [`EgressTarget::as_str`] rather than the caller's string.
/// That is deliberate: the guard and the HTTP client then read one parse of the
/// input, so a parser differential cannot authorize one URL and fetch another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressTarget {
    url: String,
    host: String,
    port: u16,
    path: String,
}

impl EgressTarget {
    /// The canonical URL to fetch: `https`, no fragment, no trailing dot in the
    /// host, no userinfo, no explicit default port.
    pub fn as_str(&self) -> &str {
        &self.url
    }

    /// The host, lowercase and bracket-free (an IPv6 literal is bare here and
    /// bracketed in [`EgressTarget::as_str`]).
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The port. Always [`DEFAULT_HTTPS_PORT`].
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The path, without a query or fragment.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Whether the host is an IP literal, which needs no name resolution.
    pub fn is_ip_literal(&self) -> bool {
        self.host.parse::<IpAddr>().is_ok()
    }
}

/// Screens one URL with no allowlist: the distribution seam's question, and the
/// static half of every egress decision.
///
/// Refuses, in order: an over-long or unparseable URL, a scheme that is not
/// `https`, userinfo, a non-default port, and a host that is not publicly
/// routable.
pub fn screen(raw: &str) -> Result<EgressTarget, EgressDenial> {
    if raw.is_empty() {
        return Err(EgressDenial::MalformedUrl);
    }
    if raw.len() > MAX_URL_BYTES {
        return Err(EgressDenial::UrlTooLong);
    }
    let mut parsed = Url::parse(raw).map_err(|_| EgressDenial::MalformedUrl)?;
    if parsed.scheme() != "https" {
        return Err(EgressDenial::SchemeDenied);
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(EgressDenial::UserInfoDenied);
    }
    if parsed.port().is_some() {
        return Err(EgressDenial::PortDenied);
    }
    let host = parsed.host().ok_or(EgressDenial::MalformedUrl)?;
    if host_is_denied(&host) {
        return Err(EgressDenial::PrivateAddress);
    }
    // `api.example.com.` and `api.example.com` are one host. Strip the dot so
    // the allowlist match, the denylist and the connection all see one
    // spelling; a trailing dot is a classic way past a suffix check.
    let trailing_dot = match parsed.host() {
        Some(Host::Domain(name)) => name.strip_suffix('.').map(str::to_string),
        _ => None,
    };
    if let Some(trimmed) = trailing_dot {
        parsed
            .set_host(Some(&trimmed))
            .map_err(|_| EgressDenial::MalformedUrl)?;
    }
    // A fragment never reaches a server, so it is dropped rather than refused.
    parsed.set_fragment(None);
    let host = canonical_host(&parsed).ok_or(EgressDenial::MalformedUrl)?;
    let port = parsed
        .port_or_known_default()
        .ok_or(EgressDenial::MalformedUrl)?;
    let path = parsed.path().to_string();
    Ok(EgressTarget {
        url: parsed.to_string(),
        host,
        port,
        path,
    })
}

/// Screens `raw` and matches it against a consented allowlist.
///
/// The allowlist is a list of [`EgressScope`]s, not raw strings: a caller that
/// reuses a parsed profile pays nothing per request. An empty list refuses with
/// [`EgressDenial::EmptyAllowlist`] rather than falling through, because an
/// empty allowlist and an absent one are the same answer to a request.
pub fn authorize(raw: &str, allowlist: &[EgressScope]) -> Result<EgressTarget, EgressDenial> {
    let target = screen(raw)?;
    if allowlist.is_empty() {
        return Err(EgressDenial::EmptyAllowlist);
    }
    let mut host_matched = false;
    for scope in allowlist {
        if scope.host != target.host || scope.port != target.port {
            continue;
        }
        host_matched = true;
        if scope.path_allows(&target.path) {
            return Ok(target);
        }
    }
    Err(if host_matched {
        EgressDenial::PathDenied
    } else {
        EgressDenial::HostDenied
    })
}

/// [`authorize`] over the raw scope strings a manifest carries
/// (`connector_profile.outbound_scopes`). Convenience for a caller that has not
/// cached parsed scopes; it parses each entry and fails closed if any is
/// malformed, because a typo in a consent declaration must not quietly shrink
/// the allowlist to the entries that happened to parse.
pub fn authorize_strings(raw: &str, allowlist: &[String]) -> Result<EgressTarget, EgressDenial> {
    let scopes = allowlist
        .iter()
        .map(|scope| EgressScope::parse(scope))
        .collect::<Result<Vec<_>, _>>()?;
    authorize(raw, &scopes)
}

/// Judges the addresses a hostname resolved to, before the transport connects.
///
/// The caller resolves once and pins exactly these addresses; there is no
/// second lookup for an attacker to win. An empty answer is
/// [`EgressDenial::Unresolved`], and one bad address refuses the whole name —
/// a name that resolves to a public and a private address is a name that can
/// reach the private one.
pub fn check_resolved(addrs: &[IpAddr]) -> Result<(), EgressDenial> {
    if addrs.is_empty() {
        return Err(EgressDenial::Unresolved);
    }
    if addrs.iter().any(|addr| !address_is_public(*addr)) {
        return Err(EgressDenial::PrivateAddress);
    }
    Ok(())
}

/// Whether a redirect status must be refused. True for every `3xx`, because
/// [`MAX_REDIRECTS`] is zero.
pub fn is_redirect(status: u16) -> bool {
    (300..400).contains(&status)
}

/// Whether an address is publicly routable. The inverse of every special-use
/// range this build knows: private, loopback, link-local (where cloud metadata
/// lives), CGNAT, benchmarking, documentation, multicast, reserved, and the
/// IPv6 equivalents including unique-local.
///
/// `Ipv4Addr::is_global` is still unstable, so the ranges are spelled out here
/// and tested one by one.
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
    // An IPv4-mapped address is the IPv4 it carries, so `::ffff:127.0.0.1` is
    // loopback rather than a routable IPv6 address.
    if let Some(v4) = ip.to_ipv4_mapped() {
        return ipv4_is_public(v4);
    }
    let s = ip.segments();
    // The deprecated IPv4-compatible form `::a.b.c.d` embeds the same address.
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

/// The one spelling of a host the allowlist match and the pinned connection
/// both use: a domain is lowercase with no trailing dot, and an IP literal is
/// bracket-free so [`EgressTarget::is_ip_literal`] can recognize it. The URL
/// itself keeps whatever brackets the serialization needs.
fn canonical_host(url: &Url) -> Option<String> {
    match url.host()? {
        Host::Domain(name) => Some(name.trim_end_matches('.').to_ascii_lowercase()),
        Host::Ipv4(ip) => Some(ip.to_string()),
        Host::Ipv6(ip) => Some(ip.to_string()),
    }
}

fn host_is_denied(host: &Host<&str>) -> bool {
    match host {
        Host::Ipv4(ip) => !ipv4_is_public(*ip),
        Host::Ipv6(ip) => !ipv6_is_public(*ip),
        Host::Domain(name) => domain_is_denied(name),
    }
}

fn domain_is_denied(name: &str) -> bool {
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    if name.is_empty() {
        return true;
    }
    // A single label has no public meaning: it resolves through the host's
    // search domains, which is how `metadata` becomes a link-local address on a
    // cloud instance.
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
    // `.internal` already covers the first two; they are named because they are
    // the endpoints that matter, not because the suffix check misses them.
    matches!(
        name.as_str(),
        "metadata.google.internal" | "metadata.azure.internal" | "instance-data.ec2.internal"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scopes(raw: &[&str]) -> Vec<EgressScope> {
        raw.iter()
            .map(|scope| EgressScope::parse(scope).expect("test scope parses"))
            .collect()
    }

    fn api() -> Vec<EgressScope> {
        scopes(&["https://api.example.com/v1", "https://billing.example.com"])
    }

    #[test]
    fn a_private_address_is_refused_in_every_spelling() {
        for url in [
            "https://10.0.0.1/v1",
            "https://172.16.4.4/v1",
            "https://192.168.1.1/v1",
            "https://127.0.0.1/v1",
            "https://0.0.0.0/v1",
            "https://100.64.0.1/v1",
            "https://[::1]/v1",
            "https://[fc00::1]/v1",
            "https://[fe80::1]/v1",
            "https://[::ffff:127.0.0.1]/v1",
            // The WHATWG host parser normalizes this to 127.0.0.1; a string
            // prefix check would miss it.
            "https://0x7f000001/v1",
            "https://2130706433/v1",
        ] {
            assert_eq!(
                screen(url),
                Err(EgressDenial::PrivateAddress),
                "{url} must not be routable"
            );
        }
    }

    #[test]
    fn a_link_local_and_metadata_endpoint_is_refused_by_address_and_by_name() {
        for url in [
            "https://169.254.169.254/latest/meta-data/",
            "https://[fd00:ec2::254]/latest/meta-data/",
            "https://metadata.google.internal/computeMetadata/v1/",
            "https://metadata.azure.internal/metadata/instance",
            "https://instance-data.ec2.internal/",
            "https://metadata/",
            "https://100.100.100.200/latest/meta-data/",
        ] {
            assert_eq!(
                screen(url),
                Err(EgressDenial::PrivateAddress),
                "{url} must not be reachable"
            );
        }
    }

    #[test]
    fn a_scheme_that_is_not_https_is_refused() {
        for url in [
            "http://api.example.com/v1",
            "ftp://api.example.com/v1",
            "file:///etc/passwd",
            "ws://api.example.com/v1",
        ] {
            assert_eq!(screen(url), Err(EgressDenial::SchemeDenied), "{url}");
        }
    }

    #[test]
    fn a_host_outside_the_allowlist_is_refused() {
        assert_eq!(
            authorize("https://evil.example.net/v1", &api()),
            Err(EgressDenial::HostDenied)
        );
        // A suffix of an allowlisted host is a different host.
        assert_eq!(
            authorize("https://api.example.com.evil.net/v1", &api()),
            Err(EgressDenial::HostDenied)
        );
        // So is a subdomain of one.
        assert_eq!(
            authorize("https://sub.api.example.com/v1", &api()),
            Err(EgressDenial::HostDenied)
        );
    }

    #[test]
    fn a_path_outside_the_scope_prefix_is_refused() {
        assert_eq!(
            authorize("https://api.example.com/v2/tickets", &api()),
            Err(EgressDenial::PathDenied)
        );
        // A string prefix would admit /v10; a segment boundary does not.
        assert_eq!(
            authorize("https://api.example.com/v10", &api()),
            Err(EgressDenial::PathDenied)
        );
        // Dot segments are normalized by the parser before the match.
        assert_eq!(
            authorize("https://api.example.com/v1/../admin", &api()),
            Err(EgressDenial::PathDenied)
        );
        // Percent-encoding does not smuggle a path past the prefix.
        assert_eq!(
            authorize("https://api.example.com/%76%31/tickets", &api()),
            Err(EgressDenial::PathDenied)
        );
        assert_eq!(
            authorize("https://api.example.com/v1/tickets", &api())
                .expect("prefix admits a child path")
                .path(),
            "/v1/tickets"
        );
        // The root scope admits every path.
        assert!(authorize("https://billing.example.com/any/where", &api()).is_ok());
    }

    #[test]
    fn a_userinfo_component_is_refused() {
        assert_eq!(
            screen("https://user:pw@api.example.com/v1"),
            Err(EgressDenial::UserInfoDenied)
        );
        assert_eq!(
            screen("https://user@api.example.com/v1"),
            Err(EgressDenial::UserInfoDenied)
        );
        // The authority after `@` is the host the parser actually connects to,
        // so a credentials-looking prefix cannot name the allowlisted host. The
        // userinfo refusal fires first, which is the stronger answer.
        assert_eq!(
            authorize("https://api.example.com@evil.example.net/v1", &api()),
            Err(EgressDenial::UserInfoDenied)
        );
    }

    #[test]
    fn a_port_that_is_not_the_default_is_refused() {
        assert_eq!(
            screen("https://api.example.com:8443/v1"),
            Err(EgressDenial::PortDenied)
        );
        assert_eq!(
            screen("https://api.example.com:80/v1"),
            Err(EgressDenial::PortDenied)
        );
        // An explicit :443 is the default and the parser normalizes it away.
        let target = screen("https://api.example.com:443/v1").expect("default port");
        assert_eq!(target.port(), DEFAULT_HTTPS_PORT);
        assert_eq!(target.as_str(), "https://api.example.com/v1");
        // A scope may not declare a port that no request can use.
        assert_eq!(
            EgressScope::parse("https://api.example.com:8443/v1"),
            Err(EgressDenial::PortDenied)
        );
    }

    #[test]
    fn a_redirect_is_never_followed_and_its_target_is_judged_the_same_way() {
        assert_eq!(MAX_REDIRECTS, 0);
        for status in [301, 302, 303, 307, 308] {
            assert!(is_redirect(status), "{status} is a redirect");
        }
        assert!(!is_redirect(200));
        assert!(!is_redirect(204));
        // If a caller did follow one, the target would still have to pass the
        // allowlist — which is why following is pointless as well as unsafe.
        assert_eq!(
            authorize("https://evil.example.net/steal", &api()),
            Err(EgressDenial::HostDenied)
        );
    }

    #[test]
    fn a_dns_name_that_resolves_into_a_private_range_is_refused() {
        let public: IpAddr = "93.184.216.34".parse().unwrap();
        let loopback: IpAddr = "127.0.0.1".parse().unwrap();
        let link_local: IpAddr = "169.254.169.254".parse().unwrap();
        let ula: IpAddr = "fd00::1".parse().unwrap();

        assert_eq!(check_resolved(&[public]), Ok(()));
        assert_eq!(
            check_resolved(&[loopback]),
            Err(EgressDenial::PrivateAddress)
        );
        assert_eq!(
            check_resolved(&[public, link_local]),
            Err(EgressDenial::PrivateAddress),
            "one bad answer refuses the whole name"
        );
        assert_eq!(check_resolved(&[ula]), Err(EgressDenial::PrivateAddress));
        assert_eq!(check_resolved(&[]), Err(EgressDenial::Unresolved));
    }

    #[test]
    fn a_scope_must_be_a_consentable_url() {
        assert_eq!(
            EgressScope::parse("https://*.example.com"),
            Err(EgressDenial::ScopeMalformed),
            "a wildcard is not consent"
        );
        assert_eq!(
            EgressScope::parse("https://api.example.com/v1?x=1"),
            Err(EgressDenial::ScopeMalformed)
        );
        assert_eq!(
            EgressScope::parse("https://api.example.com/v1#frag"),
            Err(EgressDenial::ScopeMalformed)
        );
        assert_eq!(
            EgressScope::parse("http://api.example.com/v1"),
            Err(EgressDenial::SchemeDenied)
        );
        assert_eq!(
            EgressScope::parse("https://user:pw@api.example.com/v1"),
            Err(EgressDenial::UserInfoDenied)
        );
        assert_eq!(
            EgressScope::parse("https://127.0.0.1/v1"),
            Err(EgressDenial::PrivateAddress)
        );
        assert_eq!(
            EgressScope::parse("not-a-url"),
            Err(EgressDenial::ScopeMalformed)
        );
    }

    #[test]
    fn an_empty_allowlist_refuses_rather_than_matching_everything() {
        assert_eq!(
            authorize("https://api.example.com/v1", &[]),
            Err(EgressDenial::EmptyAllowlist)
        );
    }

    #[test]
    fn a_malformed_or_overlong_url_is_a_typed_refusal() {
        assert_eq!(screen(""), Err(EgressDenial::MalformedUrl));
        assert_eq!(screen("not a url"), Err(EgressDenial::MalformedUrl));
        assert_eq!(screen("https://"), Err(EgressDenial::MalformedUrl));
        let long = format!("https://api.example.com/{}", "a".repeat(MAX_URL_BYTES));
        assert_eq!(screen(&long), Err(EgressDenial::UrlTooLong));
    }

    #[test]
    fn the_target_is_the_exact_string_the_transport_fetches() {
        let target = authorize("https://API.Example.com/v1/tickets?state=open#top", &api())
            .expect("host case, query and fragment are all fine");
        assert_eq!(
            target.as_str(),
            "https://api.example.com/v1/tickets?state=open"
        );
        assert_eq!(target.host(), "api.example.com");
        assert_eq!(target.path(), "/v1/tickets");
        assert_eq!(target.port(), 443);
        assert!(!target.is_ip_literal());
    }

    #[test]
    fn an_ipv6_literal_keeps_its_brackets_in_the_url_and_not_in_the_host() {
        let target = authorize(
            "https://[2606:4700:4700::1111]/dns-query",
            &scopes(&["https://[2606:4700:4700::1111]/dns-query"]),
        )
        .expect("a public IPv6 literal may be allowlisted");
        assert_eq!(target.host(), "2606:4700:4700::1111");
        assert!(target.is_ip_literal());
        assert_eq!(target.as_str(), "https://[2606:4700:4700::1111]/dns-query");
    }

    #[test]
    fn a_trailing_dot_host_is_one_host_not_two() {
        // The allowlist says `api.example.com`; `api.example.com.` is the same
        // DNS name and must match it rather than slip past as "unknown".
        let target = authorize("https://api.example.com./v1/tickets", &api())
            .expect("the trailing dot names the same host");
        assert_eq!(target.host(), "api.example.com");
        assert_eq!(target.as_str(), "https://api.example.com/v1/tickets");
        // And it cannot be used to smuggle a denied name past the suffix check.
        assert_eq!(
            screen("https://localhost./v1"),
            Err(EgressDenial::PrivateAddress)
        );
    }

    #[test]
    fn every_denial_has_a_stable_string() {
        let all = [
            EgressDenial::MalformedUrl,
            EgressDenial::UrlTooLong,
            EgressDenial::SchemeDenied,
            EgressDenial::UserInfoDenied,
            EgressDenial::PortDenied,
            EgressDenial::HostDenied,
            EgressDenial::PathDenied,
            EgressDenial::PrivateAddress,
            EgressDenial::RedirectDenied,
            EgressDenial::Unresolved,
            EgressDenial::ScopeMalformed,
            EgressDenial::EmptyAllowlist,
        ];
        let mut seen = std::collections::HashSet::new();
        for denial in all {
            assert!(denial.as_str().starts_with("egress_"));
            assert!(seen.insert(denial.as_str()), "{denial:?} reuses a string");
            assert_eq!(denial.to_string(), denial.as_str());
        }
    }

    #[test]
    fn the_address_classifier_covers_the_special_use_ranges() {
        let public = [
            "93.184.216.34",
            "8.8.8.8",
            "1.1.1.1",
            "2606:4700:4700::1111",
        ];
        for raw in public {
            assert!(address_is_public(raw.parse().unwrap()), "{raw} is public");
        }
        let private = [
            "10.1.2.3",
            "172.31.255.255",
            "192.168.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
            "0.0.0.0",
            "::",
            "::1",
            "fc00::1",
            "fd00:ec2::254",
            "fe80::1",
            "ff02::1",
            "2001:db8::1",
            "100::1",
            "::ffff:10.0.0.1",
        ];
        for raw in private {
            assert!(
                !address_is_public(raw.parse().unwrap()),
                "{raw} is not public"
            );
        }
    }
}
