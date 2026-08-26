# Meridian Agent Platform (companion spec)

> Cited as binding by `docs/prompts/BUILD-PROMPT.md` §4.9 and the Agent-platform
> overlay in §7. This file is the expanded normative reference.

Meridian doubles as an **agent harness**: the same domain that serves humans,
exposed to agents through capability-scoped, audited, consented surfaces.

## 1. Surfaces vs. extensions

REST, WebSocket, and MCP are **surfaces** projecting ONE contract. Plugins and
HTTP connectors are the actual **extensions** consuming it.

- The MCP server (`crates/mcp-server`) is a separately deployable adapter whose
  tool catalog derives mechanically from OpenAPI + the capability registry at
  build time. Zero hand-written tools, zero business logic.
- Tool writes ride Stage 4–5; reads land with the MCP crate in Stage 3.

## 2. Typed settings registry (Tier 1 — lands Stages 1–2)

Every user-visible setting is declared once in a typed registry
(`key, kind, default, scope: org|user, visibility, validation`). UI renders
settings pages FROM the registry; code reads settings THROUGH it. No ad-hoc
env/config reads for behavior a user can change. Ships with Meridian's own
settings hub (org settings + user preferences pages).

## 3. Three-tier onboarding

1. **Org wizard** (Stages 1–2): first-run flow — org profile, business-hours
   calendar, default board + statuses, admin invite. Idempotent, skippable;
   org works untouched if skipped entirely (defaults-first).
2. **Module activation flows** (Stage 3–4): per-module guided setup (SLA
   policies, portal, invoicing, KB…), each documenting its "none" mode.
3. **Guided playbooks** (Stage 4–5): day-to-day recipes (triage queue, EOD time
   hygiene, dispatch morning review) surfaced in-app and exposed as MCP tools.

## 4. Agent skills & layered rules

- `AgentSkill`: versioned declarative manifest (id, version, capabilities,
  prompts/rules). Stored, diffable, rollback-able.
- Rules evaluate in layers before EVERY tool call:
  platform → org → role/skill → session consent. Deny wins; evaluation is
  logged.
- Untrusted/retrieved content never renders raw and never reaches tools
  without scoping (§4.8 of BUILD-PROMPT applies to agents doubly).

## 5. Provenance & audit

Append-only audit records actor provenance for humans AND agents alike:
`{actor_kind: human|agent|mcp|plugin, actor_id, skill_version, session_id,
tool, scopes_used, decision}`. Audit is queryable in the admin UI.

## 6. Documents

Documents keep immutable version history (`DocumentVersion`: append-only,
diffable, restorable). Restores create NEW versions; history is never rewritten.

## 7. Safety invariants (binding)

1. Capability scopes are deny-by-default; consent is explicit per install/session.
2. Every invocation is audit-logged (no silent calls).
3. The widget registry decides what may render; props are JSON-Schema validated.
4. Rate/resource limits enforced host-side, not by convention.
