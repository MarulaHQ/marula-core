# plugins/ — first-party WASM plugins (Apache-2.0)

Purpose: home for first-party Extism/WASM plugins built on `packages/plugin-kit`
(PDK). Plugins pin a PDK generation via `api_version` in their manifest
(docs/10-agent-platform.md §6) and call ONLY the public API.

Non-purpose: no third-party plugin code (marketplace registry v1 lands Stage 4),
no connector code (that lives in `connectors/`), no host runtime (see
`crates/plugin-host`).

Status: scaffolded, empty — populates with Phase 4 "plugin runtime v2 +
marketplace registry v1". Licensing for everything added here: Apache-2.0.
