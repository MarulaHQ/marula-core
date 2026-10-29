# `ops.v1_first_response`

> Contract level **v1** · introduced 0026
> Binding rules live on the [read-model index](../read-model.md).

One row per ticket: creation → earliest **staff** comment. Portal replies are
structurally excluded (portal identities are not staff rows: migration 0021
separated them, so their comments always carry `author_id` NULL). Any
staff-authored note counts as
"we reacted".

## Columns (in contract order)

| # | Column | Type | Nullable | Notes |
|---|---|---|---|---|
| 1 | `ticket_id` | uuid | no | ticket id (PK of this view) |
| 2 | `org_id` | uuid | no | owning org (RLS seam) |
| 3 | `ticket_number` | text | no | denormalized ticket number |
| 4 | `client_id` | uuid | yes | nullable client |
| 5 | `assignee_id` | uuid | yes | nullable staff assignee (self-scoping dimension) |
| 6 | `created_at` | timestamptz | no | ticket creation instant |
| 7 | `first_staff_comment_at` | timestamptz | yes | earliest staff-authored comment; NULL while unanswered |
| 8 | `first_response_secs` | bigint | yes | latency seconds; NULL while unanswered (negative values are clock-skew artifacts, surfaced as-is) |
| 9 | `first_response_due` | timestamptz | yes | current SLA response target |

## Example

```sql
SELECT AVG(first_response_secs)::numeric(10,1) AS avg_first_response_secs
FROM ops.v1_first_response
WHERE org_id = $1
  AND created_at >= now() - interval '7 days'
  AND first_response_secs IS NOT NULL;
```