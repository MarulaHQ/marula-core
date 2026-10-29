# ADR 0019 — Tenancy boundary: multi-tenant foundation, single-tenant surface

**Status:** Accepted · 2026-09-16
**Renumbered 2026-10-01.** This ADR was written as
`0008-tenancy-boundary.md`, where it collided with
`0008-ai-availability-matrix-and-agent-runner.md`. The AI ADR is the
heavily-cited 0008, named by migration 0081, `ai_policy.rs`,
`chat_runner.rs`, and ADRs 0010 and 0011. It kept its number, because
migrations are append-only historical records and rewriting their citations
is not a repair. This one moved to the next free slot, **0019**. No decision
content changed, only the identifier.
**Scope:** data model, RLS doctrine, API surface, deployment posture of the
open core, and its relationship to commercially operated deployments of the
core

## Context

The core is architected around an `org` boundary from the very first
migration. Every user-data table carries `org_id` with an isolation policy,
the application connects as a non-owner Postgres role, and each transaction
binds `app.org_id` with default-deny when unset
(`crates/core-db/migrations/0001_orgs_auth.sql`). This org scaffolding is
deliberate and permanent. It is what makes export, RBAC, settings scoping,
webhooks and realtime auth org-scoped and safe.

However, exposure is a different question from capability. The core is
positioned as a self-hosted, single-organization product. Marula alone must
run a full MSP operation, and a multi-tenant SaaS control plane is a named
non-goal. Commercial operations are a layer above the core. Two failure
modes exist, and both must be avoided:

1. **Removing the org scaffolding** because the surface is single-tenant.
   That would destroy RLS discipline, export correctness, and the extension
   seam.
2. **Growing the core toward a SaaS control plane** (org signup,
   provisioning, billing, cross-org administration). That converts the open
   core into the thin free tier of a hosted product, which the product
   strategy explicitly rejects.

Separately, licensing reality must be stated honestly. The core is
AGPL-3.0, which permits third parties to operate it as a hosted service
provided they meet the license's source-sharing obligations. The core does
not, cannot, and should not attempt to technically or legally "prohibit
native multi-tenanting" by others. What the strategy does instead is
structural, and that is the binding decision here.

## Decision

- **Capability and surface are decoupled.** The core *supports* multi-tenant
  operation at the data layer (`org_id` everywhere, RLS, per-request org
  binding). It does not *expose* tenancy as a product surface: no org
  signup or self-provisioning, no cross-org administration UI, no usage
  metering, no billing, no control plane. The shipped product posture is
  one organization per deployment.
- **The scaffolding is doctrine, not dead code.** The multi-tenant data
  layer (org scoping, RLS isolation, org-bound credentials/tokens) stays
  permanent core architecture, covered by the same
  two-approver review as all authz/RLS changes. Single-tenant deployments
  exercise them unchanged. Hosted operations of the core, by us or anyone,
  build on them through public, versioned extension points only, never
  through new code paths inside the core.
- **Multi-tenant operation as a commercial product is out of core scope.**
  Tenant lifecycle, provisioning, entitlement, metering, and the SaaS
  control plane are commercial-layer components. They operate the core via
  public APIs and the plugin PDK, and "one deployment (logical) per tenant"
  is the reference shape, which keeps per-tenant upgrades, data residency,
  and isolation structural rather than procedural. These components live in
  the commercial planning repo, and their design constraint is recorded
  there; this repo carries no knowledge of their internals.
- **No licensing anti-circumvention.** The core does not attempt to detect,
  prevent, or degrade third-party multi-tenant hosting. AGPL-3.0 permits it
  under its source obligations, and the core already forbids gating on
  commercial entitlements. The competitive boundary is structural: nothing
  in the core assists tenancy operations. Assertions of exclusive rights to
  operate the core as a hosted service live in trademark and commercial
  relationships, not in this codebase.
- **Isolation triple-check stays mechanical.** Any surfaced feature that
  could widen the org boundary (shared tables, cross-org search, global
  entities) requires an RFC, and the RLS-in-CI harness is the enforcement
  mechanism for the default-deny posture.

## Consequences

- Export/backup, settings, audit and event envelopes keep their `org_id`
  predicates ("belt-and-suspenders" per `crates/core-db/src/export.rs`). The
  cost is negligible, and it is what keeps a future hosted layer safe by
  construction.
- Contribution review: PRs introducing provisioning, signup, metering, or
  cross-org surfaces to the core are rejected by rule, not by reviewer
  taste.
- The naming overlap between the outbox relay worker
  (`core_events::run_relay`, worker :8082) and any future product name is a
  documentation concern only; no code rename is required by this ADR.
- If the strategy ever changes, meaning the core grows a hosted tier, this
  ADR is superseded by a new ADR plus an RFC, never amended silently.
