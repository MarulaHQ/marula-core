# Pinned Tiptap JSON schema (ADR 0007, core-kb content validation)

- **Source:** `https://tiptap.dev/docs/assets/schemas/tiptap-json-schema.json`
  (the published JSON Schema for valid Tiptap JSON content, referenced from
  <https://tiptap.dev/docs/editor/core-concepts/introduction>).
- **Pinned at:** 2026-09-16 · sha256
  `a6bba39d778a6f81188f9d6c781cff7acbd4e92947da9deb05455285c4faa893`
- **File:** `tiptap-json-schema.json` (draft 2020-12; validates the `doc`
  member of the `{"schema_ver": 1, "doc": …}` envelope stored in
  `kb_articles.content_jsonb`).

## Upgrade rule (binding)

Upgrading past a pinned schema version is a REVIEWED diff in CI: the new file
replaces this one and the sha256 here is updated in the same commit, together
with any validator extension needed (Marula node allowlist in
`crates/core-kb/src/content.rs`). A dependent `tiptap-yaml/README`-style note is
not needed — this file is the diff-review artifact itself: any PR touching
`tiptap-json-schema.json` must state pulled-sha and the reason; the CI hex-scan
and migration gates never auto-accept schema churn.

## Validator relationship

`crates/core-kb/src/content.rs` implements these rules natively in Rust (no
schema-engine dependency): required `type`, `content` arrays of nodes, `marks`
shape, text-node/text-key exclusivity (`allOf` conditionals). The pinned file
remains the source of truth for shape UPGRADES: when Tiptap publishes a new
schema, the diff between versions defines which validator changes to make.
Node/MARK names are additionally constrained by the Marula extension
allowlist (schema control = extension allowlist) — the published schema is
structural, ours is structural + allowlist.
