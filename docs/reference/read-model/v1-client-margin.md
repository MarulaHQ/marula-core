# `ops.v1_client_margin`

> Contract level **v1** · introduced 0188
> Binding rules live on the [read-model index](../read-model.md).

One row per client per money event: the revenue an invoice or credit note
books, and the cost of the time the client consumed. Its date column is
`period_day`, so a window over it becomes one row per client per period — that
rollup is the `client_margin` report.

This view is the only read model with **no personal dimension**. Margin belongs
to a client, not to a staff member, so RBAC self-scoping cannot apply to it.
Access is a permission: the report route requires `invoicing:view`, and the
report builder refuses stored specs on this view.

## Columns (in contract order)

| # | Column | Type | Nullable | Notes |
|---|---|---|---|---|
| 1 | `org_id` | uuid | no | owning org (RLS seam) |
| 2 | `client_id` | uuid | no | client the money belongs to |
| 3 | `client_name` | text | no | denormalized client name |
| 4 | `period_day` | date | no | event date: invoice issue date, credit-note creation date in UTC, or the entry's UTC work day |
| 5 | `revenue_minor` | bigint | no | base-currency revenue. Negative for a credit note |
| 6 | `cost_minor` | bigint | yes | costed labour for the event. NULL when no rate resolved |
| 7 | `costed_minutes` | integer | no | minutes priced into `cost_minor` |
| 8 | `uncosted_minutes` | integer | no | minutes with no rate. Never priced at zero |
| 9 | `margin_minor` | bigint | yes | `revenue_minor − COALESCE(cost_minor, 0)`. NULL exactly when `cost_minor` is NULL |
| 10 | `unconverted_credit_minor` | bigint | no | credit notes in a non-base currency with no issuing invoice to borrow a rate from |

## What counts

- **Revenue**: invoices with status `issued` or `paid`, dated by `issued_at`.
  `draft` is not revenue yet and `void` never was. The amount is
  `COALESCE(base_total_minor, total_minor)`: a document issued in the org base
  currency carries no conversion snapshot and its own amount is the base
  amount (0065).
- **Credit notes**: status `issued`, dated by `created_at`, subtracted. A
  non-base credit converts through its issuing invoice's locked rate
  (`fx_half_even_div`, 0067). If there is no invoice to borrow a rate from the
  amount lands in `unconverted_credit_minor` instead of being dropped.
- **Time**: every entry attributed to a client, billable or not. Cost is
  incurred by the work, not by its billing classification or by an approval
  having been recorded. Entries with no client (internal work, or project time
  on a project without a client) cannot be attributed and are absent. The report's totals are the sum of
  its rows, so the two never disagree.
- **Cost**: `minutes × hourly rate`, rounded once per entry half up —
  `(minutes × rate + 30) / 60`, the same rule core-billing prices labour with.
  Rounding per entry is why a client's total is the sum of the same integer
  cents the org total is.

## Cost rates

Two columns hold the rate, both nullable, both in org base-currency minor units
per hour:

- `users.cost_rate_minor` — the person's rate.
- `orgs.default_cost_rate_minor` — the org fallback.

A NULL user rate falls back to the org default. When both are NULL the entry's
minutes are `uncosted_minutes`, `cost_minor` is NULL and `margin_minor` is NULL.
Unknown cost is never read as zero: a client whose work cannot be costed has no
margin to report, and the report says so.

## Example

Revenue, cost and margin for one month, with uncosted work surfaced:

```sql
SELECT client_name,
       SUM(revenue_minor)   AS revenue_minor,
       SUM(cost_minor)      AS cost_minor,
       SUM(uncosted_minutes) AS uncosted_minutes,
       CASE WHEN SUM(uncosted_minutes) = 0
            THEN SUM(revenue_minor) - SUM(cost_minor) END AS margin_minor
FROM ops.v1_client_margin
WHERE org_id = $1
  AND period_day >= DATE '2026-06-01'
  AND period_day <  DATE '2026-07-01'
GROUP BY client_id, client_name
ORDER BY margin_minor DESC NULLS LAST;
```

A client with cost and no invoice appears with a negative `margin_minor`. That
is the point of the view: starting from the invoice list would hide it.
