# ADR 0007 — Tiptap as the rich-text editor framework for knowledgebase content

**Status:** Accepted (user directive) · 2026-09-16
**Scope:** every rich-text authoring surface that starts as knowledgebase content —
KB articles now; ticket rich-notes, custom-form rich fields, portal-facing docs later

## Context

The knowledge base ships v1 with structured article content. As it grows toward
allowlisted embeds, first-class linked documents, image/code/table blocks, and
concurrent collaboration (`docs/02` §7), we need a versatile editor decision that
holds across the whole platform: verifiably compatible with the Rust-backed,
API-first backend; extensible by the open-source community (the plugin doctrine in
`docs/02` §5); and headless so every editing surface renders in Workbench/Tangerine
tokens with zero visual drift.

Candidates evaluated: **Tiptap 3** (headless layer over ProseMirror; MIT core and
most extensions; paid surface is complete feature bundles, not components) and
**Lexical 0.x** (Meta's React-first editor; fully MIT, smaller extension corpus).

## Decision

- **Tiptap 3 is the editor framework.** Schema-driven document model; the canonical
  stored representation for KB article content is **Tiptap JSON** (a platform-neutral
  tree, validated against the published Tiptap JSON schema) in a `content_jsonb`
  column. The Rust backend stores, migrates, and validates content as data; it never
  needs editor internals to serve search indexes, RAG corpora, or read-only rendering.
- **Headless only.** No Tiptap-bundled UI/CSS ships in `apps/web`; toolbars, slash
  menus, and editors' shells are built from our component primitives so light/dark
  parity, `≤120ms` motion law, and WCAG baselines apply unchanged (Wave-16 lane-doctrine).
- **MIT-only extension policy (binding).** The editor and allowed extensions come
  only from the MIT-licensed core/extension set (the June-2025 OSS batch included).
  No Tiptap paid bundle (cloud collaboration, comments, conversion, AI templates)
  ever enters the tree — the same boundary discipline as the C3/B11 vault decision:
  OSS features stay whole in the core, closed product surfaces buy their own stack.
- **ProseMirror is the escape hatch.** `@tiptap/pm` re-exports ProseMirror 1:1, so
  deep editor plugins and a worst-case fork path remain open-source and extractable —
  the MIT core survives any future Tiptap GmbH licensing shift that would threaten us.
- **Collaboration, if/when adopted, is self-hosted Hocuspocus (MIT) on the existing
  Postgres/Redis infra** behind the same instant-first envelope — managed Tiptap
  Cloud is rejected exactly like managed Ably/Pusher was (`docs/05-stack-decisions`
  §6): no closed SaaS in the open core's critical path.
- **Community extensions are the normal path**, not an exception: editor work
  prefers composing/reaching for OSS Tiptap extensions; only where no extension
  exists do we write one first, and Marula-specific extensions (KB embeds against
  the allowlist registry per ADR 0004, RelatedAssetLink records, resolution-note
  taxonomy pickers) are regular Tiptap extensions published from `plugins/`.
- Server-side content validation lives in `crates/core-kb`: schema-check on write,
  sanitizer pass over any HTML rendered from stored JSON, link/embed attributes
  validated against the ADR 0004 allowlist registry.

## Consequences

- **Scope boundary (binding, clarified 2026-09-16): Tiptap goes where the storage
  contract is Tiptap JSON — nowhere else.** KB articles (Wave 16 lane 2) are in.
  Everything the note-taxonomy/workbar composer pattern governs is **out**: the
  workbench-v2 floating composer (`WbComposer`), the WorkBar notes/tasks workbar,
  timesheet/checklist textarea inputs, portal replies stay **plain textarea** — they
  submit text lines under the one-composer rule (`docs/notes-tasks-workbar.md`),
  ride per-line taxonomy/provenance metadata keys, and would break the per-field
  autosave/undo if their storage silently became a rich document tree. If a
  durable-document note need ever emerges, that is a NEW content type (a link
  target, like ADR 0006), never a silent storage upgrade of the composer.
  Email drafts remain plain text + typed variables; AI-assist for all these
  surfaces rides the Wave 18 runner over the text content, not over a rich tree.

- KB article storage migrates to the Tiptap-JSON canonical model; the instant-first
  autosave contract (`docs/instant-first.md` §7.2) is met unchanged (debounced
  idempotent writes already exist; the editor emits granular transactions that map to
  them).
- Lexical rejected for now: plugin authorship reaches into Meta internals with a
  thin ecosystem, collaboration is "build it yourself", and — decisive here — it
  would leave the Rust API boundary without a strict server-side document model.
- Bundle budget holds: Tiptap packages are modular; `@tiptap/pm` rides ProseMirror's
  proven tree-diff updates so large-article p95 stays inside the interaction budget.
- Re-evaluation triggers: Tiptap changing the MIT core license; Tiptap 5/6 schema
  break; or Lexical reaching production collaboration maturity *without* Yjs caveats
  plus a community extension corpus comparable to Tiptap's.
