# Marula Agent Platform (companion spec)

> The normative reference for the agent surfaces. [`instant-first.md`](instant-first.md)
> carries the interaction rules every agent surface must obey.

Marula doubles as an **agent harness**: the same domain that serves humans,
exposed to agents through capability-scoped, audited, consented surfaces.

## 1. Surfaces vs. extensions

REST, the SSE live stream, and MCP are **surfaces** projecting ONE contract. Plugins and
HTTP connectors are the actual **extensions** consuming it.

- The MCP server (`crates/mcp-server`) is a separately deployable adapter whose
  tool catalog derives mechanically from OpenAPI + the capability registry at
  build time. Zero hand-written tools, zero business logic.
- Tool writes ride Stage 4–5. Reads land with the MCP crate in Stage 3.

## 2. Typed settings registry (Tier 1 — lands Stages 1–2)

Every user-visible setting is declared once in a typed registry
(`key, kind, default, scope: org|user, visibility, validation`). UI renders
settings pages FROM the registry. Code reads settings THROUGH it. No ad-hoc
env/config reads for behavior a user can change. Ships with Marula's own
settings hub (org settings + user preferences pages).

## 3. Three-tier onboarding

1. **Org wizard** (Stages 1–2): first-run flow — org profile, business-hours
   calendar, default board + statuses, admin invite. Idempotent, skippable. Org works untouched if skipped entirely (defaults-first).
2. **Module activation flows** (Stage 3–4): per-module guided setup (SLA
   policies, portal, invoicing, KB…), each documenting its "none" mode.
3. **Guided playbooks** (Stage 4–5): day-to-day recipes (triage queue, EOD time
   hygiene, dispatch morning review) surfaced in-app and exposed as MCP tools.

## 4. Agent skills & layered rules

- `AgentSkill`: versioned declarative manifest (id, version, capabilities,
  prompts/rules). Stored, diffable, rollback-able.
- Rules evaluate in layers before EVERY tool call:
  platform → org → role/skill → session consent. Deny wins. Evaluation is
  logged.
- **Agent actions on a user's behalf default to approval.** Because writes are
  autosaved and instantly reconciled, an agent acting "on your behalf" must not
  silently rewrite a user's records: agent-originated changes default to
  *proposed, awaiting approval*, keyed to the involvement dial and customizable
  per action type, per entity, per field, and by client-visible or commit-sensitive
  classification. Autonomous mode is an explicit, per-capability grant, never a
  global switch. Client-visible or external commitments never bypass a human.
  See [`instant-first.md`](instant-first.md).
- Untrusted or retrieved content never renders raw, and never reaches a tool
  without scoping. That rule applies to agents twice over.

## 5. Provenance & audit

Append-only audit records actor provenance for humans AND agents alike:
`{actor_kind: human|agent|mcp|plugin, actor_id, skill_version, session_id,
tool, scopes_used, decision}`. Audit is queryable in the admin UI.

- **"Who did this" is a query, not a sift.** Undo, autosave and agent actions
  keep many small events flowing, so the audit is exposed as a queryable
  provenance read model: which record and field changed, what the before and
  after values were, who acted (human, agent, API key, undo), the
  session/skill and rule-set hash, and when — never a raw-log scan (see
  [`instant-first.md`](instant-first.md)).
- Undo of a change emits a normal audited "reverted field X to its value at T"
  event. The trail is always append-only and undo never rewrites it.

## 6. Documents

Documents keep immutable version history (`DocumentVersion`: append-only,
diffable, restorable). Restores create NEW versions. History is never rewritten.

## 7. Safety invariants (binding)

1. Capability scopes are deny-by-default. Consent is explicit per install/session.
2. Every invocation is audit-logged (no silent calls).
3. The widget registry decides what may render. Props are JSON-Schema validated.
4. Rate/resource limits enforced host-side, not by convention.
