# connectors

> **Purpose:** First-party HTTP connectors (Apache-2.0). The standard set (ADR 0013) is NinjaOne, Hudu, M365 Graph, UniFi, QBO, Xero, 3CX, Slack/Teams, Rewst, plus the category references: the generic REST DMS push target (`dms-sync`), the signed-webhook alert ingestor, the vendor-agnostic DNS-record-management contract (`adapters-core::dns`), and its single reference provider (`cloudflare`) — reference implementations of the connector pattern, free forever, no account needed.
> **Non-purpose:** No hosted OAuth apps, credential vaulting, or sync-monitoring operations — hosting/operating connections commercially is out of scope for this repo. Connectors beyond the standard set publish through the marketplace and are built in the commercial repos (first-party: `relay/connectors/<name>`, one git repo per connector); nothing here ever carries an entitlement check. Every adapter documents its none-mode.

The shared support crates sit alongside the connectors: `adapters-core` (live-REST
transport: auth, pagination, org→client mapping, and the DNS-record-management
contract) and `ci-feeders` / `dms-sync` (the feeder and DMS engines whose public
mapper surfaces community connectors build on).

## DNS connector boundary (Wave 47 lanes A8 / A11)

Core's per-org custom-domain verification is generic DNS: it resolves
`_marula-verify.{domain}` `TXT` through the system resolver and compares the token.
That works with any DNS host, so no Core endpoint requires a DNS provider plugin, and
an operator can always paste the records by hand. The plugin is convenience.

The in-repo standard set therefore holds one category seam plus one reference provider
rather than a shelf of near-identical vendors:

- **`adapters-core::dns`** — the vendor-agnostic contract. `DnsRecordProvider`
  (zone discovery, `_marula-verify.<domain>` TXT lifecycle, proxy-mode-aware edge
  `A`/`AAAA`/`CNAME` upsert), with `NoneDnsRecordProvider` producing the manual
  paste instructions and the TXT matcher that mirrors Core's verification.
- **`cloudflare`** — the reference implementation, plus read-only edge-posture
  helpers for the TLS runbook (`GET /zones/{id}/settings/ssl`, the Origin CA
  inventory, and whether a hostname's record is orange-cloud proxied).

Per-vendor DNS plugins beyond that reference (Route 53, Gandi, Azure DNS, Google Cloud
DNS, and the rest) are marketplace connectors built in the commercial repos
(`marula-hq`), the same boundary as every other non-standard-set connector. The
Cloudflare credential is an operator-created API token scoped to `Zone:DNS:Edit`,
stored in Core's existing org-settings / BYOK secret surface — operating a hosted
OAuth connection is commercial-layer work and does not belong here.
