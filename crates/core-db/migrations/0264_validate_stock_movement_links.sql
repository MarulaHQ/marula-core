-- Validates the constraints migration 0263 added `NOT VALID`, each in this
-- file's transaction instead of holding 0263's ACCESS EXCLUSIVE lock across the
-- scans. New rows were already held to every predicate from the moment 0263
-- committed; this pass proves the existing rows.
--
-- The scans are real but row-scaled: `stock_movements` and
-- `ticket_cost_lines` grow with movements and lines, not events, and both
-- widened predicates only accept values they accepted before (plus the new
-- stock-movement reversal clause), so every stored row passes.

ALTER TABLE stock_movements VALIDATE CONSTRAINT stock_movements_ticket_cost_line_fk;
ALTER TABLE stock_movements VALIDATE CONSTRAINT stock_movements_reverses_movement_fk;
ALTER TABLE ticket_cost_lines VALIDATE CONSTRAINT ticket_cost_lines_quantity_check;
ALTER TABLE stock_movements VALIDATE CONSTRAINT stock_movements_kind_check;
