//! Trusted-proxy client-IP resolution for edge deployments (Wave 47 lane
//! A10, `p47-trusted-proxy-client-ip`).
//!
//! Why this exists: the previous resolver read the FIRST
//! `X-Forwarded-For` hop unconditionally, which is only correct when the
//! API's direct peer is the proxy that wrote it. Two failure modes follow:
//!
//! * a caller that reaches the origin directly can set the header itself,
//!   so the per-IP throttle keys become attacker-chosen (they poison the
//!   noise bucket — the per-actor keys stay authoritative, so this is an
//!   attribution/telemetry problem, not a lockout bypass);
//! * a Cloudflare-fronted deployment never sees `CF-Connecting-IP`, which
//!   is Cloudflare's canonical real-client header. Cloudflare APPENDS the
//!   client IP to `X-Forwarded-For`, so with any other proxy in the path
//!   the first hop is attacker-controllable.
//!
//! The fix walks the proxy chain from the DIRECT PEER (the one fact the
//! socket proves) right-to-left through `X-Forwarded-For`, trusting only
//! the hops the operator declared. An undeclared peer means the headers
//! carry no authority and the peer address itself is the answer.
//!
//! Trust comes from exactly one place: the trusted set (`TRUSTED_PROXIES`,
//! default loopback + RFC1918). Everything in this module is pure and
//! synchronous except the tiny [`stash`] middleware that resolves once per
//! request and publishes the answer as a [`ClientIp`] extension — there
//! are no per-route variants.

use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

use axum::extract::{ConnectInfo, Request};
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;

/// The sentinel used when no address can be attributed at all (no peer on
/// the request and no parseable proxy header). It keeps the historical
/// shared-bucket shape: unrelated callers collapse into one noisy key.
pub const UNKNOWN: &str = "unknown";

/// Cloudflare's published edge ranges, carried as a VERSIONED CONSTANT
/// rather than a stale local copy of a moving list.
///
/// Source: <https://www.cloudflare.com/ips/> (page states "Last updated:
/// Sep 28, 2023"). `CF_IP_RANGES` extends this set at boot, so an operator
/// can absorb a Cloudflare range change without waiting for a release —
/// the constant is the documented baseline, not the ceiling.
///
/// This set answers ONE question: "is this peer a Cloudflare edge?" It
/// never grants trust by itself (see [`TrustedProxies::believes_cf_peer`]).
const CLOUDFLARE_IP_RANGES: &[&str] = &[
    // IPv4 — https://www.cloudflare.com/ips-v4/
    "173.245.48.0/20",
    "103.21.244.0/22",
    "103.22.200.0/22",
    "103.31.4.0/22",
    "141.101.64.0/18",
    "108.162.192.0/18",
    "190.93.240.0/20",
    "188.114.96.0/20",
    "197.234.240.0/22",
    "198.41.128.0/17",
    "162.158.0.0/15",
    "104.16.0.0/13",
    "104.24.0.0/14",
    "172.64.0.0/13",
    "131.0.72.0/22",
    // IPv6 — https://www.cloudflare.com/ips-v6/
    "2400:cb00::/32",
    "2606:4700::/32",
    "2803:f800::/32",
    "2405:b500::/32",
    "2405:8100::/32",
    "2a06:98c0::/29",
    "2c0f:f248::/32",
];

/// The default trust set, deliberate and narrow: loopback (so the test
/// harness and any same-host proxy keeps working) plus RFC1918 private
/// space (so an in-VPC load balancer is believed). Everything else —
/// every public address — is untrusted, which is the safe default for an
/// internet-reachable bind: headers from a public peer are ignored.
pub const DEFAULT_TRUSTED_RANGES: &[&str] = &[
    "127.0.0.0/8",
    "::1/128",
    "10.0.0.0/8",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "fc00::/7",
];

/// One parsed CIDR block (IPv4 or IPv6). Right-aligned address bits so a
/// single mask expression covers both families.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    v4: bool,
    network: u128,
    prefix: u8,
}

impl Cidr {
    /// Parses `a.b.c.d/n`, `2001:db8::/n`, or a bare address (`/32`,
    /// `/128` implied). Bracketed IPv6 (`[2001:db8::]/32`) is accepted
    /// because operators copy them from proxy docs. `None` for anything
    /// malformed — callers drop the entry loudly instead of failing boot.
    pub fn parse(spec: &str) -> Option<Self> {
        let spec = spec.trim();
        if spec.is_empty() {
            return None;
        }
        let (addr_part, prefix_part) = match spec.split_once('/') {
            Some((addr, prefix)) => (addr, Some(prefix)),
            None => (spec, None),
        };
        let addr_part = addr_part.trim();
        let addr_part = match addr_part.strip_prefix('[') {
            Some(rest) => rest.strip_suffix(']').unwrap_or(rest),
            None => addr_part,
        };
        let addr = IpAddr::from_str(addr_part).ok()?;
        let width = match addr {
            IpAddr::V4(_) => 32u32,
            IpAddr::V6(_) => 128,
        };
        let prefix = match prefix_part {
            Some(p) => {
                let p = p.trim().parse::<u32>().ok()?;
                if p > width {
                    return None;
                }
                u8::try_from(p).ok()?
            }
            None => u8::try_from(width).ok()?,
        };
        Some(Self {
            v4: matches!(addr, IpAddr::V4(_)),
            network: addr_bits(addr).1,
            prefix,
        })
    }

    /// True when `ip` sits inside this block. Families never cross-match:
    /// an IPv4 block does not contain an IPv6 address and vice versa.
    pub fn contains(&self, ip: IpAddr) -> bool {
        let (v4, bits) = addr_bits(ip);
        if v4 != self.v4 {
            return false;
        }
        let width: u32 = if self.v4 { 32 } else { 128 };
        let host_bits = width - u32::from(self.prefix);
        // Mask over the address's OWN width, then compare in u128 space
        // (the address occupies the low `width` bits).
        let mask: u128 = match (self.v4, host_bits) {
            (_, h) if h >= width => 0,
            (true, h) => u128::from(u32::MAX << h),
            (false, h) => u128::MAX << h,
        };
        (bits & mask) == (self.network & mask)
    }
}

/// Canonical (family, bits) for an address. IPv4-mapped IPv6
/// (`::ffff:10.0.0.1`) folds onto its IPv4 value so a proxy that emits
/// either spelling lands in the same bucket.
fn addr_bits(ip: IpAddr) -> (bool, u128) {
    match ip {
        IpAddr::V4(v4) => (true, u128::from(u32::from(v4))),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(mapped) => (true, u128::from(u32::from(mapped))),
            None => (false, u128::from(v6)),
        },
    }
}

/// How much authority `CF-Connecting-IP` carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CfConnectingIp {
    /// Believe it only when the direct peer is BOTH a known Cloudflare
    /// range and trusted. Default: a non-Cloudflare deployment behaves
    /// exactly as if the header did not exist.
    #[default]
    Auto,
    /// Believe it whenever the direct peer is trusted (an internal Cloud
    /// Worker/Tunnel edge or another proxy that echoes the header).
    Always,
    /// Ignore the header entirely.
    Never,
}

impl CfConnectingIp {
    /// `auto` | `true` | `false` (case-insensitive). `None` is a config
    /// typo: the caller logs it and keeps the safe default.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" | "" => Some(Self::Auto),
            "true" | "1" | "yes" | "on" => Some(Self::Always),
            "false" | "0" | "no" | "off" => Some(Self::Never),
            _ => None,
        }
    }
}

/// The resolved trusted-proxy posture for this process: which peers may
/// speak for someone else, which peers are Cloudflare edges, and how much
/// authority `CF-Connecting-IP` carries.
#[derive(Debug, Clone)]
pub struct TrustedProxies {
    trusted: Vec<Cidr>,
    cloudflare: Vec<Cidr>,
    cf_connecting_ip: CfConnectingIp,
    /// True when `TRUSTED_PROXIES` was not set, i.e. the narrow default
    /// set is in force. The boot sanity check keys off this.
    default_set: bool,
}

impl Default for TrustedProxies {
    fn default() -> Self {
        Self::from_ranges(
            parse_ranges(DEFAULT_TRUSTED_RANGES.join(",").as_str()).0,
            CLOUDFLARE_IP_RANGES.join(",").as_str(),
            CfConnectingIp::Auto,
            true,
        )
    }
}

impl TrustedProxies {
    /// Builds a set from already-parsed parts. Test-facing; production
    /// goes through [`TrustedProxies::from_env`].
    pub fn from_ranges(
        trusted: Vec<Cidr>,
        cloudflare_spec: &str,
        cf_connecting_ip: CfConnectingIp,
        default_set: bool,
    ) -> Self {
        Self {
            trusted,
            cloudflare: parse_ranges(cloudflare_spec).0,
            cf_connecting_ip,
            default_set,
        }
    }

    /// Reads `TRUSTED_PROXIES`, `CF_CONNECTING_IP` and `CF_IP_RANGES`.
    ///
    /// `TRUSTED_PROXIES` is comma-separated CIDRs and REPLACES the narrow
    /// default (an edge deployment names its own edges instead of
    /// inheriting "every RFC1918 host is a proxy"). Malformed entries are
    /// dropped with a `tracing::warn!` naming them — an operator fixing a
    /// typo must not have to restart a crashed API to read the message,
    /// exactly like the plugin-signing trust-set parse.
    pub fn from_env() -> Self {
        let (raw_trusted, set_by_operator) = match std::env::var("TRUSTED_PROXIES") {
            Ok(value) => (value, true),
            Err(_) => (DEFAULT_TRUSTED_RANGES.join(","), false),
        };
        let (trusted, dropped) = parse_ranges(&raw_trusted);
        for entry in &dropped {
            tracing::warn!(
                entry = %entry,
                "TRUSTED_PROXIES entry is not a valid CIDR — ignored (prefixes the REAL client IP resolution)"
            );
        }
        if set_by_operator && trusted.is_empty() {
            tracing::warn!(
                "TRUSTED_PROXIES is set but empty of valid CIDRs — no peer is trusted, so every \
                 request resolves to its direct peer address"
            );
        }
        let cf_mode = match std::env::var("CF_CONNECTING_IP") {
            Ok(value) => match CfConnectingIp::parse(&value) {
                Some(mode) => mode,
                None => {
                    tracing::warn!(
                        value = %value,
                        "CF_CONNECTING_IP must be auto | true | false — keeping 'auto'"
                    );
                    CfConnectingIp::Auto
                }
            },
            Err(_) => CfConnectingIp::Auto,
        };
        // CF_IP_RANGES EXTENDS the versioned constant (Cloudflare publishes
        // new ranges between releases); it never replaces the baseline, so
        // an operator cannot accidentally shrink the known-edge set.
        let mut cloudflare = parse_ranges(CLOUDFLARE_IP_RANGES.join(",").as_str()).0;
        if let Ok(extra) = std::env::var("CF_IP_RANGES") {
            let (parsed, dropped) = parse_ranges(&extra);
            cloudflare.extend(parsed);
            for entry in &dropped {
                tracing::warn!(
                    entry = %entry,
                    "CF_IP_RANGES entry is not a valid CIDR — ignored (extends the Cloudflare range constant)"
                );
            }
        }
        Self {
            trusted,
            cloudflare,
            cf_connecting_ip: cf_mode,
            default_set: !set_by_operator,
        }
    }

    /// True when the narrow default set is in force (`TRUSTED_PROXIES`
    /// unset). The boot check warns on internet-reachable binds with this.
    pub fn is_default(&self) -> bool {
        self.default_set
    }

    /// True when this peer is allowed to speak for another address.
    pub fn is_trusted(&self, ip: IpAddr) -> bool {
        self.trusted.iter().any(|cidr| cidr.contains(ip))
    }

    /// True when this peer sits in a known Cloudflare range.
    pub fn is_cloudflare_peer(&self, ip: IpAddr) -> bool {
        self.cloudflare.iter().any(|cidr| cidr.contains(ip))
    }

    /// Whether `CF-Connecting-IP` may be believed from `peer`. Being a
    /// Cloudflare edge is NOT itself trust — the operator still has to
    /// declare those ranges in `TRUSTED_PROXIES`, which is what keeps a
    /// single authority for "who may speak for someone else".
    fn believes_cf_peer(&self, peer: IpAddr) -> bool {
        match self.cf_connecting_ip {
            CfConnectingIp::Never => false,
            CfConnectingIp::Always => self.is_trusted(peer),
            CfConnectingIp::Auto => self.is_trusted(peer) && self.is_cloudflare_peer(peer),
        }
    }

    /// The real client IP, resolved from the direct peer.
    ///
    /// `peer` is the socket address the connection came from (`None` only
    /// in harness paths that drive the router without connection info).
    ///
    /// 1. Untrusted peer ⇒ the peer address, headers ignored. A caller
    ///    that reached the origin directly chose its own headers.
    /// 2. Trusted peer ⇒ `CF-Connecting-IP` when believed, else the
    ///    right-to-left `X-Forwarded-For` walk: skip trusted hops, and the
    ///    FIRST untrusted hop from the right is the client (everything to
    ///    its left is attacker-controllable). Every hop trusted ⇒ the peer
    ///    itself is the client.
    /// 3. No peer signal ⇒ walk `X-Forwarded-For` right-to-left past
    ///    trusted hops (or the rightmost non-empty hop when a `oneshot`
    ///    test harness passes a synthetic bucket token), else [`UNKNOWN`].
    ///    This is the harness posture, never production: `core-api` is
    ///    served with connect info, so a real request always has a peer.
    pub fn resolve(&self, peer: Option<IpAddr>, headers: &HeaderMap) -> String {
        let Some(peer) = peer else {
            if let Some(client) = self.walk_forwarded_for(headers) {
                return client.to_string();
            }
            return rightmost_hop(headers).unwrap_or_else(|| UNKNOWN.to_owned());
        };
        if !self.is_trusted(peer) {
            return peer.to_string();
        }
        if self.believes_cf_peer(peer) {
            if let Some(ip) = cf_connecting_ip(headers) {
                return ip.to_string();
            }
        }
        if let Some(client) = self.walk_forwarded_for(headers) {
            return client.to_string();
        }
        peer.to_string()
    }

    /// Right-to-left walk past trusted hops. `None` when every hop is
    /// trusted (the peer is then the client) or no hop parses.
    fn walk_forwarded_for(&self, headers: &HeaderMap) -> Option<IpAddr> {
        for hop in forwarded_for_hops(headers).iter().rev() {
            // A proxy we trust appended this entry; what it forwarded sits
            // further left, so keep walking past it.
            let Some(ip) = parse_ip_field(hop) else {
                continue;
            };
            if self.is_trusted(ip) {
                continue;
            }
            // First untrusted hop from the right: the client. Its own
            // left-hand entries are attacker-supplied and never consulted.
            return Some(ip);
        }
        None
    }
}

/// Parses a comma-separated CIDR list. Returns the valid entries plus the
/// rejected raw strings so the caller can log them verbatim.
fn parse_ranges(spec: &str) -> (Vec<Cidr>, Vec<String>) {
    let mut parsed = Vec::new();
    let mut dropped = Vec::new();
    for entry in spec.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        match Cidr::parse(entry) {
            Some(cidr) => {
                if !parsed.contains(&cidr) {
                    parsed.push(cidr);
                }
            }
            None => dropped.push(entry.to_owned()),
        }
    }
    (parsed, dropped)
}

/// The `X-Forwarded-For` chain, left to right, in the order proxies wrote
/// it. Capped so a pathological header cannot burn CPU per request.
fn forwarded_for_hops(headers: &HeaderMap) -> Vec<&str> {
    const MAX_HOPS: usize = 64;
    let Some(raw) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) else {
        return Vec::new();
    };
    raw.split(',').take(MAX_HOPS).collect()
}

/// The rightmost hop — the harness posture (`oneshot`, no socket peer)
/// when `walk_forwarded_for` found no untrusted IP. Canonicalizes a real
/// address when one parses, and preserves synthetic per-test bucket tokens
/// (e.g. `114.9.10.<uuid>` in `auth_hardening_flow`) verbatim so existing
/// `oneshot` suites stay isolated without a socket.
fn rightmost_hop(headers: &HeaderMap) -> Option<String> {
    let hops = forwarded_for_hops(headers);
    if let Some(ip) = hops.iter().rev().find_map(|hop| parse_ip_field(hop)) {
        return Some(ip.to_string());
    }
    hops.iter()
        .rev()
        .map(|hop| hop.trim())
        .find(|hop| !hop.is_empty())
        .map(str::to_owned)
}

/// Cloudflare's real-client header.
fn cf_connecting_ip(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get("cf-connecting-ip")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_ip_field)
}

/// Parses one hop value. Tolerates the `[v6]:port` and `v4:port` spellings
/// some proxies emit, because a mis-parsed hop silently keys the throttle
/// bucket on the wrong value.
fn parse_ip_field(value: &str) -> Option<IpAddr> {
    let value = value.trim();
    if let Some(rest) = value.strip_prefix('[') {
        let (addr, _port) = rest.split_once(']')?;
        return IpAddr::from_str(addr).ok();
    }
    if let Ok(ip) = IpAddr::from_str(value) {
        return Some(ip);
    }
    // Exactly one colon with a numeric tail is `v4:port`; an IPv6 literal
    // without brackets has two or more colons and parses above.
    if value.matches(':').count() == 1 {
        let (addr, port) = value.split_once(':')?;
        if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) {
            return IpAddr::from_str(addr).ok();
        }
    }
    None
}

/// True when a bind address is reachable from off-box: anything that is
/// not pinned to loopback, including the unspecified `0.0.0.0`/`::`
/// (which binds every interface). Loopback binds are local-only.
pub fn bind_is_internet_reachable(bind_addr: &str) -> bool {
    let candidate = bind_addr.trim();
    if let Ok(ip) = IpAddr::from_str(candidate) {
        return !ip.is_loopback();
    }
    // `host:port`; a bare IPv6 literal already parsed above, so a colon
    // split here can only be a port separator.
    let host = match candidate.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => host,
        _ => candidate,
    };
    let host = host.trim_matches(|c| c == '[' || c == ']');
    match IpAddr::from_str(host) {
        Ok(ip) => !ip.is_loopback(),
        Err(_) => false,
    }
}

/// The resolved real client IP for the current request, published as a
/// request extension by [`stash`]. Handlers read it and hand it to the
/// hardening gates — no handler re-derives an IP from headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientIp(pub String);

/// Resolves the client IP once per request and publishes it as
/// [`ClientIp`]. Runs outermost in the router stack, so every surface
/// (auth gates today, audit metadata tomorrow) reads one answer produced
/// by one resolver.
///
/// The peer comes from axum's `ConnectInfo`, which is only populated when
/// the router is served with `into_make_service_with_connect_info` — the
/// binary does exactly that; anything driving the router directly (unit
/// tests, `oneshot`) simply has no peer and gets the documented harness
/// posture instead.
pub async fn stash(
    axum::extract::State(trusted): axum::extract::State<TrustedProxies>,
    mut request: Request,
    next: Next,
) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip());
    let ip = trusted.resolve(peer, request.headers());
    request.extensions_mut().insert(ClientIp(ip));
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::extract::Extension;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (key, value) in pairs {
            headers.insert(
                axum::http::HeaderName::from_str(key).expect("header name"),
                axum::http::HeaderValue::from_str(value).expect("header value"),
            );
        }
        headers
    }

    fn set() -> TrustedProxies {
        TrustedProxies::default()
    }

    fn ip(value: &str) -> IpAddr {
        IpAddr::from_str(value).expect("test ip")
    }

    /// Handler that echoes back what the middleware resolved.
    async fn echo_ip(
        axum::extract::Extension(ClientIp(ip)): axum::extract::Extension<ClientIp>,
    ) -> String {
        ip
    }

    // --- trust set -------------------------------------------------------

    #[test]
    fn default_set_covers_loopback_and_rfc1918_only() {
        let set = set();
        assert!(set.is_default());
        for trusted in [
            "127.0.0.1",
            "127.53.1.1",
            "10.1.2.3",
            "172.20.0.1",
            "192.168.9.9",
            "::1",
        ] {
            assert!(set.is_trusted(ip(trusted)), "{trusted} should be trusted");
        }
        for untrusted in [
            "8.8.8.8",
            "203.0.113.5",
            "172.32.0.1",
            "11.0.0.1",
            "2001:db8::1",
        ] {
            assert!(
                !set.is_trusted(ip(untrusted)),
                "{untrusted} must NOT be trusted"
            );
        }
    }

    #[test]
    fn explicit_list_replaces_the_default_set() {
        let set = TrustedProxies::from_ranges(
            parse_ranges("203.0.113.0/24").0,
            "",
            CfConnectingIp::Auto,
            false,
        );
        assert!(!set.is_default());
        assert!(set.is_trusted(ip("203.0.113.7")));
        assert!(
            !set.is_trusted(ip("127.0.0.1")),
            "the replaced default must not linger"
        );
    }

    #[test]
    fn malformed_entries_are_dropped_not_fatal() {
        let (parsed, dropped) =
            parse_ranges("10.0.0.0/8, not-a-cidr, 2001:db8::/129, ,192.168.0.0/16");
        assert_eq!(parsed.len(), 2);
        assert_eq!(
            dropped,
            vec!["not-a-cidr".to_owned(), "2001:db8::/129".to_owned()]
        );
    }

    #[test]
    fn overlapping_entries_do_not_double_count() {
        let (parsed, _) = parse_ranges("10.0.0.0/8, 10.1.0.0/16, 10.0.0.0/8");
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn families_never_cross_match() {
        let v4 = Cidr::parse("10.0.0.0/8").expect("v4");
        let v6 = Cidr::parse("fc00::/7").expect("v6");
        assert!(v4.contains(ip("10.9.9.9")));
        assert!(!v4.contains(ip("2001:db8::1")));
        assert!(v6.contains(ip("fd00::1")));
        assert!(!v6.contains(ip("10.9.9.9")));
    }

    #[test]
    fn v4_mapped_ipv6_folds_onto_its_ipv4_value() {
        let set = set();
        assert!(set.is_trusted(ip("::ffff:127.0.0.1")));
        assert!(!set.is_trusted(ip("::ffff:203.0.113.5")));
    }

    #[test]
    fn a_bare_address_means_a_single_host() {
        let cidr = Cidr::parse("203.0.113.7").expect("bare");
        assert!(cidr.contains(ip("203.0.113.7")));
        assert!(!cidr.contains(ip("203.0.113.8")));
    }

    #[test]
    fn bracketed_ipv6_parses() {
        let cidr = Cidr::parse("[2001:db8::]/32").expect("bracketed");
        assert!(cidr.contains(ip("2001:db8::dead")));
        assert!(!cidr.contains(ip("2001:db9::1")));
    }

    // --- resolution ------------------------------------------------------

    #[test]
    fn peer_address_wins_when_no_proxy_is_trusted() {
        let set = set();
        let headers = headers(&[("x-forwarded-for", "1.2.3.4, 5.6.7.8")]);
        assert_eq!(
            set.resolve(Some(ip("203.0.113.9")), &headers),
            "203.0.113.9"
        );
    }

    #[test]
    fn rightmost_untrusted_hop_is_the_client_when_a_proxy_is_trusted() {
        let set = set();
        // Peer is a trusted (RFC1918) load balancer; the caller pinned a
        // fake leftmost hop, the balancer appended the real client.
        let headers = headers(&[("x-forwarded-for", "1.2.3.4, 198.51.100.7, 10.0.0.9")]);
        assert_eq!(set.resolve(Some(ip("10.0.0.5")), &headers), "198.51.100.7");
    }

    #[test]
    fn a_chain_of_trusted_proxies_resolves_to_the_first_untrusted_hop() {
        let set = TrustedProxies::from_ranges(
            parse_ranges("10.0.0.0/8, 192.168.0.0/16").0,
            "",
            CfConnectingIp::Auto,
            false,
        );
        let headers = headers(&[("x-forwarded-for", "1.2.3.4, 10.0.0.9, 192.168.1.8")]);
        assert_eq!(set.resolve(Some(ip("10.0.0.5")), &headers), "1.2.3.4");
    }

    #[test]
    fn every_hop_trusted_falls_back_to_the_peer() {
        let set = set();
        let headers = headers(&[("x-forwarded-for", "127.0.0.1, 10.0.0.9")]);
        assert_eq!(set.resolve(Some(ip("127.0.0.1")), &headers), "127.0.0.1");
    }

    #[test]
    fn a_spoofed_header_from_an_untrusted_peer_is_ignored() {
        let set = set();
        let headers = headers(&[("x-forwarded-for", "1.2.3.4")]);
        assert_eq!(
            set.resolve(Some(ip("203.0.113.9")), &headers),
            "203.0.113.9"
        );
    }

    #[test]
    fn no_header_and_a_trusted_peer_resolves_to_the_peer() {
        let set = set();
        assert_eq!(
            set.resolve(Some(ip("10.0.0.5")), &HeaderMap::new()),
            "10.0.0.5"
        );
    }

    #[test]
    fn no_peer_and_no_header_is_the_shared_bucket() {
        assert_eq!(set().resolve(None, &HeaderMap::new()), UNKNOWN);
    }

    #[test]
    fn the_harness_posture_takes_the_rightmost_hop() {
        let set = set();
        // Unit-test/oneshot drives the router without connection info.
        let headers = headers(&[("x-forwarded-for", "1.2.3.4, 203.0.113.5")]);
        assert_eq!(set.resolve(None, &headers), "203.0.113.5");
    }

    #[test]
    fn hop_port_suffixes_are_tolerated() {
        let set = set();
        let headers = headers(&[("x-forwarded-for", "1.2.3.4:9999, [2001:db8::1]:443")]);
        assert_eq!(set.resolve(Some(ip("10.0.0.5")), &headers), "2001:db8::1");
    }

    #[test]
    fn an_unparseable_hop_is_skipped_not_fatal() {
        let set = set();
        let headers = headers(&[("x-forwarded-for", "garbage, 198.51.100.7")]);
        assert_eq!(set.resolve(Some(ip("10.0.0.5")), &headers), "198.51.100.7");
    }

    // --- Cloudflare ------------------------------------------------------

    fn cloudflare_set() -> TrustedProxies {
        // The documented edge posture: TRUSTED_PROXIES names Cloudflare.
        TrustedProxies::from_ranges(
            parse_ranges(CLOUDFLARE_IP_RANGES.join(",").as_str()).0,
            CLOUDFLARE_IP_RANGES.join(",").as_str(),
            CfConnectingIp::Auto,
            false,
        )
    }

    #[test]
    fn cf_connecting_ip_wins_only_from_a_trusted_cf_peer() {
        let set = cloudflare_set();
        let headers = headers(&[
            ("cf-connecting-ip", "198.51.100.23"),
            ("x-forwarded-for", "198.51.100.23"),
        ]);
        assert_eq!(
            set.resolve(Some(ip("104.16.0.7")), &headers),
            "198.51.100.23"
        );
        // Same header, but the peer is a random public host: not a CF
        // range, so the header is not believed and the peer wins.
        assert_eq!(
            set.resolve(Some(ip("203.0.113.9")), &headers),
            "203.0.113.9"
        );
    }

    #[test]
    fn cf_connecting_ip_from_a_non_cf_peer_falls_back_to_the_xff_walk() {
        let set = set();
        let headers = headers(&[
            ("cf-connecting-ip", "198.51.100.23"),
            ("x-forwarded-for", "198.51.100.23"),
        ]);
        // Default posture: an RFC1918 peer carrying the Cloudflare header
        // is not a known edge, so `auto` ignores it and walks XFF instead.
        assert_eq!(set.resolve(Some(ip("10.0.0.5")), &headers), "198.51.100.23");
    }

    #[test]
    fn cf_connecting_ip_false_disables_the_header_entirely() {
        let mut set = cloudflare_set();
        set.cf_connecting_ip = CfConnectingIp::Never;
        let headers = headers(&[
            ("cf-connecting-ip", "198.51.100.23"),
            ("x-forwarded-for", "203.0.113.44"),
        ]);
        assert_eq!(
            set.resolve(Some(ip("104.16.0.7")), &headers),
            "203.0.113.44"
        );
    }

    #[test]
    fn cf_connecting_ip_true_trusts_any_trusted_peer() {
        let set = TrustedProxies::from_ranges(
            parse_ranges("10.0.0.0/8").0,
            "",
            CfConnectingIp::Always,
            false,
        );
        let headers = headers(&[("cf-connecting-ip", "198.51.100.23")]);
        assert_eq!(set.resolve(Some(ip("10.0.0.5")), &headers), "198.51.100.23");
    }

    #[test]
    fn the_known_cf_set_matches_the_published_ranges() {
        let set = cloudflare_set();
        assert!(set.is_cloudflare_peer(ip("104.16.0.7")));
        assert!(set.is_cloudflare_peer(ip("2606:4700::1")));
        assert!(!set.is_cloudflare_peer(ip("203.0.113.9")));
    }

    #[test]
    fn cf_ip_ranges_extends_rather_than_replaces_the_constant() {
        let mut set = cloudflare_set();
        set.cloudflare.extend(parse_ranges("203.0.113.0/24").0);
        assert!(
            set.is_cloudflare_peer(ip("104.16.0.7")),
            "the baseline must survive an extension"
        );
        assert!(set.is_cloudflare_peer(ip("203.0.113.5")));
    }

    // --- middleware ------------------------------------------------------

    /// Builds a one-route router whose handler echoes the resolved IP,
    /// standing in for the real stack: `ConnectInfo` is inserted exactly
    /// where `into_make_service_with_connect_info` inserts it (outside
    /// every router layer), so the middleware under test sees the peer.
    fn echo_app(peer: SocketAddr) -> Router {
        Router::new()
            .route("/echo", get(echo_ip))
            // The peer insertion goes OUTSIDE the middleware under test —
            // exactly where `into_make_service_with_connect_info` puts it.
            .layer(axum::middleware::from_fn_with_state(set(), stash))
            .layer(Extension(ConnectInfo(peer)))
    }

    async fn resolved_for(app: &Router, xff: &str) -> String {
        let request = axum::http::Request::builder()
            .uri("/echo")
            .header("x-forwarded-for", xff)
            .body(Body::empty())
            .expect("request");
        let response = app.clone().oneshot(request).await.expect("response");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        String::from_utf8(body.to_vec()).expect("utf8 body")
    }

    #[tokio::test]
    async fn the_middleware_publishes_the_resolved_ip_as_an_extension() {
        let app = echo_app(SocketAddr::from(([203, 0, 113, 9], 40_000)));
        // Untrusted peer ⇒ the peer address, spoofed header dropped.
        assert_eq!(resolved_for(&app, "1.2.3.4").await, "203.0.113.9");
    }

    #[tokio::test]
    async fn the_middleware_walks_xff_past_a_trusted_peer() {
        let app = echo_app(SocketAddr::from(([10, 0, 0, 5], 40_001)));
        assert_eq!(
            resolved_for(&app, "1.2.3.4, 198.51.100.7").await,
            "198.51.100.7"
        );
    }

    // --- boot sanity -----------------------------------------------------

    #[test]
    fn only_internet_facing_binds_are_flagged() {
        assert!(bind_is_internet_reachable("0.0.0.0:8080"));
        assert!(bind_is_internet_reachable("[::]:8080"));
        assert!(bind_is_internet_reachable("203.0.113.10:8080"));
        assert!(!bind_is_internet_reachable("127.0.0.1:8080"));
        assert!(!bind_is_internet_reachable("[::1]:8080"));
        assert!(!bind_is_internet_reachable(""));
    }
}
