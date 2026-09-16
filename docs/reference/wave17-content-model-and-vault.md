# Wave 17 technical brief — KB content model (ADR 0007) + native vault (ADR 0006)

> **Status:** Decisions pre-bound so lane agents implement, not re-litigate · 2026-09-16
> **Sources:** [ADR 0006](../adr/0006-native-secrets-vault-and-request-portal-plugin-boundary.md) ·
> [ADR 0007](../adr/0007-tiptap-editor-framework.md) ·
> [TASKLIST.md](../../TASKLIST.md) Wave 17 · ADR 0004 · docs/instant-first.md · docs/upgrade-compatibility-policy.md
> **Lanes:** `p17-kb-tiptap-model` (0078 / port 54378) ∥ `p17-vault-trees-org-rbac`
> (0079 / port 54379) → `p17-kb-vault-closeout` (UI, no migration)

## Hard boundaries (binding, all lanes)

- Two backend lanes are **schema-disjoint**: lane 1 touches `kb_articles`-family
  columns; lane 2 adds new tables/rows only. A lane may read the other's tables but
  never writes them. Reuse over re-invention: the migration conventions block
  (lane-exclusive HEAD pair, in-transaction transforms per policy §4.5,
  `map_unique_constraint` entries for every new unique constraint), the cross-entity
  `RecordLink` seam, `crypto::seal`.
- OpenAPI pin pair (`packages/sdk/openapi.json` = `fixtures/openapi/v1/openapi.json`,
  COMPACT minified) regenerated per DX12 before each lane closes.
- Secrets material NEVER reaches: stack traces, error envelopes (`error.rs`), OTEL
  attributes, SDK response objects (metadata-only lists + audited reveal posture —
  the shipped `/passwords` contract does not change).

## Lane 1 — content model contract (ADR 0007)

- **Canonical storage:** `kb_articles.body`/`content` migrates to Tiptap JSON in
  `content_jsonb` (jsonb, GIN-indexed jsonb_path_ops where useful for FTS preflight).
  The storage format is a versioned envelope: `{"schema_ver": 1, "doc": <tiptap-doc>}`
  so future v2 renders can coexist (U6 preflight-friendly: additive migration holds
  existing `v1` render path).
- **Server-side validation (pure, in `crates/core-kb`):** validate against the
  pinned published Tiptap JSON schema version (pin the schema file in-repo at
  adoption commit so CI diff-reviews upgrades); max depth + byte bounds reuse
  `MAX_BODY_BYTES` semantics; sanitizer pass over any HTML produced for rendering —
  no script tags, `javascript:` URLs, or unsanctioned iframes ever serialise.
- **Embeds/links:** every embed node carries `provider_id` + normalized URL validated
  through the ADR 0004 allowlist registry (`core-kb` `rich.rs` — no new code path).
- **Read-only render:** server renders stored JSON → HTML/React without the editor
  package in the bundle; per-tenant theme tokens apply; draft visibility rules ride
  the existing KB FTS/indexer contract (`p15-docs-closeout` finding applies).
- **Editor shell (web):** `apps/web/lib/editor/` (wiring) + `packages/ui/src/editor/`
  (primitives), both scaffolded with purpose/non-purpose READMEs — read before work.
- **Autosave:** every editor transaction → debounced idempotent mutation on the
  IF1–IF3 envelope (idempotency key + `version`/`If-Match`); replays byte-identical;
  undo history lands in the existing autosave read model (IF7/IF8) — no sidecar
  invented in the editor.

## Lane 2 — native vault contract (ADR 0006)

- **Category trees:** `secret_folders(id, org_id NULLABLE, client_id NULLABLE,
  parent_id, slug, label, kind, is_default_seed, created_at, updated_at)` — exactly
  one of org/client scope non-null (CHECK), unique `(parent, slug)`. Seeded defaults
  (passwords / api-keys / certificates / recovery-codes / documents) are immutable
  seed rows; customization = FORK (new row graph), never edit-in-place (ADR 0004
  template doctrine); user-added folders are ordinary rows.
- **Secret records:** generalize beside `PasswordEntry` — new `secret_records`
  (client-nullable/org-nullable scope like folders; `folder_id`; `kind`:
  password/api-key/certificate/recovery-code/document; metadata columns) +
  `secret_fields` where every material field rides `crypto::seal` AES-256-GCM (AAD
  `(org_id, record_id, field_key)`), reveal-only via the audited endpoint — parity
  with the shipped `/passwords/{id}/reveal` posture. `PasswordEntry` remains for the
  existing UI/responses; migration maps any client-vault entries forward via the
  deterministic importer (`imports/` lane pattern), never a destructive cutover.
- **Record links:** reuse the cross-entity `RecordLink` seam (asset related-items
  pattern from Wave 13 lane 5): secret ↔ ConfigItem/app/KB-article/ticket with
  typed link labels; broken targets surf as broken like the asset-docs semantic.
- **Org-level + category RBAC:** new module permission `secrets:read`/`secrets:edit`
  + `secret_category_grants` (role × folder); central policy module gates every
  route; **preset roles updated as data** (Technician: org-process KB articles yes,
  restricted categories no — mirror the `financial:read` gate pattern). Portals never
  gain vault read except where lane-free explicitly.
- **Noisy-neighbor floor (platform property):** per-org write budget guard on vault
  mutations (token bucket in the worker path, admission control at the API layer)
  so bulk imports (worker job, 100–500-key chunks, resumable checkpoints) can never
  starve interactive p95. Files ride object storage as ciphertext with pointer+hash
  rows — never material blobs in `text` columns.

## Lane 3 — close-out contract

- Surfaces: `/kb` (editor + renderer live), KB → client → secret-tree navigation,
  org/client scope toggle, category-ACL branches strictly hidden (typed
  "not available" — never dead buttons), secret↔record link panels, role-gated
  reveal preserved. Vision-review evidence + REVIEW verdict under `design/qa-kb-vault/`.
- Wave 16's in-flight `/kb` and `/passwords` lanes transfer editor/section findings
  here; do not duplicate wire-ups.

## Definition-of-done (every lane)

SQLx migration(s) on the reserved pair + `map_unique_constraint` entries; SQLx tests
against real Postgres incl. the RLS-coverage assertion for every new/changed table;
RLS coverage guard extended; typed SDK regen + byte-identical pin pair; audit events
emitted for every mutation; clippy -deny-warnings + fmt clean; full-suite gate green
on the lane branch (precondition: Wave 15 lane 2 cleared the known failures).
