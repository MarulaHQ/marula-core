-- A purchase line's `billable` becomes tri-state.
--
-- It previously stored a resolved default, so a line created under a stock
-- header recorded `false`, and a later header edit that moved the line into
-- client custody left that `false` behind: the line then recharged without being
-- billable, while a line created under the client header defaulted to true.
--
-- `NULL` now means "the caller said nothing", and the effective value follows
-- the line's destination wherever it is read or received. An explicit `true` or
-- `false` is preserved, which is what the flag is for. The column default goes
-- too, so a caller that omits the field stores `NULL` rather than `false`;
-- existing booleans keep the value computed for the destination they had.
--
-- Lock cost: dropping a NOT NULL and a default is metadata-only, and no row is
-- rewritten.
--
-- The tools constraint is added NOT VALID so the exclusive lock does not wait on
-- a scan of every purchase line. New and changed rows are checked immediately;
-- migration 0243 validates the existing ones in its own transaction.

ALTER TABLE purchase_order_lines
  ALTER COLUMN billable DROP NOT NULL,
  ALTER COLUMN billable DROP DEFAULT;

COMMENT ON COLUMN purchase_order_lines.billable IS
  'Explicit billable choice for the line. NULL means the caller said nothing and the effective value follows the destination class.';

-- Tools are not a purchase destination. A tool is received as sale stock and
-- the serial insert trigger classifies it from the item's tool flag, so a
-- destination of `tools` would record a receipt with no tool movement. The API
-- refuses it; this keeps a writer that bypasses the API honest.
ALTER TABLE purchase_order_lines
  ADD CONSTRAINT purchase_order_lines_no_tools_destination
  CHECK (destination_class IS DISTINCT FROM 'tools') NOT VALID;
