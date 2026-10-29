# ADR 0008 — AI availability matrix, usage caps ledger, `ModelCredential` slot, and the agent chat runner

**Status:** Accepted · 2026-09-16
**Scope:** `CompletionProvider` edge, BYOK store, agent harness
(`docs/10-agent-platform.md`), settings registry, and the surface-to-model
policy that every AI-assisted surface reads

## Context

The shipped AI edge is thin by design: sealed BYOK keys (one org key per
provider), embeddings only (the chat-completions transport is explicitly "not
wired"), a flat `(provider, model)` settings pair, and no usage accounting.
The agent harness has tools (MCP reads + IF8 proposals), plus skills,
sessions, provenance, and no model runner. The product goal: pieces of AI appear in
*many* places (general chat, ticket windows, document drafting, ticket note
editing, email drafting, plus paid-tier surfaces), and the org admin decides
who gets what, where, at what model, within what limits. Both users and
admins must be able to see progress against caps.

## Decision

### 1. The availability matrix is a core data model, not settings sprawl

- **Surfaces are a registry, not an enum.** Native v1 generation surfaces:
  `general_chat`, `ticket_note_assist`, `doc_drafting`, `email_drafting`;
  internal: `embeddings_search`. **Paid-tier/plugin surfaces**
  (`data_analysis`, `automation_writing`, `dashboard_generation`,
  `transcription`) are registered through the same surface registry, so the
  commercial editions and community plugins can attach them without core
  schema changes. The core does not build those features. It must support
  plugging them in exactly as designed (the vault-ingress precedent).
- **Model catalog is admin-managed** (`ai_model_catalog`: provider, model id,
  label, capabilities, cost-weight class). Users pick from the allowed set
  **per surface**, and the pick sticks via the typed settings registry user
  scope. Policy precedence: user pick, then org default, always within the
  admin-allowlisted set.
- **Grants:** availability = `(surface, grantee role/user)` rows in the same
  deny-by-default, audit-logged discipline as the capability registry.
  Admins grant per role or per user, per surface.
- **Caps:** an append-only `usage_ledger` keyed `(org, user, surface,
  provider, model, tokens)` plus `ai_caps` (org-wide budget, per-user
  budget/token caps, where hard caps bound the call and soft caps warn and
  notify).
  Progress is **queryable read models** (per-user and per-org progress): the
  "who did this is a query" doctrine applied to spend. Enforcement sits at
  the completion edge *before* dispatch, and it never 5xxs a human workflow.
  It degrades per the same degrade-never-fail posture as search (`ai.rs`).

### 2. Credentials: user AND org, plus the `ModelCredential` slot

- v1 ships **both** credential kinds: org keys (the existing BYOK path) and
  per-user personal keys. Same AES sealing, same never-returned posture. A
  user's personal key serves only their calls, and only in surfaces the
  admin has granted them. An admin policy flag decides whether users may add
  personal credentials at all.
- A new **`ModelCredential` adapter slot** lands in core: sealed credential
  plus a provider-kind descriptor, surfaced exactly like a BYOK secret. The
  descriptor names one of three shapes: api-key, oauth-token, and
  subscription-account.
  **Where a provider OFFERS official OAuth access, the core ships the OAuth
  connect flow** (generic OAuth 2.0: redirect, token exchange, sealed token
  storage, refresh/rotation), so users sign in with e.g. ChatGPT/Codex or
  Copilot accounts. **Providers without such an official path (e.g.
  Anthropic) stay API-key-only.** The catalog marks which kinds each
  provider supports, and the UI simply renders what the provider permits.
  **Subscription bridges that ride these official OAuth paths are plug-in
  providers against this slot** (commercial editions first, community
  later). The core stores and gates whatever passes the sealed credential
  seam. It meters every use, refreshes the sealed material, and audits the
  activity trail. It never implements
  provider-specific account-bridging hacks or TOS-gray scraping. If a
  provider has no sanctioned path, that provider gets no bridge.

### 3. The chat runner is the agent-harness's model loop (one, consumed by all)

- One `ChatRunner` in core: resolves the caller, surface, allowed
  models, and the wardrobe of tools (the same MCP-catalog-derived read
  tools, via the capability gate), then runs the tool loop. Agent-authored
  writes stay in the IF8 proposal pipeline (approval default, involvement
  dial). Streaming reaches clients (SSE/WS), and every model call and tool
  call lands in the usage ledger plus the provenance audit.
- In-editor surfaces (ticket note assist, doc drafting, email drafting)
  consume the SAME runner scoped to their surface's grant set. Proposal and
  insert flows ride the involvement dial, never direct writes past the
  human.
- The wardens from `docs/10-agent-platform.md` apply unchanged:
  deny-by-default capabilities, audited invocations, and no untrusted
  content into tools unscoped.

### 4. The commercial edition is the consumer, not the center

- Core owns the matrix schema, the ledger, the caps, the credential slot, the
  runner, and the surface registry.
- The commercial edition consumes all of it: the managed LiteLLM router
  becomes just another provider visible in the catalog. Task-class budgets
  ride the same ledger schema. Member-subscription bridges (Claude/Copilot
  team plans) ship commercial-side as sealed `ModelCredential` providers.
  **The commercial edition can also offer models directly through its
  subscription (small markup) using the exact same slot**, with no special
  core privileges and no parallel authz.

## Consequences

- The matrix, catalog, ledger, caps, and the credential slot land as migrations
  0081/0082, head-ordered after the plugin/skill registry migration (0079)
  per ADR 0009 and the workbar migration (0080).
- The existing flat settings (`ai.completion_provider`, `ai.completion_model`)
  remain as an overridable default. The matrix supersedes them without a
  breaking cut (a U6-safe additive migration). The AI settings surfaces stay
  typed "not available yet" until the matrix work closes.
- Premium surfaces (data analysis, automation writing, dashboard generation,
  transcription AI) stay plugin-shaped: registered surfaces, no core
  implementations.
- Re-evaluation triggers: the commercial entitlement model landing, and the
  first member-subscription bridge proving the credential slot.
