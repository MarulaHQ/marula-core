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
| [connector-specification.md](connector-specification.md) | The bar a connector meets (R1–R13), the reconciled standard set, what proves each requirement, and the conformance debt that is still open |
| [connector-capabilities.md](connector-capabilities.md) | What a connector can reach: the five behaviours, the product kinds and record types, the evidence each deep link needs, and the form to request a new one |
| [connector-surface.md](connector-surface.md) | The six staff-UI surfaces a connector appears on, drawn once, so a marketplace connector feels as native as a core one |
| [core-and-packs.md](core-and-packs.md) | What the open core ships, what a pack adds, and the checks that hold the boundary |
| [reliability-matrix.md](reliability-matrix.md) | One row per subsystem: the guarantee, the limit, the failure mode, and where an operator sees it |
| [reliability-levers.md](reliability-levers.md) | Every descriptor lever with its default, and the retry budget each preset resolves to. Generated from the descriptor and checked against it |
| [connection-pooling.md](connection-pooling.md) | PgBouncer in transaction mode, the connection ceiling, and the measurements behind both |
| [data-lifecycle.md](data-lifecycle.md) | Retention per table and profile, object-store integrity, the erasure contract |
| [rpo-rto.md](rpo-rto.md) | The recovery numbers each profile delivers, and the drill that measures them |
