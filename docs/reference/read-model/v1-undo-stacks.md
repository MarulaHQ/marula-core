# `ops.v1_record_undo_stack` & `ops.v1_field_undo_stack`

> Contract level **v1** · introduced 0041
> Binding rules live on the [read-model index](../read-model.md).

The instant-first undo read model over the 0028 write trail — two versioned
views (IF7 undo history):

## `ops.v1_record_undo_stack` — one row per applied write on a record

| # | Column | Type | Nullable | Notes |
|---|---|---|---|---|
| 1 | `write_id` | uuid | no | 0028 write-trail id (PK of this view) |
| 2 | `org_id` | uuid | no | owning org (RLS seam) |
| 3 | `entity_type` | text | no | logical entity kind |
| 4 | `entity_id` | uuid | no | affected record |
| 5 | `actor_user_id` | uuid | yes | nullable authoring staff member |
| 6 | `applied_at` | timestamptz | no | write instant |
| 7 | `undoable` | boolean | no | whether the write participates in undo |

## `ops.v1_field_undo_stack` — per-field applied-state history

| # | Column | Type | Nullable | Notes |
|---|---|---|---|---|
| 1 | `write_id` | uuid | no | 0028 write-trail id |
| 2 | `org_id` | uuid | no | owning org (RLS seam) |
| 3 | `entity_type` | text | no | logical entity kind |
| 4 | `entity_id` | uuid | no | affected record |
| 5 | `field` | text | no | changed field name |
| 6 | `applied_value` | jsonb | no | the applied (post-write) value |
| 7 | `previous_value` | jsonb | yes | nullable pre-write value |
| 8 | `actor_user_id` | uuid | yes | nullable authoring staff member |
| 9 | `applied_at` | timestamptz | no | write instant |
| 10 | `undoable` | boolean | no | whether the write participates in undo |

## Example

```sql
SELECT field, previous_value, applied_value, applied_at
FROM ops.v1_field_undo_stack
WHERE org_id = $1 AND entity_id = $2
ORDER BY applied_at DESC
LIMIT 20;
```