# ADR 0008 — Tenancy boundary: multi-tenant foundation, single-tenant surface

**Status:** Accepted (user directive) · 2026-09-16
**Scope:** data model, RLS doctrine, API surface, deployment posture of the open
core; relationship to commercially operated deployments of the core

## Context

The core is architected around an `org` boundary from the very first migration:
every user-data table carries `org_id` with an isolation policy, the application
connects as a non-owner Postgres role, and each transaction binds `app.org_id`
with default-deny when unset (`crates/core-db/migrations/0001_orgs_auth.sql`).
This org scaffolding is deliberate and permanent — it is what makes export,
RBAC, settings scoping, webhooks, and realtime auth org-scoped and safe.

But exposure is a different question from capability. The core is positioned as
a self-hosted, single-organization product
(`docs/STRATEGY.md` §1–§2, trust product: "Meridian alone must run a full MSP
operation"), while "multi-tenant SaaS control plane" is a named non-goal
(§4) and commercial operations are a layer above the core (§7). Two failure
modes exist and must both be avoided:

1. **Removing the org scaffolding** because the surface is single-tenant — this
   would destroy RLS discipline, export correctness, and the extension seam.
2. **Growing the core toward a SaaS control plane** (org signup, provisioning,
   billing, cross-org administration) — this converts the open core into the
   thin free tier of a hosted product, which our strategy explicitly rejects.

Separately, licensing reality must be stated honestly: the core is AGPL-3.0,
which permits third parties to operate it as a hosted service provided they
meet the license's source-sharing obligations. The core does not, cannot, and
should not attempt to technically or legally "prohibit native multi-tenanting"
by others. What the strategy does instead is structural, and is the binding
decision here.

## Decision

- **Capability and surface are decoupled.** The core *supports* multi-tenant
  operation at the data layer (org_id everywhere, RLS, per-request org binding)
  but does not *expose* tenancy as a product surface: no org signup or
  self-provisioning, no cross-org administration UI, no usage metering,
  no billing, no control plane. The shipped product posture is one
  organization per deployment.
- **The scaffolding is doctrine, not dead code.** Multi-tenant data-layer
  structures (org scoping, RLS isolation, org-bound credentials/tokens) are
  maintained as permanent core architecture and covered by the same
  two-approver review as all authz/RLS changes (`docs/STRATEGY.md` §5).
  Single-tenant deployments exercise them unchanged; hosted operations of the
  core — by us or anyone — build on them through public, versioned extension
  points only (§7), never through new code paths inside the core.
- **Multi-tenant operation as a commercial product is out of core scope.**
  Tenant lifecycle, provisioning, entitlement, metering, and the SaaS control
  plane are commercial-layer components (§4 non-goals; §7). They operate the
  core via public APIs and the plugin PDK; "one deployment (logical) per
  tenant" is the reference shape, keeping per-tenant upgrades, data residency,
  and isolation structural rather than procedural. These components live in
  the commercial planning repo, and their design constraint is recorded there;
  this repo carries no knowledge of their internals.
- **No licensing anti-circumvention.** The core does not attempt to detect,
  prevent, or degrade third-party multi-tenant hosting (AGPL-3.0 permits it
  under its source obligations; §7 already forbids the core gating on
  commercial entitlements). The competitive boundary is structural: nothing in
  the core assists tenancy operations. Assertions of exclusive rights to
  operate the core as a hosted service live in trademark and commercial
  relationships, not in this codebase.
- **Isolation triple-check stays mechanical.** Any surfaced feature that could
  widen the org boundary (shared tables, cross-org search, global entities)
  requires an RFC; the RLS-in-CI harness is the enforcement mechanism for the
  default-deny posture.

## Consequences

- Export/backup, settings, audit, and event envelopes keep their `org_id`
  predicates ("belt-and-suspenders" per `crates/core-db/src/export.rs`) — cost
  is negligible, and it is what keeps a future hosted layer safe by
  construction.
- Contribution review: PRs introducing provisioning, signup, metering, or
  cross-org surfaces to the core are rejected by rule, not by reviewer taste.
- The naming overlap between the outbox relay worker
  (`core_events::run_relay`, worker :8082) and any future product name is a
  documentation concern only; no code rename is required by this ADR.
- If the strategy ever changes (core growing a hosted tier), this ADR is
  superseded by a new ADR plus an RFC — never amended silently.
