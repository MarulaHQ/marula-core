# `ops.v1_tickets`

> Contract level **v1** · introduced 0026 · taxonomy columns appended 0059
> Binding rules live on the [read-model index](../read-model.md).

One row per ticket with the workflow, SLA, and classification truth joined in.
`category` is the **semantic** category from `board_statuses` (the workflow
layer is authoritative, and `tickets.category` mirrors it by transactional
update) — renames of boards/statuses never change it (§5 guarantee).

## Columns (in contract order)

| # | Column | Type | Nullable | Notes |
|---|---|---|---|---|
| 1 | `id` | uuid | no | ticket id (PK of this view) |
| 2 | `org_id` | uuid | no | owning org (RLS seam) |
| 3 | `number` | text | no | human ticket number, org-unique |
| 4 | `board_id` | uuid | no | owning board |
| 5 | `board_name` | text | no | denormalized board name |
| 6 | `status_id` | uuid | no | current status |
| 7 | `status_name` | text | no | current status name |
| 8 | `category` | text | no | semantic category (`open`\|`in_progress`\|`waiting`\|`closed`) from board_statuses |
| 9 | `priority` | text | no | `p1`…`p4` |
| 10 | `title` | text | no | ticket title |
| 11 | `client_id` | uuid | yes | nullable client |
| 12 | `client_name` | text | yes | denormalized client name |
| 13 | `site_id` | uuid | yes | nullable site |
| 14 | `assignee_id` | uuid | yes | nullable staff assignee |
| 15 | `assignee_name` | text | yes | denormalized assignee display name |
| 16 | `requester_id` | uuid | yes | nullable staff requester |
| 17 | `due_at` | timestamptz | yes | nullable business deadline |
| 18 | `resolved_at` | timestamptz | yes | set when moved into closed category |
| 19 | `created_at` | timestamptz | no | |
| 20 | `updated_at` | timestamptz | no | |
| 21 | `first_response_due` | timestamptz | yes | current SLA clock target (null when no SLA state) |
| 22 | `resolution_due` | timestamptz | yes | current SLA resolution target |
| 23 | `breached` | boolean | no | SLA breach flag (false when no SLA state) |
| 24 | `ticket_type_id` | uuid | yes | `tickets.type_id` → ticket_types (0059) |
| 25 | `ticket_type_name` | text | yes | denormalized type name |
| 26 | `ticket_category_id` | uuid | yes | resolved ROOT category: the referenced node's parent when it is a subcategory, else the node |
| 27 | `ticket_category_name` | text | yes | resolved root category name |
| 28 | `ticket_subcategory_id` | uuid | yes | set only when the referenced node is a subcategory |
| 29 | `ticket_subcategory_name` | text | yes | subcategory name |

## Contract notes

- Columns 1–23 are the byte-stable 0026 prefix, and 24–29 were appended in
  0059. New columns may only append; order is NOT part of the contract.
- `category` is the semantic workflow primitive, never a label, and the
  `ticket_*` taxonomy columns are classification-only (§5) and never affect
  SLA, billing, or automation behavior.

## Example

```sql
SELECT number, priority, status_name, client_name,
       ticket_category_name, ticket_subcategory_name
FROM ops.v1_tickets
WHERE org_id = $1 AND category = 'open'
ORDER BY created_at DESC
LIMIT 50;
```
