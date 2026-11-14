# adapters-core

Purpose: the shared transport crate every first-party connector builds on. It
holds vendor auth, pagination flows, the operator's org mapping, the canonical
vendor-field helpers, and the DNS record contract. Apache-2.0, support crate,
not a connector: it carries no vendor identity, no manifest and no standard-set
row. Workspace member. LANDED in Wave 12 lane 9, extended by Wave 15 lane 4 and
by Wave 47 lanes A8 and A11.

Why it exists: the semantic half of the adapter pattern already lives in the
feeder and DMS engines. What those crates do not carry is the transport half,
the part every live vendor pull needs and no two vendors do the same way. That
half is defined here, once.

## What an adapter gets

- **Auth** (`auth::VendorAuth`). A static key under a named header (IT Glue:
  `x-api-key`), a bearer token the operator supplies (ImmyBot, Hudu), or the
  client credentials grant used by M365 Graph. The grant is cached for the
  process, keyed by token endpoint and client id, and refreshed a minute before
  expiry so a long pull never rides a stale bearer. Every `Debug` impl redacts
  the secret.
- **Pagination** (`client::VendorClient`). One base URL, one auth flow, and the
  three page flows the vendors actually use: `@odata.nextLink` continuations,
  JSON:API `links.next` with `page[number]`, and plain page/size parameters that
  stop at the first short page. `fetch_all` follows every page and returns a
  typed error rather than a truncated list.
- **Typed errors** (`VendorError`). `Unauthorized` for 401 and 403,
  `RateLimited` for 429, `Rejected` for any other refusal, `Network` for
  transport. The vendor's own message is kept verbatim inside the variant, so a
  run report never paraphrases a vendor.
- **Org mapping** (`org_map`). The operator declares which vendor organization
  belongs to which Marula client. An undeclared organization is an error, never
  a guess, so a record cannot be attributed to a client nobody named.
- **Field extraction** (`vendor_fields`). `first_str`, `string_list`,
  `observed_fields` and their `_in` variants. These sit on the CI3 identity
  path, so a connector that copies them would split convergence for the same
  physical device. Use this crate's version.
- **DNS contract** (`dns`). `DnsRecordProvider` covers zone discovery, the
  `_marula-verify` TXT lifecycle, and a proxied-aware edge record upsert.
  `NoneDnsRecordProvider` returns the manual paste instructions and the
  deterministic TXT matcher, so custom domains never depend on a provider.

## The one rule, and its one exception

**Read this crate first if you are writing a third-party connector.** It is the
scaffolding, and a new adapter is a thin layer on top of it: depend on this
crate, construct one `VendorClient`, choose one `PageFlow`, resolve every vendor
organization through `OrgMapping` before the first write, and push through your
own sink over the public API.

What an adapter must not do is import a `crates/` module (R2). If a connector
needs a core type to do its job, the seam is the wrong shape; that is an RFC to
Core, not a dependency to add. The set has exactly one exception: `mapbox`
implements `core-routing`'s `RoutingProvider`, and its README says so. This
crate imports nothing from `crates/`.

## What it does not do

- No retries and no backoff. A rate limit surfaces as `RateLimited` and the
  caller decides when to try again.
- No credential storage, rotation or vaulting. Secrets arrive just in time from
  the operator's own store.
- No sinks and no Marula writes. Transport ends at the connector, which pushes
  through the public API.
- No vendor payload schemas. Each connector owns its own shapes.
- No manifest and no surfaces. A connector is the artifact, not this crate.

## None mode

Transport is not a run. The none sink belongs to the connector that owns the
write, and each ships its own: `NoneExportSink` for the export crates,
`NoneFeederSink` for the feeders, `NoneCallLogSink` for the telephony adapter.
The exception is `NoneDnsRecordProvider` here, because the DNS contract is
defined here, and its manual instructions are a real answer rather than a stub.

## Tests

`cargo test -p adapters-core`. Stub servers built with axum reproduce each flow
offline: the auth header on every page, bearer auth, one token fetch followed by
a cache hit, `nextLink` continuations, JSON:API pages, short-page termination,
the typed error per status, a refused token grant, and the org mapping's refusal
to guess. The `vendor_fields` behaviour is pinned separately, since a change
there moves identity convergence. The DNS contract's behaviour is asserted in
the `cloudflare` suite and in Core's own verification tests.

## Licence

Apache-2.0, separate from the AGPL core, and the same licence as the rest of the
standard set. No entitlement check anywhere (R11).
