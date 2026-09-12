# `ops.v1_fx_reconciliation`

> Contract level **v1** · introduced 0067
> Binding rules live on the [read-model index](../read-model.md).

One row per cross-currency FACT recorded at one of the five FX lock moments
(migration 0065, ADR 0002): expenses, cost lines, invoice issue, labor
pricing lines, inventory receipts, and payments. Single-currency facts
(currency = org base) carry no snapshot and produce no row — their original
amount IS the base amount, and they are already covered by the other
`ops.v1_*` views.

Reports convert NEVER — they read the immutable snapshot columns. Realized /
unrealized gain-loss stays in the accounting system; this view states the
boundary, it does not revalue.

## Additivity contract (read before summing)

The union is **NOT additive across `kind`**. One cross-currency invoice
appears as an `invoice_issue` row (document total), one `labor_pricing` row
per priced line (lines sum to ≈ the total), and one `payment` row per
settlement — the same money seen through three lock moments, by design
("original amount, rate, base equivalent" per fact).

**Never sum across `kind`.** Aggregate per `kind`, or per invoice through
`invoice_id`, or the books double-count.

## Columns (in contract order)

| # | Column | Type | Nullable | Notes |
|---|---|---|---|---|
| 1 | `org_id` | uuid | no | owning org (RLS seam) |
| 2 | `kind` | text | no | `expense` · `cost_line` · `invoice_issue` · `labor_pricing` · `inventory` · `payment` |
| 3 | `fact_id` | uuid | no | fact row id (PK of this view, per kind) |
| 4 | `ticket_id` | uuid | yes | owning ticket (expense / cost_line rows) |
| 5 | `invoice_id` | uuid | yes | owning invoice (invoice_issue = its own id; labor_pricing, payment) |
| 6 | `original_currency` | text | no | ISO 4217 fact currency |
| 7 | `original_amount_minor` | bigint | no | fact amount in minor units (cost lines recomputed with the same half-even integer rule the write path locked — never a float product) |
| 8 | `fx_rate_used` | numeric(24,12) | no | immutable rate, fact→base major units |
| 9 | `fx_rate_date` | date | no | rate's effective date at lock time |
| 10 | `fx_source` | text | no | rate source at lock time (`manual` in v1) |
| 11 | `base_amount_minor` | bigint | no | immutable base-currency equivalent |
| 12 | `sync_target` | text | no | books sync target; `'none'` until an accounting export target is configured per deployment |
| 13 | `fact_at` | timestamptz | no | when the fact was recorded (expense incurred_at, payment received_at, document created_at) |

## Example

```sql
-- Realized conversions per kind in a window (never cross-kind sums):
SELECT kind, count(*) AS facts, sum(base_amount_minor) AS base_minor
FROM ops.v1_fx_reconciliation
WHERE fact_at >= '2026-09-01'
GROUP BY kind
ORDER BY kind;
```

## Query-supporting indexes

Each underlying fact table carries a partial index on the snapshot-bearing
subset (0067): `(org_id, <fact timestamp> DESC) WHERE fx_rate_used IS NOT
NULL` — the RLS org predicate, the snapshot filter, and the `fact_at DESC`
top-N ordering all resolve through one index probe per branch.
