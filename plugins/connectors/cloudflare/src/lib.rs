//! First-party Cloudflare connector (ADR 0013 standard set, Wave 47 lane A11
//! `p47-cloudflare-plugin`).
//!
//! This crate is the reference DNS provider for Marula's per-org custom-domain
//! seam. It rides the `adapters-core` transport like every other first-party
//! connector, and it is **not a hosted OAuth app**: the credential is an API
//! token the operator creates at Cloudflare → My Profile → API Tokens, scoped
//! to `Zone:DNS:Edit` (plus the optional read-only scopes the edge-posture
//! helpers use), and stored in Core's existing org-settings / BYOK secret
//! surface. Operating OAuth connections is the commercial layer's job, per
//! `plugins/connectors/README.md` and ADR 0013.
//!
//! ## What it does
//!
//! * **Custom-domain DNS automation** ([`dns::CloudflareDnsClient`]) —
//!   discovers the authoritative zone for a custom domain by walking parent
//!   labels against `GET /zones?name=`, then idempotently creates/updates and
//!   removes the `_marula-verify.<domain>` `TXT` record carrying the issued
//!   verification token, and upserts the operator-declared platform edge
//!   `A`, `AAAA`, or `CNAME` record in proxy-mode-aware form.
//! * **Edge posture checks for the V2 TLS runbook**
//!   ([`posture::CloudflareEdgePosture`]) — read-only (never a write): the
//!   zone's SSL/TLS encryption mode against the Full (strict) requirement,
//!   the Origin CA certificate inventory, and whether the hostname's
//!   `A`/`AAAA`/`CNAME` record is orange-cloud proxied, packaged as a
//!   structured checklist for `docs/site/reference/reverse-proxy-tls.md`.
//! * **None mode** — the vendor-agnostic contract lives in `adapters-core`
//!   ([`NoneDnsRecordProvider`]) and is re-exported here. Without this crate
//!   the operator pastes the same records by hand at any DNS host and Core's
//!   system-resolver TXT verifier validates them identically, so the plugin
//!   is pure convenience and never a dependency.
//!
//! ## None mode (honest posture)
//!
//! With the plugin absent or disabled, [`NoneDnsRecordProvider`] produces the
//! manual record instructions and the deterministic TXT matcher that mirrors
//! Core's verification, so the custom-domain feature works with zero DNS
//! integration: No Cloudflare token, no zone access, no network calls of this
//! crate's own.

pub mod api;
pub mod dns;
pub mod posture;

#[cfg(test)]
mod tests;

pub use api::{CloudflareApi, CloudflareConfig, CloudflareError, CLOUDFLARE_API_V4};
pub use dns::CloudflareDnsClient;
pub use posture::{
    CloudflareEdgePosture, EdgePostureReport, OriginCaCertificate, SslMode, ZoneSslPosture,
};

// The vendor-agnostic DNS-record-management contract is defined in
// `adapters-core::dns` and re-exported here so lane A8 (`p47-org-custom-domains`)
// and any consumer can take the contract and this implementation from one
// import path without a second dependency decision.
pub use adapters_core::dns::{
    candidate_zone_names, normalize_domain, verification_record_name, DnsProviderError,
    DnsRecordKind, DnsRecordProvider, DnsRecordState, DnsZone, EdgeRecordSpec, EdgeTargetKind,
    ManualDnsInstructions, ManualDnsRecord, NoneDnsRecordProvider, VERIFICATION_TXT_PREFIX,
};
