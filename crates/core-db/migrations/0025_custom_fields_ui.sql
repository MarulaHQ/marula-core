-- Custom fields UI v1 (TASKLIST 2.17). Migration 0009 already provides the
-- definition + JSONB value tables with org RLS; this adds what the ticket
-- form surface needs on top of it:
--   * board scoping for definitions (NULL = visible on every board),
--   * a value-lookup index so a ticket detail/list can fetch its values
--     without scanning, and a partial index for the active-definition read.

ALTER TABLE custom_field_defs
  ADD COLUMN board_id uuid REFERENCES boards(id) ON DELETE CASCADE;

CREATE INDEX custom_field_values_entity_idx
  ON custom_field_values (org_id, entity_id);

CREATE INDEX custom_field_defs_active_idx
  ON custom_field_defs (org_id, entity, board_id)
  WHERE archived_at IS NULL;
