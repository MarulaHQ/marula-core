# `ops.v1_board_statuses`

> Contract level **v1** · introduced 0059
> Binding rules live on the [read-model index](../read-model.md).

One row per board status: the org's workflow customization surface. Names and
colors are custom; **`category` is the stable semantic primitive** behavior
binds to (§5) — renaming a status never changes SLA, billing, automation, or
reporting behavior.

## Columns (in contract order)

| # | Column | Type | Nullable | Notes |
|---|---|---|---|---|
| 1 | `id` | uuid | no | status id (PK of this view) |
| 2 | `org_id` | uuid | no | owning org (RLS seam) |
| 3 | `board_id` | uuid | no | owning board |
| 4 | `board_name` | text | no | denormalized board name |
| 5 | `name` | text | no | status name (customizable) |
| 6 | `category` | text | no | semantic category (`open`\|`in_progress`\|`waiting`\|`closed`) |
| 7 | `sort_order` | integer | no | display order within the board |
| 8 | `is_entry` | boolean | no | entry status for new tickets |
| 9 | `color` | text | yes | nullable status color |

## Example

```sql
SELECT board_name, name, category, sort_order, is_entry
FROM ops.v1_board_statuses
WHERE org_id = $1
ORDER BY board_name, sort_order;
```