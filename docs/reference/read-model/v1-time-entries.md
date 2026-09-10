# `ops.v1_time_entries`

> Contract level **v1** · introduced 0026
> Binding rules live on the [read-model index](../read-model.md).

One row per logged work entry, with the ticket/client/user joins carried for
grouped reporting.

## Columns (in contract order)

| # | Column | Type | Nullable | Notes |
|---|---|---|---|---|
| 1 | `id` | uuid | no | entry id (PK of this view) |
| 2 | `org_id` | uuid | no | owning org (RLS seam) |
| 3 | `ticket_id` | uuid | no | charged ticket |
| 4 | `ticket_number` | text | no | denormalized ticket number |
| 5 | `client_id` | uuid | yes | client inherited from the ticket |
| 6 | `client_name` | text | yes | denormalized client name |
| 7 | `user_id` | uuid | no | authoring staff member |
| 8 | `user_display_name` | text | yes | staff display name |
| 9 | `started_at` | timestamptz | no | work interval start |
| 10 | `ended_at` | timestamptz | no | work interval end |
| 11 | `minutes` | integer | no | billed minutes (≥ 0) |
| 12 | `billable` | boolean | no | non-billable flag drives billable math in reports |
| 13 | `work_type` | text | yes | nullable classification |
| 14 | `day_utc` | date | no | calendar day of `started_at` in UTC (v1 grouping key) |
| 15 | `timer_session_id` | uuid | yes | nullable origin session |
| 16 | `created_at` | timestamptz | no | |

## Example

```sql
SELECT user_display_name, day_utc,
       SUM(minutes) AS logged_minutes,
       SUM(CASE WHEN billable THEN minutes ELSE 0 END) AS billable_minutes
FROM ops.v1_time_entries
WHERE org_id = $1
GROUP BY user_display_name, day_utc
ORDER BY day_utc DESC;
```
