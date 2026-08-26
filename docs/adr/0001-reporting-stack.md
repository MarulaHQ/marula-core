# ADR 0001 — Reporting stack: thin in-app builder over read-model views

**Status:** Accepted · 2026-08-27
**Scope:** Reporting engine track (TASKLIST R0–R6)

## Context

End users need custom report building (pick a view → filters → group-by →
visualization, saved and shareable). Options considered:

1. Embed a full BI application (Metabase / Superset / Grafana).
2. Adopt a headless semantic layer (Cube.js).
3. Build a thin in-app builder on top of the versioned read-model views
   (R0) using commodity React chart libraries.

Constraints from the adopted policy and doctrine:

- Report definitions must be **customizations-as-data** living in OUR store,
  so the upgrade preflight (U5/U6) can flag deprecated-construct usage
  (upgrade-compatibility-policy §5). External stores would be invisible to it.
- Upgrade-compatibility §2 defines exactly four stable boundaries; embedding
  a fifth (a BI app's own surface) is contract creep.
- Single-writer §4 and standalone-first deployment favor not adding required
  sidecar runtimes.
- End-user query builders' classic risk is SQL injection through identifier
  interpolation.

## Decision

Build option 3, with the following composition:

- **Visualization (frontend):** Recharts (MIT) for bar/line/pie/table.
  Reevaluate only if a concrete need exceeds it; ECharts (Apache-2.0) is the
  fallback. Dashboard drag-arrangement (R2) may add react-grid-layout (MIT)
  when dashboards land.
- **Report specification (R3):** typed report-spec rows in our DB validated
  by serde enums (view id from a fixed allowlist, filter operands, group-by
  columns, aggregate + viz hint). This keeps specs inside U5 manifest scope.
- **Query execution (backend):** a Rust compiler mapping validated specs to
  parameterized SQL against the `ops.v1_*` read-model views ONLY. Identifiers
  originate exclusively from the field catalog (enum-matched, never string
  interpolated); user-supplied values bind as parameters. No path accepts raw
  SQL, ever.
- **External BI compatibility (escape hatch):** the documented read-model
  views double as the supported interface for Metabase/Superset/Grafana/
  PowerBI connecting directly to Postgres via a read-only reporting role.
  Zero Meridian code required; documented operator recipe ships with R0.
  In-app RBAC does NOT extend into these tools — operators grant views-level
  access consciously.

## Consequences

- No new runtime services required (standalone-first holds); all new
  dependencies are frontend, permissive-licensed, tree-shakeable.
- Saved reports are migration-safe by construction: they reference views +
  catalog ids, both covered by the deprecation clock (U4) and preflight (U6).
- Scale limits acknowledged: the builder targets aggregate/tabular needs,
  not OLAP exploration; heavy analytics intentionally routed to external BI
  over the views. Revisit pre-aggregation/marts only under measured load.
- Recharts bundle-size cost accepted; charts render lazily on report pages.
