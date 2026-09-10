# `ops.v1_sla_outcomes`

> Contract level **v1** · introduced 0026
> Binding rules live on the [read-model index](../read-model.md).

One row per ticket that carries SLA state: targets, actuals, and attainment
verdicts (joins carry no duplicates — PKs).

## Columns (in contract order)

| # | Column | Type | Nullable | Notes |
|---|---|---|---|---|
| 1 | `ticket_id` | uuid | no | FK to `ops.v1_tickets.id` |
| 2 | `org_id` | uuid | no | owning org (RLS seam) |
| 3 | `ticket_number` | text | no | denormalized ticket number |
| 4 | `priority` | text | no | ticket priority at outcome time |
| 5 | `assignee_id` | uuid | yes | nullable staff assignee (self-scoping dimension) |
| 6 | `ticket_created_at` | timestamptz | no | window filter anchor |
| 7 | `policy_id` | uuid | no | governing SLA policy |
| 8 | `policy_name` | text | no | denormalized policy name |
| 9 | `calendar_name` | text | yes | attached business calendar (0015); NULL = embedded-calendar regime |
| 10 | `calendar_timezone` | text | yes | calendar tz for local-hour context |
| 11 | `first_response_due` | timestamptz | yes | target |
| 12 | `first_responded_at` | timestamptz | yes | actual |
| 13 | `response_met` | boolean | yes | three-valued: NULL until both due+actual exist, else actual ≤ due |
| 14 | `resolution_due` | timestamptz | yes | target |
| 15 | `resolved_at` | timestamptz | yes | ticket resolution instant |
| 16 | `resolution_met` | boolean | yes | three-valued like `response_met` |
| 17 | `breached` | boolean | no | breach flag from the escalation engine |
| 18 | `paused_business_secs` | bigint | no | cumulative paused budget consumption |

## Example

```sql
SELECT policy_name,
       COUNT(*) FILTER (WHERE response_met IS NOT DISTINCT FROM false) AS response_misses,
       COUNT(*) FILTER (WHERE resolution_met IS NOT DISTINCT FROM false) AS resolution_misses
FROM ops.v1_sla_outcomes
WHERE org_id = $1 AND ticket_created_at >= now() - interval '30 days'
GROUP BY policy_name;
```
