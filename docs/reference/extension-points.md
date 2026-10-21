# Extension points

What a premium pack, connector, federation bundle or auth provider may build on
without changing the core. Nine points, each with what it grants, what it
refuses, and the core file that owns it.

Two rules sit above everything below.

1. **Core carries no tier knowledge.** No entitlement check, no plan column, no
   "premium only" branch. A pack is a pack because it is installed, and the core
   asks only what a principal may do (`crates/core-domain/src/policy.rs`).
2. **A grant is a subset of a permission the core already understands.** A
   plugin capability, an API-key scope and a role grant all resolve into the
   same `Permission` vocabulary. Nothing here is a second authorization system.

Where a point cannot support a pack today, the gap is written down as a gap.
A gap that is documented is a scheduling problem; a gap that is hidden becomes a
pack that cannot ship.

## The matrix

| Point | What a pack may do | What it may not do | Core owner |
|---|---|---|---|
| Plugin | Declare capabilities, contribute declarative surfaces, receive grants, emit declared event types | Hold an admin special; ship browser-executed code; run anything the core does not execute | `crates/core-domain/src/entities/plugins.rs` |
| Connector | Ingest HMAC-signed events; call scoped API paths; declare egress in its manifest | Path-depend on a core crate; read a scope it was not granted; write the audit trail | `crates/core-api/src/routes/webhook_receiver.rs` |
| Federation bundle | Add an identity source; read the group surface it expands | Mint sessions; hold a capability or scope; become the authority for a role | `crates/core-api/src/auth/federation.rs` |
| Auth provider | Present assertions through the bundle contract; JIT-provision under org policy | Bypass MFA; create a parallel session store; override the conservative link decision | `docs/reference/auth-provider-contract.md` |
| Webhook / event | Subscribe to events with a signed delivery ledger; publish declared event types | Invent an event type outside the registry; receive another org's envelope | `crates/core-api/src/routes/webhooks.rs` |
| Read model | Read the `ops.v1_*` views and the materialized usage row | Define a view of its own inside core; write a read model | `docs/reference/read-model.md` |
| OTel | Depend on the published metric, log-field and health contract | Write series into core; expect a metrics scrape endpoint | `crates/core-otel/src/contract.rs` |
| Audit | Export the trail incrementally with a stable cursor | Write, edit or delete an audit row; read past the org scope | `crates/core-api/src/routes/audit.rs` |
| Usage | Read per-org counts and quota settings; count its own ingest volume | Enforce a quota; attribute another tenant's traffic; add a counter table | `crates/core-api/src/routes/org_usage.rs` |

## Plugin

The declarative half is shipped: manifest v2, the closed capability vocabulary
(`PluginCapability`, thirty-three keys), per-install grants, digest-pinned
distribution, consent diffing, and surface profiles for widgets, automation
actions, report render kinds, field validators, trigger sources, read nodes and
event emission. `crates/plugin-host/src/lib.rs` authorises an invocation;
`crates/plugin-host/src/surface.rs` derives what an install may contribute.

A capability resolves onto an existing permission, never onto a new one. The
observability reader is the newest example: `observability.read` resolves onto
`observability:read`, which reads the audit trail and opens nothing else. Health
needs no capability at all, because `/api/v1/health` is public.

**Refused by construction.** No capability maps onto an admin special, and
`admin_specials_should_never_be_grantable` fails the build if one ever does. A
grant outside the manifest's declared set is rejected at install.

**Gap.** The core does not execute plugin code yet — `PluginRuntime` is a trait
with no sandbox behind it (`docs/reference/plugin-runtime.md`). Two consequences
for a pack: its outbound work cannot be metered per org, and it cannot call a
host function for anything, metering included. Until the executor lands, a pack
that needs per-tenant counting records it on its own side and reads core's
numbers through the usage point.

## Connector

A connector is an HTTP adapter that talks to the public API and nothing else.
Two ingest endpoints accept HMAC-signed bodies without a JWT: the generic
receiver (`POST /api/webhooks/ingest/{source_ref}`) and the alert ingestor
(`POST /api/alerts/ingest/{source_ref}`). Both resolve the org from the source
ref, verify the signature constant-time, fence replays on a dedup key, and
attribute the request to the org in `marula.org.ingest_requests`.

**Refused.** A connector may not import a core crate (the workspace members
under `plugins/connectors/` carry no path dependency into `crates/`), may not
hold a scope it was not granted (the API-key path list in
`crates/core-api/src/auth/middleware.rs` is an allowance, never a grant), and may
not write an audit row of its own.

## Federation bundle

A bundle adds an identity source and routes every login through the same token
machinery a password login uses. `FederationBundle` is versioned
(`crates/core-api/src/auth/federation.rs`) and registered from an installed
artifact: an install is an artifact, not an authority. Approval is not
activation, and a revoked install never registers.

The group surface a bundle expands is core's, not the bundle's:

- `crates/core-db/src/directory_groups.rs` resolves an external group id and
  lists the groups a user belongs to.
- `crates/core-db/src/users.rs` returns the effective roles, the additive union
  of direct grants and group grants.

**Refused.** A bundle cannot hold a capability or a scope — ingest refuses both
by name — and it never becomes the authority for a role. It reads the surface;
core decides what the surface means.

**Gap.** An out-of-tree bundle process has no HTTP read path to that surface. The
group routes require `special:manage_users` or `special:manage_roles`, which no
plugin may hold, and no `directory_groups:read` scope exists. An in-process
bundle reads the functions directly, which is what R12 wired; a separately
deployed sync service would need a read route and a scope, and that is a row, not
a patch.

## Auth provider

One source of assertions among several. A provider never decides a login: it
presents verified claims, and the conservative link decision
(`crates/core-api/src/auth/federation.rs`) plus the org MFA gate decide the rest.
The wire contract, presets and the assertion shape live in
`docs/reference/auth-provider-contract.md`.

## Webhook / event

Outbound subscriptions are org-scoped and signed, with a delivery ledger and a
test-fire path. Inbound machine traffic arrives through the ingest endpoints
above. Plugin-emitted events pass the event-type registry: an `events.emit` grant
lets a runtime publish a type it declared, and an unknown type is refused rather
than stored.

The live stream (`crates/core-api/src/routes/events.rs`) is a read of the same
outbox, not a second bus.

## Read model

Operational numbers are views, not tables a pack writes. The `ops` schema family
is defined with `security_invoker = true` (the convention established in
`crates/core-db/migrations/0026_reports_read_model.sql`), so row-level security
applies exactly as if the caller had queried the base tables. Reports read them
through `crates/core-api/src/routes/reports.rs`, and the shape is documented in
`docs/reference/read-model.md`.

**Gap.** A pack cannot add a view to core. Its own read model lives in its own
store and is refreshed from the API or the event stream.

## OTel

The contract is the interface: metric names, kinds, units, the complete attribute
set, stability per metric, the log field allowlist, and both processes' health
surfaces, all in one versioned table (`crates/core-otel/src/contract.rs`) with
one published artifact per revision under `crates/core-otel/contract/`. A rename
fails `committed_contract_matches_the_descriptor_table`, and the only exit is a
new version.

Per-tenant volume is available two ways, and they answer different questions:
`marula.org.requests` counts authenticated API traffic by status class, and
`marula.org.ingest_requests` counts authenticated machine ingest by source. A
connector's own POSTs land in the second series; nothing else in core sees them.

**Gap.** There is no scrape or query endpoint. Series leave by OTLP; a pack reads
them from the metrics store, under whatever retention that store has. The core
publishes the names and refuses to be the dashboard.

## Audit

`audit_events` is append-only and org-scoped by row-level security. The read is
`GET /api/audit-events`, and its export form is the resumable one:
`order=asc` with `after=<cursor>` walks forward on `(occurred_at, id)`, so a row
written while a page was in flight always lands in a later page and no row is
delivered twice. The cursor is opaque and versioned.

A pack reaches that read with `observability:read` (plugin capability
`observability.read`), which is a reader grant: no write, no delete, no widening
past the org. The human grant is `special:view_audit_log`, which a plugin may
never hold.

**Refused.** No second audit store. A pack that needs its own trail keeps its own
log; it does not add a column here.

## Usage

`org_usage` is a materialized read row per org, refreshed from the
`ops.v1_org_usage` view so the definition lives in one place. The API exposes it
at `GET /api/orgs/{org_id}/usage` and `GET /api/orgs/{org_id}/quotas`; the
capability is `org_usage.read`, the permission `org_usage:read`, held by machine
principals and custom roles and by no shipped preset.

Configured limits are ordinary org settings under the `usage_quota.` prefix.
Nothing in core reads them to refuse a request, and that is deliberate: a report
never gates.

**Gap, stated in the payload itself.** The usage endpoint reports
`api_calls_last_24h` as unavailable with the reason beside it, because a metric
series cannot be read back into a recorded-row answer. Chart the counters from
the metrics store.

## Change discipline

This document is checked, not trusted. `crates/core-otel/tests/extension_point_matrix.rs`
reads it and fails when a cited core file stops existing or a cited metric name
leaves the contract. Adding an extension point means adding a row here with a
citation the test can see; removing one means deleting both in the same change.
