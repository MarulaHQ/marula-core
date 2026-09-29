# apps/web/lib/editor/ — Tiptap editor shell (ADR 0007, Wave 16 lane 2)

> **Status:** populated (Wave 16 lane 2, 2026-09-16). Extension set + /kb
> editor wiring landed on the IF1–IF3 autosave envelope.

**Purpose:** the ONE home for Marula's rich-text editor shell built on
**Tiptap 3** (`@tiptap/core`, `@tiptap/pm`, `@tiptap/react`):

* the composed extension set (`extensions/index.ts`: StarterKit 3 + Image,
  TaskList/TaskItem, YouTube + the Marula OSS nodes — MIT-only policy, no
  Tiptap paid bundles, ever; ADR 0007 / `packages/sdk`-pinned schema mirrors
  `crates/core-kb/schemas/`);
* the `/kb` article editor wiring: debounced, idempotent autosave against the
  IF1–IF3 envelope (`apps/web/lib/envelope-write.ts` — byte-identical replays);
* Marula-authored OSS Tiptap extensions where none exists:
  allowlisted KB-embeds (`crates/core-kb/src/rich.rs` provider registry, ADR 0004),
  RelatedAssetLink `RecordLink` records (per ADR 0006), note-taxonomy pickers;
* the **read-only renderer** that re-renders stored Tiptap JSON server-side (RSC)
  with the editor never in the read bundle.

**Non-purpose:** no business/editor policy that Rust must enforce identically
(that lives in `crates/core-kb` — schema validation, sanitizer, allowlist); no
vault/category surfaces (ADR 0006 browsing rides Wave 16 lane 6 close-out); no
MCP/agent writer loops (agent-authored prose rides `docs/10-agent-platform.md`
approval binding).

**Contract notes for the lane:**
* Canonical storage is **Tiptap JSON** in `content_jsonb`; the editor emits
  transactions; the store path is plain autosave JSON — no HTML round-trips through
  the API.
- Schema control = extension allowlist, server-side validated against the
  published Tiptap JSON schema version pinned in `core-kb`.
* Theme: every toolbar/menu element is our primitives (Workbench/Tangerine tokens;
  packages/ui editor primitives; light+dark parity; `≤120ms`, transform/opacity,
  `prefers-reduced-motion`).
* Secrets/knowledgebase surfaces stay **out** of this module — they are lane 2/3.
