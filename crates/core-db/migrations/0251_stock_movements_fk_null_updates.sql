-- Foreign-key actions on the movement ledger are not edits.
--
-- `stock_movements` is append-only, and 0246 lets the tenant-erasure cascade
-- through it. Six of its reference columns are `ON DELETE SET NULL`
-- (`from_location_id`, `to_location_id`, `serial_id`, `client_id`, `ticket_id`,
-- `created_by`), so deleting a location, a serial, a client, a ticket or a user
-- runs an UPDATE that clears one of them. The 0246 guard refused that update,
-- which turned those deletes into failures instead of the cascade the schema
-- declares.
--
-- This replacement permits exactly that shape: any of the six columns may go
-- from a value to NULL, and every other column must be unchanged. A direct
-- update, a value change, or a re-pointed reference still raises. The
-- dev-reset bypass and the erasure DELETE exemption are preserved.

CREATE OR REPLACE FUNCTION stock_movements_append_only() RETURNS trigger AS $$
BEGIN
  IF current_setting('app.dev_reset_bypass', true) = 'on' THEN
    RETURN COALESCE(NEW, OLD);
  END IF;

  IF TG_OP = 'DELETE' THEN
    IF NOT EXISTS (SELECT 1 FROM orgs o WHERE o.id = OLD.org_id) THEN
      RETURN OLD;
    END IF;
  ELSIF TG_OP = 'UPDATE' THEN
    IF (NEW.from_location_id IS NOT DISTINCT FROM OLD.from_location_id
        OR (NEW.from_location_id IS NULL AND OLD.from_location_id IS NOT NULL))
       AND (NEW.to_location_id IS NOT DISTINCT FROM OLD.to_location_id
        OR (NEW.to_location_id IS NULL AND OLD.to_location_id IS NOT NULL))
       AND (NEW.serial_id IS NOT DISTINCT FROM OLD.serial_id
        OR (NEW.serial_id IS NULL AND OLD.serial_id IS NOT NULL))
       AND (NEW.client_id IS NOT DISTINCT FROM OLD.client_id
        OR (NEW.client_id IS NULL AND OLD.client_id IS NOT NULL))
       AND (NEW.ticket_id IS NOT DISTINCT FROM OLD.ticket_id
        OR (NEW.ticket_id IS NULL AND OLD.ticket_id IS NOT NULL))
       AND (NEW.created_by IS NOT DISTINCT FROM OLD.created_by
        OR (NEW.created_by IS NULL AND OLD.created_by IS NOT NULL))
       AND (to_jsonb(NEW)
              - 'from_location_id' - 'to_location_id' - 'serial_id'
              - 'client_id' - 'ticket_id' - 'created_by')
         = (to_jsonb(OLD)
              - 'from_location_id' - 'to_location_id' - 'serial_id'
              - 'client_id' - 'ticket_id' - 'created_by')
    THEN
      RETURN NEW;
    END IF;
  END IF;

  RAISE EXCEPTION 'stock_movements is append-only: % rejected', TG_OP;
END;
$$ LANGUAGE plpgsql;
