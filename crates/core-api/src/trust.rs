//! trust — the trusted-peer predicate for client-supplied network
//! identity (Wave 47 `p47-client-cert-auth`, shared with the
//! trusted-proxy-client-ip lane).
//!
//! Purpose: a forwarded assertion about the caller (the presented client
//! certificate, or `X-Forwarded-For`) is only meaningful when the request
//! arrives from a hop the deployment trusts. This module is that ONE
//! predicate: given the connecting peer's IP and the operator's trust
//! list, decide trust. Without it, anyone could forge the header and
//! impersonate a service account. The default list is loopback + private
//! ranges (every real deployment fronts core-api with an edge proxy on a
//! private network), and an operator replaces it with `MARULA_TRUSTED_PROXIES`
//! when the proxy IPs differ.
//!
//! Non-purpose: no header parsing (callers own which header carries the
//! assertion), no socket plumbing (the caller extracts the peer IP from
//! `ConnectInfo`), no DNS/CIDR auto-discovery. A pure, unit-tested
//! function over the operator's list.

use std::net::IpAddr;
#[cfg(test)]
use std::net::Ipv4Addr;

/// A single trusted network entry (an IP or an IP/prefix).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    net: IpAddr,
    bits: u8,
}

impl Cidr {
    /// Parses `10.0.0.0/8`, `192.168.1.5`, `::1`, `fe80::/10`, etc. A bare
    /// IP is a single-host CIDR. Returns `None` for a malformed address,
    /// a non-numeric prefix, or a prefix longer than the family allows.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let (ip_part, bits) = match s.split_once('/') {
            Some((ip, bits)) => (ip.trim(), Some(bits.trim())),
            None => (s, None),
        };
        let net: IpAddr = ip_part.parse().ok()?;
        let max = family_bits(net);
        let bits = match bits {
            Some(b) => {
                let b: u8 = b.parse().ok()?;
                if b > max {
                    return None;
                }
                b
            }
            None => max,
        };
        Some(Self { net, bits })
    }

    fn contains(&self, ip: IpAddr) -> bool {
        match (self.net, ip) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let m = v4_mask(self.bits);
                (u32::from(net) & m) == (u32::from(ip) & m)
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let m = v6_mask(self.bits);
                (u128::from(net) & m) == (u128::from(ip) & m)
            }
            // Different families never match a single entry.
            _ => false,
        }
    }
}

fn family_bits(ip: IpAddr) -> u8 {
    match ip {
        IpAddr::V4(_) => 32,
        IpAddr::V6(_) => 128,
    }
}

fn v4_mask(bits: u8) -> u32 {
    match bits {
        0 => 0,
        b if b >= 32 => u32::MAX,
        b => u32::MAX << (32 - b as u32),
    }
}

fn v6_mask(bits: u8) -> u128 {
    match bits {
        0 => 0,
        b if b >= 128 => u128::MAX,
        b => u128::MAX << (128 - b as u32),
    }
}

/// The built-in default trust list: loopback + RFC1918 private ranges +
/// IPv6 unique-local + link-local. Every first-party deployment fronts
/// core-api with an edge proxy on one of these, so forwarded assertions
/// work out of the box while a stray public client is still refused.
pub fn default_cidrs() -> Vec<Cidr> {
    [
        "127.0.0.0/8",
        "10.0.0.0/8",
        "172.16.0.0/12",
        "192.168.0.0/16",
        "::1/128",
        "fc00::/7",
        "fe80::/10",
    ]
    .iter()
    .filter_map(|s| Cidr::parse(s))
    .collect()
}

/// Resolves the operator's list into CIDRs. An EMPTY list means the
/// built-in default (loopback + private); a NON-empty list replaces it,
/// with malformed entries dropped (an operator fixing a typo must not take
/// the core down — the rest still applies).
pub fn resolve_trusted(configured: &[String]) -> Vec<Cidr> {
    let parsed: Vec<Cidr> = configured
        .iter()
        .filter_map(|entry| {
            let cidr = Cidr::parse(entry);
            if cidr.is_none() {
                tracing::warn!(entry = %entry, "MARULA_TRUSTED_PROXIES entry dropped: malformed");
            }
            cidr
        })
        .collect();
    if parsed.is_empty() {
        default_cidrs()
    } else {
        parsed
    }
}

/// Whether `ip` is trusted given the operator's configured list
/// (`Config.trusted_proxies`). An empty configured list falls back to the
/// loopback + private default. This is the single predicate both the
/// client-certificate and forwarded-client-IP trust decisions call.
pub fn peer_is_trusted(configured: &[String], ip: IpAddr) -> bool {
    resolve_trusted(configured).iter().any(|c| c.contains(ip))
}

/// The single normalizer for a client-certificate fingerprint, shared by
/// BOTH the enrollment side (`routes::service_accounts::create`) and the
/// auth side (`auth::middleware::cert_presentation`). One owner keeps the
/// two ends of the enroll-vs-auth invariant coupled: the key enrollment
/// stores is exactly the key authentication looks up. Separators and
/// whitespace are stripped so the common `AB:CD:…`/spaced encodings land on
/// the same key, and the result must be exactly 64 lowercase hex digits
/// (a SHA-256 digest); anything else is rejected.
pub fn normalize_fingerprint(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_ascii_lowercase();
    (cleaned.len() == 64).then_some(cleaned)
}

// Named address constants keep the tests free of `unwrap`-on-parse.
#[cfg(test)]
fn v4(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(a, b, c, d))
}
#[cfg(test)]
fn v6(s: &str) -> IpAddr {
    s.parse().expect("valid ipv6 literal in test")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_and_private_are_trusted_by_default() {
        assert!(peer_is_trusted(&[], v4(127, 0, 0, 1)));
        assert!(peer_is_trusted(&[], v4(10, 1, 2, 3)));
        assert!(peer_is_trusted(&[], v4(172, 16, 0, 1)));
        assert!(peer_is_trusted(&[], v4(172, 31, 255, 255)));
        assert!(peer_is_trusted(&[], v4(192, 168, 0, 10)));
        assert!(peer_is_trusted(&[], v6("::1")));
    }

    #[test]
    fn a_public_peer_is_not_trusted_by_default() {
        assert!(!peer_is_trusted(&[], v4(8, 8, 8, 8)));
        assert!(!peer_is_trusted(&[], v4(172, 32, 0, 1))); // just past 172.16/12
        assert!(!peer_is_trusted(&[], v4(11, 0, 0, 1)));
        assert!(!peer_is_trusted(&[], v6("2606:4700::1")));
    }

    #[test]
    fn operator_list_replaces_the_default() {
        let cfg = vec!["203.0.113.0/24".to_string()];
        assert!(peer_is_trusted(&cfg, v4(203, 0, 113, 42)));
        // The default loopback is NO LONGER trusted once the operator
        // narrows the set — a forged header from an untrusted hop is
        // refused even on loopback.
        assert!(!peer_is_trusted(&cfg, v4(127, 0, 0, 1)));
    }

    #[test]
    fn malformed_entries_are_dropped_not_fatal() {
        // A single good entry among junk still resolves and applies.
        let cfg = vec![
            "not-an-ip".to_string(),
            "10.0.0.0/99".to_string(), // prefix too long -> dropped
            "192.168.0.0/16".to_string(),
        ];
        assert!(peer_is_trusted(&cfg, v4(192, 168, 7, 7)));
        assert!(!peer_is_trusted(&cfg, v4(10, 0, 0, 1)));
    }

    #[test]
    fn all_malformed_falls_back_to_default() {
        let cfg = vec!["junk".to_string(), "999.999.999.999".to_string()];
        // Nothing parsed -> default list still guards the deployment.
        assert!(peer_is_trusted(&cfg, v4(127, 0, 0, 1)));
        assert!(!peer_is_trusted(&cfg, v4(8, 8, 8, 8)));
    }

    #[test]
    fn cross_family_never_matches() {
        let cfg = vec!["127.0.0.0/8".to_string()];
        assert!(!peer_is_trusted(&cfg, v6("::1")));
    }

    #[test]
    fn single_host_entry_requires_exact_match() {
        let cfg = vec!["192.0.2.9".to_string()];
        assert!(peer_is_trusted(&cfg, v4(192, 0, 2, 9)));
        assert!(!peer_is_trusted(&cfg, v4(192, 0, 2, 10)));
    }

    #[test]
    fn fingerprint_normalization_is_enroll_auth_parity() {
        // The contiguous key that both sides reduce to.
        let base = "ab12cd34ef56ab78";
        let raw = format!("{base}{base}{base}{base}"); // 64 hex, lowercase
        let canonical = normalize_fingerprint(&raw).expect("64-hex is valid");

        // Every common wire encoding a proxy actually emits reduces to the
        // SAME key, so what enrollment stored is what authentication looks
        // up. (A `0x` prefix is deliberately NOT a form: stripping its `x`
        // would leave a stray `0` and corrupt the digest.)
        let colon = raw
            .chars()
            .collect::<Vec<_>>()
            .chunks(2)
            .map(|c| c.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join(":");
        let spaced = colon.replace(':', " ");
        let upper = raw.to_uppercase();
        let colon_upper = colon.to_uppercase();
        for form in [raw.clone(), colon, spaced, upper, colon_upper] {
            assert_eq!(
                normalize_fingerprint(&form).as_deref(),
                Some(canonical.as_str()),
                "form must normalize to the enrolled key"
            );
        }
        assert_eq!(canonical, raw);
    }

    #[test]
    fn fingerprint_rejects_wrong_length_or_non_hex() {
        assert!(normalize_fingerprint("short").is_none());
        assert!(normalize_fingerprint(& "zz".repeat(32)).is_none()); // 64 non-hex
        assert!(normalize_fingerprint(& "ab".repeat(31)).is_none()); // 62 hex
        assert!(normalize_fingerprint("").is_none());
    }
}
