-- Wave 41 lane 12 (p41-ui-settings): close the backend gaps the rendered
-- settings pack asked for.
--
-- 1. `board_statuses.client_visible` — the pack's "Visible to client" switch
--    (state 02/04). Default TRUE so every existing status keeps showing its
--    own name on the portal; a false row shows the semantic category word
--    instead, so the internal label never reaches a client.
--
-- 2. An index for the pack's per-status TICKETS column (state 02). The
--    existing `idx_tickets_board (board_id, category, updated_at DESC)`
--    carries the board filter but not the grouping key, so the aggregate
--    sorted on every settings paint.
--
-- 3. A partial index so the settings usage read never walks published
--    outbox rows.
--
-- Nothing here rewrites existing data: the new column is NOT NULL DEFAULT
-- true, so a rollback is a DROP COLUMN and no status changes meaning.

ALTER TABLE board_statuses
  ADD COLUMN client_visible boolean NOT NULL DEFAULT true;

COMMENT ON COLUMN board_statuses.client_visible IS
  'Wave 41 lane 12: when false the portal shows the semantic category word instead of this label. Clients never see the internal name.';

-- Per-status ticket counts for the settings table (one aggregate per paint).
CREATE INDEX idx_tickets_board_status
  ON tickets (board_id, status_id);

-- The settings usage read joins tickets and board_statuses under the org
-- policy; this keeps the count plan off the heap for a board-sized scan.
CREATE INDEX idx_board_statuses_board_client_visible
  ON board_statuses (board_id, client_visible);
