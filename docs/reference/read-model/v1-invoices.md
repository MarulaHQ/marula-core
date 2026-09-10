# `ops.v1_invoices`

> Contract level **v1** · introduced 0059
> Binding rules live on the [read-model index](../read-model.md).

One row per invoice document (0036 semantics: money is integer minor units +
ISO 4217 currency copied from the contract — no conversion; two state
machines as data).

## Columns (in contract order)

| # | Column | Type | Nullable | Notes |
|---|---|---|---|---|
| 1 | `id` | uuid | no | invoice id (PK of this view) |
| 2 | `org_id` | uuid | no | owning org (RLS seam) |
| 3 | `run_id` | uuid | yes | origin invoice run (NULL for manual invoices) |
| 4 | `run_number` | text | yes | denormalized run number |
| 5 | `client_id` | uuid | no | billed client |
| 6 | `client_name` | text | no | denormalized client name |
| 7 | `contract_id` | uuid | no | deriving contract |
| 8 | `number` | text | no | human invoice number, org-unique |
| 9 | `status` | text | no | `draft`\|`issued`\|`paid`\|`void` |
| 10 | `currency` | text | no | ISO 4217 (all amounts use it) |
| 11 | `period_from` | date | yes | billed period start (NULL for fixed-fee schedules) |
| 12 | `period_to` | date | yes | billed period end |
| 13 | `issued_at` | date | no | issue date |
| 14 | `due_on` | date | no | due date |
| 15 | `total_minor` | bigint | no | sum of line amounts, minor units |
| 16 | `total_paid_minor` | bigint | no | recorded payment total |
| 17 | `balance_minor` | bigint | no | computed: `total_minor − total_paid_minor` |
| 18 | `dunning_state` | text | no | `none`\|`reminder_sent`\|`overdue`\|`final_notice`\|`collections` |
| 19 | `dunning_updated_at` | timestamptz | yes | last dunning transition |
| 20 | `created_by` | uuid | yes | nullable authoring staff member |
| 21 | `created_at` | timestamptz | no | |
| 22 | `updated_at` | timestamptz | no | |

## Contract notes

- Money is integer minor units; never floats.
- `balance_minor` is derived from two contract columns (not stored); it can
  never disagree with them.

## Example

```sql
SELECT number, client_name, status, dunning_state,
       total_minor, total_paid_minor, balance_minor, currency
FROM ops.v1_invoices
WHERE org_id = $1 AND status = 'issued'
ORDER BY due_on;
```