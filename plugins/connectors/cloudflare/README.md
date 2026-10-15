# cloudflare

Purpose: first-party Cloudflare connector (Apache-2.0, ADR 0013 standard set) —
the reference DNS provider for the per-org custom-domain seam. It rides the
`adapters-core` transport like every other connector in this directory.

Status: LANDED in lane `p47-cloudflare-plugin` (Wave 47 lane A11). It automates
custom-domain DNS records and reports Cloudflare edge posture. It does not run
any part of Marula.

## What it does

- **Zone discovery.** Given `support.msp.example.com`, the client asks
  `GET /zones?name=<candidate>` for `support.msp.example.com`, then
  `msp.example.com`, then `example.com`, and returns the first zone whose
  name matches a candidate and whose status is `active`. No matching zone
  produces a typed error that names every candidate it tried.
- **`_marula-verify.<domain>` TXT lifecycle.** Create or update the TXT
  record carrying the verification token Core issued, and remove it again.
  An existing row is updated in place, so re-running the call never
  duplicates the record, and removing an already-absent record succeeds.
- **Proxy-aware edge records.** Upsert the `A`, `AAAA`, or `CNAME` record
  the operator declares for the platform edge. The returned
  `DnsRecordState` reports `proxied: true` for an orange-cloud record and
  `proxied: false` for DNS-only, and a proxied record carries `ttl: 1`
  (Cloudflare's "automatic" setting) whatever the spec asked for.
- **Edge posture (read-only).** The zone's SSL/TLS encryption mode, the
  Origin CA certificate inventory for the zone, and whether the hostname's
  address or alias record is currently proxied, assembled into a checklist
  that matches `docs/site/reference/reverse-proxy-tls.md`.

## The TXT record is always unproxied

`_marula-verify.<domain>` is written with `proxied: false` every time,
whatever the edge record does. Core verifies domain ownership by resolving
that TXT value through the system resolver. A record behind the orange
cloud still serves the TXT string, but the proxied A/AAAA/CNAME beside it
resolves to the edge's IPs rather than the origin's, which is exactly the
condition the runbook calls out as "record plausibility rather than origin
IP equality". Keeping the challenge unproxied removes the variable.

## Credentials

This connector is not a hosted OAuth app. The credential is an API token
the operator creates at Cloudflare → My Profile → API Tokens. It lives in
Core's existing org-settings / BYOK secret surface. Nothing here refreshes
it, nothing stores it, and it never appears in a logged URL or in `Debug`
output (`CloudflareConfig` implements `Debug` by redaction).

| Call path | Scope |
|---|---|
| Zone discovery, TXT lifecycle, edge record upsert | `Zone:DNS:Edit` |
| `GET /zones/{id}/settings/ssl` | `Zone:Zone Settings:Read` |
| `GET /certificates?zone_id=` | `Zone:SSL and Certificates:Read` |

A 401 or 403 maps to `CloudflareError::Unauthorized` (and, through the
contract, to `DnsProviderError::UnauthorizedScope`) with the scope named in
the message, so an under-scoped token reads as an actionable error.

## Edge posture is a read

Nothing in this crate patches a zone SSL setting, mints a certificate, or
flips a record's proxy mode. The posture helpers exist so an operator can
check the V2 checklist instead of guessing at it:

| Posture question | Read |
|---|---|
| Does the zone meet the Edge-mode requirement? | `GET /zones/{id}/settings/ssl` reports `off \| flexible \| full \| strict` |
| Is there a live Origin CA certificate for the hostname? | `GET /certificates?zone_id=` returns id, hostnames, `expires_on`, revocation |
| Is the hostname orange-clouded? | the record list for the hostname, `A`/`AAAA`/`CNAME` only |

`ssl` mode `strict` satisfies the runbook requirement. `full` accepts any
certificate from the origin, `flexible` fetches the origin over plaintext
http, and `off` is off: each of those produces a warning string naming the
runbook section that fixes it rather than a bare boolean.

## None mode

Without this crate, the feature still works. `NoneDnsRecordProvider`
(re-exported here from `adapters-core::dns`) generates the records an
operator pastes into any DNS host:

```
_marula-verify.support.msp.example.com   TXT   180s   <token>
support.msp.example.com                  CNAME  1      edge.marula.example
```

Core then resolves `_marula-verify.support.msp.example.com` through the
system resolver and compares the token. That verification is what
`NoneDnsRecordProvider::verify_txt_records` mirrors, quoted or unquoted, so
the plugin is convenience and never a dependency.

## The contract it implements

The vendor-agnostic DNS-record-management trait lives in
`adapters_core::dns` (`DnsRecordProvider`, `EdgeRecordSpec`, `DnsZone`,
`DnsRecordState`, `ManualDnsInstructions`, `DnsProviderError`). It is
re-exported from this crate so a consumer can take both the contract and
its reference implementation from one import. Per-vendor DNS plugins beyond
this reference provider (Route 53, Gandi, Azure DNS, Google Cloud DNS) are
marketplace connectors built in the commercial repos.

## Tests

`src/tests.rs` runs against an in-process mock of the Cloudflare API that
holds zones and records in memory, records the request paths it saw, and can
refuse every request with 403 to reproduce an under-scoped token. It covers
zone discovery down the label walk and the refusal when no zone matches,
TXT create plus idempotent re-run, in-place token rotation, and idempotent
delete, edge `A`/`AAAA`/`CNAME` upserts in both proxy modes with the
`ttl: 1` rule and a TXT that stays unproxied beside a proxied record, the
typed authorization error for a token without the required scope, the edge
posture report in `strict` and in `flexible`, the Origin CA inventory
parsing, and the none-mode instructions round-tripping through the generic
verifier.
