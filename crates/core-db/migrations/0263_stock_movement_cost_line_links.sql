-- A consumption is money on a job, and a reversal undoes both sides.
--
-- `stock_movements` gains the cost line a movement created, so the ledger row
-- and the job's cost name each other; the line already names the movement
-- through `ticket_cost_lines.source_kind = 'stock_movement'`. A reversal is a
-- second movement naming the one it reverses, with a unique index so a
-- consumption can only be reversed once.
--
-- Additive and nullable: every existing movement keeps working.
--
-- Lock cost, stated honestly: both tables are per-org ledgers that grow with
-- movement and line count, not with event volume, so they are the largest
-- tables this file touches but still row-scaled. Each `ALTER TABLE` takes
-- ACCESS EXCLUSIVE for the rest of the transaction, and the two CHECK swaps
-- re-scan every row to prove the widened predicate (the value sets only grow,
-- so every existing row passes). `NOT VALID` plus a later validate is declined
-- for the same reason migration 0234 gives: it moves the scan, and its lock,
-- into a second transaction. The unique index is plain because the runner wraps
-- each file in a transaction, where CONCURRENTLY is illegal; it covers a
-- nullable column that no existing row fills.

ALTER TABLE stock_movements
  ADD COLUMN ticket_cost_line_id uuid REFERENCES ticket_cost_lines(id) ON DELETE SET NULL,
  ADD COLUMN reverses_movement_id uuid REFERENCES stock_movements(id) ON DELETE SET NULL;

CREATE UNIQUE INDEX uq_stock_movements_reverses
  ON stock_movements (reverses_movement_id)
  WHERE reverses_movement_id IS NOT NULL;

-- A reversal line is negative. Every ordinary cost line stays positive; a
-- negated one is a stock-movement reversal that has not been billed, which is
-- what makes "the reversal restores the job" representable at all.
ALTER TABLE ticket_cost_lines DROP CONSTRAINT ticket_cost_lines_quantity_check;
ALTER TABLE ticket_cost_lines
  ADD CONSTRAINT ticket_cost_lines_quantity_check
  CHECK (quantity > 0
         OR (quantity < 0 AND source_kind = 'stock_movement' AND invoice_line_id IS NULL));

-- A reversal is its own kind: reads can tell a restore from a fresh
-- consumption, and the quantity stays positive.
ALTER TABLE stock_movements DROP CONSTRAINT stock_movements_kind_check;
ALTER TABLE stock_movements
  ADD CONSTRAINT stock_movements_kind_check
  CHECK (kind = ANY (ARRAY['receive', 'consume', 'adjust', 'transfer', 'ship_out',
                           'checkout', 'checkin', 'loan', 'loan_return',
                           'consume_reverse']));
