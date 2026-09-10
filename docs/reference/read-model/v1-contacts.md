# `ops.v1_contacts`

> Contract level **v1** · introduced 0059
> Binding rules live on the [read-model index](../read-model.md).

One row per client contact with client/site names denormalized.

## Columns (in contract order)

| # | Column | Type | Nullable | Notes |
|---|---|---|---|---|
| 1 | `id` | uuid | no | contact id (PK of this view) |
| 2 | `org_id` | uuid | no | owning org (RLS seam) |
| 3 | `client_id` | uuid | no | owning client |
| 4 | `client_name` | text | no | denormalized client name |
| 5 | `site_id` | uuid | yes | nullable site |
| 6 | `site_name` | text | yes | denormalized site name |
| 7 | `name` | text | no | contact name |
| 8 | `email` | text | yes | |
| 9 | `phone` | text | yes | |
| 10 | `title` | text | yes | job title |
| 11 | `is_primary` | boolean | no | primary contact flag |
| 12 | `created_at` | timestamptz | no | |

## Example

```sql
SELECT co.client_name, co.name, co.email, co.phone
FROM ops.v1_contacts co
WHERE co.org_id = $1 AND co.is_primary
ORDER BY co.client_name;
```