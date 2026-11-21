-- A purchase line's `billable` becomes tri-state. Migration 0235 stored the
-- resolved default, so a draft line created under a stock header recorded
-- `false`, and a later header edit that moved the line into client custody left
-- that `false` behind: the line then recharged without being billable, while a
-- line created under the client header defaulted to true.
--
-- `NULL` now means "the caller said nothing", and the effective value follows
-- the line's destination wherever it is read or received. An explicit `true` or
-- `false` is preserved, which is what the flag is for.
--
-- Additive and metadata-only: no row is rewritten. Rows written before this
-- migration keep their stored boolean, which was computed for the destination
-- they had at the time.
--
-- This is W76-2's second migration. The reservation table in W76-1's landing
-- record sends a lane that needs one to the next free number after 0240.
-- 0241 was free when this was written, then the e-waste review fixes took it
-- (`0241_ewaste_custody_review_fixes.sql`), so this file moved to 0242 before
-- either reached the integration branch.

ALTER TABLE purchase_order_lines
  ALTER COLUMN billable DROP NOT NULL;

COMMENT ON COLUMN purchase_order_lines.billable IS
  'Explicit billable choice for the line. NULL means the caller said nothing and the effective value follows the destination class.';

-- Tools are not a purchase destination. A tool is received as sale stock and
-- the serial insert trigger (0232) classifies it from the item's tool flag, so a
-- destination of `tools` would record a receipt with no tool movement. The API
-- refuses it; this keeps a writer that bypasses the API honest.
ALTER TABLE purchase_order_lines
  ADD CONSTRAINT purchase_order_lines_no_tools_destination
  CHECK (destination_class IS DISTINCT FROM 'tools');
