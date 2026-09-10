# `ops.v1_invoice_lines`

> Contract level **v1** · introduced 0059
> Binding rules live on the [read-model index](../read-model.md).

One row per persisted invoice line (`kind` mirrors core_billing
`BillingLineKind`, 0036). Client/currency are denormalized from the invoice
for grouped reporting without a second join.

## Columns (in contract order)

| # | Column | Type | Nullable | Notes |
|---|---|---|---|---|
| 1 | `id` | uuid | no | line id (PK of this view) |
| 2 | `org_id` | uuid | no | owning org (RLS seam) |
| 3 | `invoice_id` | uuid | no | owning invoice |
| 4 | `invoice_number` | text | no | denormalized invoice number |
| 5 | `client_id` | uuid | no | inherited from the invoice |
| 6 | `contract_id` | uuid | no | deriving contract |
| 7 | `kind` | text | no | `recurring_base`\|`retainer_base`\|`retainer_overage`\|`time_materials`\|`fixed_fee` |
| 8 | `description` | text | no | line description |
| 9 | `period_start` | date | no | billed period start |
| 10 | `period_end` | date | no | billed period end |
| 11 | `quantity_minutes` | bigint | no | labor minutes (0 for amount lines) |
| 12 | `unit_amount_minor` | bigint | no | unit amount, minor units |
| 13 | `amount_minor` | bigint | no | line amount, minor units |
| 14 | `currency` | text | no | inherited from the invoice |
| 15 | `position` | integer | no | display order within the invoice |

## Example

```sql
SELECT i.number, l.kind, SUM(l.amount_minor) AS amount_minor, i.currency
FROM ops.v1_invoice_lines l
JOIN ops.v1_invoices i ON i.id = l.invoice_id
WHERE l.org_id = $1 AND l.period_start >= DATE '2026-01-01'
GROUP BY i.number, i.currency, l.kind
ORDER BY i.number;
```