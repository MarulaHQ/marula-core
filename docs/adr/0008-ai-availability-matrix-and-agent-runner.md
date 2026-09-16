# ADR 0008 — AI availability matrix, usage caps ledger, `ModelCredential` slot, and the agent chat runner

**Status:** Accepted (user directive + design session) · 2026-09-16
**Scope:** `CompletionProvider` edge, BYOK store, agent harness (`docs/10-agent-platform.md`),
settings registry, and the surface-to-model policy that every AI-assisted surface reads

## Context

The shipped AI edge is thin by design: sealed BYOK keys (one org key per provider),
embeddings only — the chat-completions transport is explicitly "not wired" — a flat
`(provider, model)` settings pair, and no usage accounting. The agent harness has
tools (MCP reads + IF8 proposals), skills, sessions, provenance — but no model
runner. The product goal (user directive): pieces of AI appear in *many* places
(general chat, ticket windows, document drafting, ticket note editing, email
drafting, plus Relay-premium surfaces), and the org admin decides who gets what,
where, at what model, within what limits — with both users and admins able to see
progress against caps.

## Decision

### 1. The availability matrix is a core data model, not settings sprawl

- **Surfaces are a registry, not an enum.** Native v1 generation surfaces:
  `general_chat`, `ticket_note_assist`, `doc_drafting`, `email_drafting`; internal:
  `embeddings_search`. **Premium/plugin surfaces** (`data_analysis`,
  `automation_writing`, `dashboard_generation`, `transcription`) are registered
  through the same surface registry so Relay (and community plugins) can attach them
  without core schema changes — the core does not build those features; it must
  support plugging them in exactly as designed (vault-ingress precedent).
- **Model catalog is admin-curated** (`ai_model_catalog`: provider, model id, label,
  capabilities, cost-weight class). Users pick from the allowed set **per surface**;
  the pick sticks via the typed settings registry user scope. Policy precedence:
  user pick → org default; always within the admin-allowlisted set.
- **Grants:** availability = `(surface, grantee role/user)` rows in the same
  deny-by-default, audit-logged discipline as the capability registry; admins grant
  per role or per user, per surface.
- **Caps:** an append-only `usage_ledger` keyed `(org, user, surface, provider,
  model, tokens)` + `ai_caps` (org-wide budget, per-user budget/token caps; hard =
  bound the call; soft = warn + notify); progress is **queryable read models**
  (per-user and per-org progress) — the "who did this is a query" doctrine applied to
  spend. Enforcement sits at the completion edge *before* dispatch; enforcement
  never 5xxs a human workflow — it degrades per the same degrade-never-fail posture
  as search (`ai.rs`).

### 2. Credentials: user AND org, plus the `ModelCredential` slot

- v1 ships **both**: org keys (existing BYOK path) and per-user personal keys —
  same AES sealing, same never-returned posture; a user's personal key serves only
  their calls and only in surfaces the admin has granted them; admin policy flag
  decides whether users may add personal credentials at all.
- New **`ModelCredential` adapter slot** in-core: sealed credential + provider-kind
  descriptor (api-key, oauth-token, subscription-account), surfaced exactly like a
  BYOK secret. **Where a provider OFFERS official OAuth access, the core ships the
  OAuth connect flow** (generic OAuth-2.0 flow: redirect, token exchange, sealed
  token storage, refresh/rotation) so users sign in with e.g. ChatGPT/Codex or
  Copilot accounts; **providers without such an official path (e.g. Anthropic) stay
  API-key-only** — the catalog marks which kinds each provider supports, and the UI
  simply renders what the provider permits. **Subscription bridges that ride these
  official OAuth paths are plug-in providers against this slot** (Relay first,
  community later) — the core stores, gates, meters, refreshes, and audits whatever
  passes the sealed credential seam, but never implements provider-specific
  account-bridging hacks or TOS-gray scraping; if a provider has no sanctioned
  path, that provider gets no bridge.

### 3. The chat runner is the agent-harness's model loop (one, consumed by all)

- One `ChatRunner` (core lane): resolves the caller → surface → allowed models →
  wardrobe of tools (the same MCP-catalog-derived read tools, via the capability
  gate) → tool-loop; agent-authored writes stay in the IF8 proposal pipeline
  (approval default, involvement dial); streaming to clients (SSE/WS); every model
  call and tool call lands in the usage ledger + provenance audit.
- In-editor surfaces (ticket note assist, doc drafting, email drafting) consume the
  SAME runner scoped to their surface's grant set — proposal/insert flows ride the
  involvement dial, never direct writes past the human.
- Wardens from `docs/10-agent-platform.md` apply unchanged: deny-by-default
  capabilities, audited invocations, no untrusted content into tools unscoped.

### 4. Relay is the consumer, not the center

- Core owns: matrix schema, ledger, caps, credential slot, runner, surface registry.
- Relay (REL-I1) consumes all of it: the managed LiteLLM router becomes just another
  provider visible in the catalog; task-class budgets (REL-S2 metering) ride the
  same ledger schema; member-subscription bridges (Claude/Copilot team plans) ship
  Relay-side as sealed `ModelCredential` providers; **Relay can also offer models
  directly through its subscription (small markup) using the exact same slot** — no
  special core privileges, no parallel authz (see relay-psa docs 03 §REL-I1).

## Consequences

- Lane scope (Wave 18, new): `p18-ai-policy-ledger` (matrix + catalog + ledger +
  caps + credential slot), `p18-agent-chat-runner` (runner + general chat surface),
  `p18-ai-closeout` (settings/ai + agent-platform pages, chat panel, per-surface
  pickers, progress views) — migration pairs 0081/0082 (head-ordered after Wave 16's plugin/skill lane 0079 per ADR 0009 and Wave 17's workbar 0080).
- Existing flat settings (`ai.completion_provider`, `ai.completion_model`) remain as
  an overridable default; the matrix supersedes them without a breaking cut (U6-safe
  additive migration).
- Wave 17's AI-adjacent UI lanes treat AI settings surfaces as typed
  "not available yet" until Wave 18 closes.
- Premium surfaces (data analysis, automation writing, dashboard generation,
  transcription AI) stay plugin-shaped: registered surfaces, no core implementations.
- Re-evaluation triggers: Relay entitlement model shapes (REL-S2) landing; first
  member-subscription bridge proving the credential slot.
