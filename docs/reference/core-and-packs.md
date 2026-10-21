# Core and premium packs

> **Audience:** whoever is deciding whether a feature belongs in the core or in a
> pack, and whoever is building the pack. This page describes the boundary. It
> says nothing about how a deployment obtains a pack.

The core is the open product: the API, the worker, the schema, the auth model, the
plugin host, the event contract. A premium pack is an installed artifact that builds
on the seams the core publishes. [Extension points](extension-points.md) is the map of
those seams, written from the pack's side and tested against the code. This page answers
the other question: what stays on each side of the line, and why the line holds.

One rule sits above everything. The core carries no knowledge of packs at all. There is
no entitlement check, no plan column, no `if premium` branch in any crate. A pack is a
pack because it is installed, and the core asks only what a principal may do
(`crates/core-domain/src/policy.rs`, [extension points](extension-points.md)).

## What the core ships

Every surface a pack can use is in the open repository, at full strength. Nothing below
is a stub waiting for a pack to complete it:

| Surface | Where it lives | What a pack does with it |
|---|---|---|
| Permissions and policies | `crates/core-domain/src/policy.rs` | Resolves every grant into the one `Permission` vocabulary. A pack's capability is a subset of a permission the core already understands |
| Plugin host | `crates/plugin-host/src/lib.rs`, `surface.rs` | Authorises an invocation and derives the declarative surfaces an install contributes |
| Ingest | `crates/core-api/src/routes/webhook_receiver.rs` | Accepts HMAC-signed machine traffic from a connector without a JWT |
| Federation | `crates/core-api/src/auth/federation.rs` | Registers an identity source from an installed artifact and routes it through the same token machinery as a password login |
| Events | `crates/core-events/src/contract.rs` | Publishes a declared event type, and refuses a type outside the registry |
| Read model | `ops.v1_*` views, `crates/core-db/migrations/0026_reports_read_model.sql` | Reads operational numbers through `security_invoker` views, so row-level security applies as if the caller had queried the base tables |
| Telemetry contract | `crates/core-otel/src/contract.rs` | Reads the published metric names, units and log fields from one versioned table |
| Audit | `crates/core-api/src/routes/audit.rs` | Walks the trail with a stable cursor, forward only, org-scoped |
| Usage | `crates/core-api/src/routes/org_usage.rs` | Reads per-org counts and configured quota settings |

## What a pack adds

A pack adds volume, a vendor binding, or a workflow that only makes sense for one
industry. It does not add capability to the core's own surfaces, and it cannot widen one:

| Point | A pack adds | The core keeps |
|---|---|---|
| Plugin | Declarative surfaces, declared event types, consented capabilities | Execution, the grant decision, the audit of both |
| Connector | An HTTP adapter to one vendor, and its egress declaration | The ingest endpoint, the signature check, the dedupe fence |
| Federation bundle | One more identity source | The group semantics, the link decision, the session |
| Auth provider | Verified assertions | The MFA gate, the token, the conservative link |
| Webhook and event | Subscriptions its org configures | The signed delivery ledger and the registry of legal types |
| Read model | Its own store, refreshed from the API or the stream | The `ops` views and the numbers in them |
| OTel | A consumer of the published series | The names, units and attributes those series carry |
| Audit | Its own log, if it needs one | The trail, which stays append-only and org-scoped |
| Usage | Its own ingest counting | The materialized usage row |

The pattern in the third column is the point. A pack reads what the core records, and
the core never reads the pack to decide something. That direction is what keeps an
install's behaviour explainable from the open code alone.

## Three things a pack may never do

Each is refused by construction rather than by review, which is the only kind of
refusal that survives a busy week:

1. **Hold an administrative special.** No capability maps onto one, and
   `admin_specials_should_never_be_grantable` fails the build if that ever changes
   ([extension points](extension-points.md)).
2. **Import a core crate.** The connectors under `plugins/` carry no path dependency into
   `crates/`, so a pack talks to the public API and nothing else.
3. **Write the audit trail.** A pack holds `observability:read` at most. The human grant
   for the trail, `special:view_audit_log`, is not reachable from a plugin capability.

## Gaps, and whose problem they are

A boundary that claims completeness would be lying, so the open gaps are named here and
owned somewhere:

- The core does not execute plugin code: `PluginRuntime` is a trait with no sandbox
  behind it ([plugin runtime](plugin-runtime.md)). A pack that needs per-tenant metering
  records it on its own side today.
- An out-of-tree federation bundle has no HTTP read path to the group surface. An
  in-process bundle reads the functions directly. A deployed sync service would need a
  read route and a scope, which is a row on a plan, not a patch.
- A pack cannot add a view to the core, and the core publishes no metrics scrape
  endpoint. Series leave by OTLP, and a pack reads them from whatever store receives them.

None of those is a missing feature in the core. Each is a decision about where work
happens, and each decision has a cost the pack pays rather than the operator.

## How the line is checked

This page describes a rule. Most of the rule is enforced by something that fails a
build:

| Check | What it fails on |
|---|---|
| `crates/core-otel/tests/extension_point_matrix.rs` | A cited core file that no longer exists, or a metric name that left the contract |
| `admin_specials_should_never_be_grantable` | A capability that would reach an administrative special |
| `scripts/check-reliability.mjs` | A second retry budget. It is listed here because the policy a pack inherits is the core's, and a pack does not carry one of its own |

One part of the boundary is convention rather than a check. The connectors under
`plugins/` carry no path dependency into `crates/` today, and nothing fails a build when
one adds it. That is a gap in the enforcement rather than in the rule, and it is worth
closing before the convention is tested by someone in a hurry.

## Where to go next

Start with [extension points](extension-points.md) for the nine seams and the core file
that owns each. For the runtime a plugin runs in, read
[plugin runtime](plugin-runtime.md). For the shape of the events a pack subscribes to,
the contract is `crates/core-events/src/contract.rs`. For what an automated agent may
call, the surface is described in [the agent set](../agents/README.md).
