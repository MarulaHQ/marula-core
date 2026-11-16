-- The pause a client is told about (migration 0222).
--
-- Purpose: a project moved to `on_hold` may carry a reason the PM writes in the
-- client's own words. Until now the portal read answered a fixed sentence,
-- because no client-safe reason was stored anywhere: the internal hold note
-- lives in staff-facing text and must not cross the portal boundary. This
-- column is the one place a reason can be typed FOR the client, so the portal
-- can say what the PM wrote instead of guessing.
--
-- It is cleared when the project leaves hold: a pause no longer in force with a
-- stale reason attached would read as the current state on the next pause.
--
-- Compat: additive — one nullable column, no backfill needed (absent means "no
-- client-safe reason"), no index (the column is read with its project row and
-- never filtered on).

ALTER TABLE projects ADD COLUMN client_hold_reason text;
