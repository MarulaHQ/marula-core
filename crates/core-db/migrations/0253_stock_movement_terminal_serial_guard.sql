-- A unit in a terminal custody class cannot move through the stock ledger.
--
-- The class-movement trigger updates `custody_class`, but the stock ledger never
-- read that column: a serial that reached `sold`, `recycled`, `destroyed` or
-- `returned` could still be received, consumed, transferred or shipped, and the
-- receive path would re-arm its `in_stock` status. That is a disposed unit
-- re-entering live inventory.
--
-- This guard refuses any movement that names such a serial. The check runs
-- before the row is written, so the transaction aborts rather than re-arming a
-- terminal unit. A movement for a serial that does not exist yet (a first
-- receive, where the upsert creates the row) has no class and passes.

CREATE FUNCTION stock_movement_terminal_serial_guard() RETURNS trigger AS $$
DECLARE
  current_class text;
BEGIN
  IF NEW.serial_id IS NULL THEN
    RETURN NEW;
  END IF;
  SELECT custody_class INTO current_class
    FROM stock_serials
   WHERE id = NEW.serial_id AND org_id = NEW.org_id;
  IF current_class IN ('sold', 'recycled', 'destroyed', 'returned') THEN
    RAISE EXCEPTION
      'serial % is in terminal custody class % and cannot move',
      NEW.serial_id, current_class;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER stock_movements_terminal_serial_guard
  BEFORE INSERT ON stock_movements
  FOR EACH ROW EXECUTE FUNCTION stock_movement_terminal_serial_guard();
