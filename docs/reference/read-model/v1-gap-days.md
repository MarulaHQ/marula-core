# `ops.v1_gap_days`

> Contract level **v1** · introduced 0026
> Binding rules live on the [read-model index](../read-model.md).

One row per (user, local day) in the missing-time audit ledger (0017) — the
missing-time audit input.

## Columns (in contract order)

| # | Column | Type | Nullable | Notes |
|---|---|---|---|---|
| 1 | `id` | uuid | no | audit ledger id (PK of this view) |
| 2 | `org_id` | uuid | no | owning org (RLS seam) |
| 3 | `user_id` | uuid | no | audited staff member |
| 4 | `user_display_name` | text | yes | denormalized name |
| 5 | `user_timezone` | text | yes | the tz the day was computed in |
| 6 | `day` | date | no | LOCAL calendar day (per-user tz semantics) |
| 7 | `activity` | boolean | no | had ticket activity that day |
| 8 | `logged_minutes` | integer | no | minutes actually logged |
| 9 | `expected_minutes` | integer | no | org threshold at audit time |
| 10 | `has_gap` | boolean | no | computed gap verdict |
| 11 | `nudge_sent_at` | timestamptz | yes | nullable EOD nudge delivery |
| 12 | `updated_at` | timestamptz | no | last ledger touch |

## Example

```sql
SELECT user_display_name, COUNT(*) AS gap_days, SUM(expected_minutes - logged_minutes) AS missing_minutes
FROM ops.v1_gap_days
WHERE org_id = $1 AND has_gap
GROUP BY user_display_name
ORDER BY missing_minutes DESC;
```