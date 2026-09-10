# `ops.v1_clients`

> Contract level **v1** · introduced 0059
> Binding rules live on the [read-model index](../read-model.md).

One row per client company (the top of the client → site → contact
hierarchy).

## Columns (in contract order)

| # | Column | Type | Nullable | Notes |
|---|---|---|---|---|
| 1 | `id` | uuid | no | client id (PK of this view) |
| 2 | `org_id` | uuid | no | owning org (RLS seam) |
| 3 | `name` | text | no | client name |
| 4 | `notes` | text | yes | free-text notes |
| 5 | `created_at` | timestamptz | no | |
| 6 | `updated_at` | timestamptz | no | |

## Contract notes

`clients.restricted_to` (per-user RBAC restriction plumbing) is deliberately
not exposed; exposing it later is an additive change.

## Example

```sql
SELECT c.name, COUNT(t.id) AS open_tickets
FROM ops.v1_clients c
LEFT JOIN ops.v1_tickets t ON t.client_id = c.id AND t.category <> 'closed'
WHERE c.org_id = $1
GROUP BY c.name
ORDER BY open_tickets DESC;
```