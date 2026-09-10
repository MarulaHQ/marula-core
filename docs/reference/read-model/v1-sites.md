# `ops.v1_sites`

> Contract level **v1** · introduced 0059
> Binding rules live on the [read-model index](../read-model.md).

One row per client site with the client name denormalized.

## Columns (in contract order)

| # | Column | Type | Nullable | Notes |
|---|---|---|---|---|
| 1 | `id` | uuid | no | site id (PK of this view) |
| 2 | `org_id` | uuid | no | owning org (RLS seam) |
| 3 | `client_id` | uuid | no | owning client |
| 4 | `client_name` | text | no | denormalized client name |
| 5 | `name` | text | no | site name |
| 6 | `address` | text | yes | street address |
| 7 | `city` | text | yes | |
| 8 | `region` | text | yes | |
| 9 | `postal_code` | text | yes | |
| 10 | `country` | text | yes | |
| 11 | `latitude` | double precision | yes | |
| 12 | `longitude` | double precision | yes | |
| 13 | `timezone` | text | yes | IANA tz for local-day grouping |
| 14 | `created_at` | timestamptz | no | |

## Example

```sql
SELECT s.client_name, s.name, s.city, s.country
FROM ops.v1_sites s
WHERE s.org_id = $1
ORDER BY s.client_name, s.name;
```