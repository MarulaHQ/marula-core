# Read model — versioned SQL views (`ops.v1_*`)

> **Status:** Authoritative contract · policy §2 ([docs/upgrade-compatibility-policy.md](../upgrade-compatibility-policy.md)) · ADR 0001
> **Audience:** external BI tools, report authors, direct-SQL readers, extension authors

The documented read model is the **supported interface for direct SQL access and
external reporting tools** (Metabase/Superset/Grafana/PowerBI, `psql`, MCP data
seams). It exists because "your database" is a promise — but the promise
attaches to these **views, not to the physical tables**:

- **Reports and external tools read `ops.v1_*` only.** Every column referenced
  here is stable for the life of `v1`; physical tables are explicitly
  **not stable, ever** (policy §2, §9).
- **Stored reports survive upgrades** because they reference view slugs, not
  physical columns (report definitions store `kind` = `ops.v1_*` slug — 0034).

## Stability & deprecation contract

- Views are **versioned by prefix**: `v1` today. The column list of a version
  is its API contract.
- **Additive changes are safe**: new columns append at the END of the column
  list, in ordinary (non-breaking) migrations. Never rely on column ORDER —
  select by name.
- **Breaking changes never edit a version in place**: a rename, removal, or
  type change ships a NEW namespace (`ops.v2_*`) alongside, announced with a
  `removal_target` (policy §3); `v1` keeps emitting until its window elapses.
  The U6 upgrade preflight flags view changes before an upgrade.
- Each view's SQL header comment (in its introducing migration) carries the
  same contract note. The core-db read-model smoke tests assert the column
  contracts mechanically — a PR that drifts a contract fails CI.

## Security model — RLS through views

Every view is created `WITH (security_invoker = true)` (PG15+, verified on
pgvector/pgvector:pg17). The view executes with the **caller's** privileges,
so every underlying table's row-level-security policy applies exactly as if
the caller had queried the base table:

- An application-role session (`meridian_app`) bound via
  `set_config('app.org_id', …)` sees its own org's rows and nothing else.
- **No binding ⇒ zero rows** (default-deny, like every RLS policy).
- Every view retains an explicit `org_id` column as the tenancy seam.
- The table OWNER bypasses RLS (views add no gate of their own); the core-db
  smoke test (`tests/ops_read_model.rs`) proves owner vs app-role scoping.

### Operator recipe (external BI, zero Meridian code — ADR 0001)

```sql
CREATE ROLE meridian_reports_ro LOGIN PASSWORD '<secret>'
  NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION;
GRANT USAGE ON SCHEMA ops TO meridian_reports_ro;
GRANT SELECT ON ALL TABLES IN SCHEMA ops TO meridian_reports_ro;
-- RLS-through-views: without an app.org_id binding the role sees ZERO rows.
-- Bind deliberately, one org per role:
ALTER ROLE meridian_reports_ro SET app.org_id = '<org uuid>';
```

In-app RBAC does **not** extend into external tools: granting a SQL login is
the conscious act. Review per scope before sharing credentials.

## v1 catalog

| View | One row per | Introduced |
|---|---|---|
| [`ops.v1_tickets`](read-model/v1-tickets.md) | ticket, workflow + SLA + classification truth joined in | 0026 (taxonomy columns 0059) |
| [`ops.v1_time_entries`](read-model/v1-time-entries.md) | logged work entry | 0026 |
| [`ops.v1_sla_outcomes`](read-model/v1-sla-outcomes.md) | ticket with SLA state | 0026 |
| [`ops.v1_gap_days`](read-model/v1-gap-days.md) | per-user missing-time audit day | 0026 |
| [`ops.v1_first_response`](read-model/v1-first-response.md) | ticket, first staff response | 0026 |
| [`ops.v1_record_undo_stack`](read-model/v1-undo-stacks.md) | applied write on a record (undo stack) | 0041 |
| [`ops.v1_field_undo_stack`](read-model/v1-undo-stacks.md) | per-field applied-state history | 0041 |
| [`ops.v1_clients`](read-model/v1-clients.md) | client company | 0059 |
| [`ops.v1_sites`](read-model/v1-sites.md) | client site | 0059 |
| [`ops.v1_contacts`](read-model/v1-contacts.md) | client contact | 0059 |
| [`ops.v1_board_statuses`](read-model/v1-board-statuses.md) | board status (workflow customization) | 0059 |
| [`ops.v1_invoices`](read-model/v1-invoices.md) | invoice document | 0059 |
| [`ops.v1_invoice_lines`](read-model/v1-invoice-lines.md) | invoice line | 0059 |
| [`ops.v1_fx_reconciliation`](read-model/v1-fx-reconciliation.md) | cross-currency fact with its immutable conversion snapshot (ADR 0002, five lock moments; NOT additive across `kind`) | 0067 |

## Related

- Policy: [upgrade-compatibility-policy.md §2](../upgrade-compatibility-policy.md) (read model), §4.4 (compat views), §9 (honest limits)
- ADR 0001 — reporting stack (thin builder over the field catalog + external BI)
- Report definitions (0034) store `kind` = view slug, so saved specs ride this contract
- Smoke tests: `crates/core-db/tests/ops_read_model.rs` (existence, security_invoker, queryability, RLS scoping, column contracts, taxonomy resolution)
