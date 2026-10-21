# reference/

Canonical-theme note: BUILD-PROMPT/mockups cite `docs/reference/marula-theme.md`;
the authoritative theme spec lives at **`docs/theme.md`** (single source).
This directory holds generated/canonical API + schema references as they exist:
OpenAPI snapshots remain under packages/sdk/openapi.json; read-model view docs
land here with R0 (`ops.v1_*` contract pages) — **the read-model contract is
authored: see [read-model.md](read-model.md) + `read-model/` per-view pages
(U3, migration 0059).**

Wave planning and decision records:

| Doc | What it is |
|---|---|
| [extension-points.md](extension-points.md) | Every seam a premium pack, connector, federation bundle or auth provider builds on: what it may do, what it may not, and the core file that owns it. Tested so it cannot drift. |
