# core-reports

Reporting engine types, field catalog, and query compiler for the versioned
`ops.v1_*` read-model layer (the private build tracker R0/R1, ADR 0001).

## Binding rules

* **Reports NEVER read physical tables.** Every statement emitted by
  [`builder::compile`] references ONLY `ops.v1_*` views (migration
  `0026_reports_read_model.sql`). View evolution is the single controlled
  seam. Renaming a physical column cannot break a saved report
  (upgrade-compatibility policy §5).
* **Identifiers are static catalog constants.** Column/filter/aggregate
  references flow exclusively from `catalog.rs`. Lookups fail closed on any
  unknown name, which makes identifier-interpolation injection structurally
  impossible.
* **Values bind as `$N` parameters.** Nothing user-supplied is ever
  interpolated into statement text.
* **Tenancy flows through RLS.** All views are `WITH (security_invoker =
  true)`. The API layer additionally binds `app.org_id` per request inside
  an explicit transaction (`core_api::routes::reports::run_rows`) using the
  dedicated `marula_app`-role pool.

## v1 report kinds

| slug | view | shape |
|---|---|---|
| `time_by_user` | `ops.v1_time_entries` | grouped rollup |
| `time_by_client` | `ops.v1_time_entries` | grouped rollup (missing clients coalesce onto the nil uuid) |
| `time_by_ticket` | `ops.v1_time_entries` | grouped rollup |
| `sla_attainment` | `ops.v1_sla_outcomes` | grouped by policy over resolved-in-window outcomes |
| `sla_breaches` | `ops.v1_sla_outcomes` | tabular, window anchored on ticket creation |
| `ticket_throughput` | `ops.v1_tickets` | per-day created/resolved spine |
| `ticket_aging` | `ops.v1_tickets` | fixed open-age buckets |
| `first_response` | `ops.v1_first_response` | tabular (portal comments structurally excluded) |

## The card contract (Wave 46 lane 1)

A card is a `ReportSpec` (what to read) plus a `CardSpec` (how it is drawn).
Both surfaces embed those two types, so a dashboard card and a report card
cannot drift:

| shape | viz | needs | default size |
|---|---|---|---|
| `chart` | `bar`, `line`, `pie` | rows, and at least one group-by dimension | `half` / `regular` |
| `stat` | `stat` | one scalar from the run summary, exactly one measure, no dimension | `third` / `compact` |
| `table` | `table` (or a chart viz with no dimension) | headers, and rows | `full` / `tall` |

`CardShape::of` derives the shape from the spec instead of trusting a separate
field. `materialise` turns a run into `CardData` and refuses with
`ReportsError::EmptyCard { shape, failure }` when the shape cannot be filled —
no rows for a chart or a table, no headers for a table, no scalar for a stat.
`CardSpec::validate` refuses a window wider than one refresh reads, and a card
that refreshes less often than the window it displays. `shape_catalogue()`
carries the prose an authoring agent reads, and `GET /api/report-catalog`
serves it.

## Non-goals (this crate)

No execution (core-api owns it), no report-spec storage (R3), no dashboard
surface or card placement (R2 and the dashboard surface lane — this crate owns
the card vocabulary, not where a card sits), no scheduling (R4), no local-day
time grouping yet (v1 groups UTC days — documented in the view header), no
OLAP/marts (ADR 0001 scale note).

## None-mode operator recipe: external BI over the views

Zero Marula code required — Metabase/Superset/Grafana/PowerBI connect
directly to Postgres through a read-only login role. The ready-to-adapt SQL
lives in the migration header comments. The essence:

```sql
CREATE ROLE marula_reports_ro LOGIN PASSWORD '<secret>'
  NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION;
GRANT USAGE ON SCHEMA ops TO marula_reports_ro;
GRANT SELECT ON ALL TABLES IN SCHEMA ops TO marula_reports_ro;
ALTER DEFAULT PRIVILEGES IN SCHEMA ops
  GRANT SELECT ON TABLES TO marula_reports_ro;

-- RLS-through-views: with NO org binding this role sees ZERO rows
-- (default deny). Bind deliberately, one org per role:
ALTER ROLE marula_reports_ro SET app.org_id = '<org uuid>';
```

In-app RBAC does not extend into external tools (ADR 0001): granting this
role is the conscious act.
