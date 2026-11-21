-- Serial <-> asset pair agreement (Wave 76 lane 1,
-- `p76-inventory-foundation`; migration 0233 continues this lane's window).
--
-- `stock_serials` and `device_assets` are two projections of one physical unit
-- (ADR 0035 §2). Migration 0232 added the nullable reference each side carries;
-- this migration makes the pair agree, so a conversion updates one unit instead
-- of creating a second one.
--
-- Two mechanisms, in order:
--
--   1. A partial unique index on each side. One serial cannot name two assets
--      and one asset cannot name two serials, including under concurrent
--      writers. They are partial so the millions of unpaired rows cost nothing.
--
--   2. A BEFORE UPDATE OF sync trigger on each side. Setting one side fills the
--      other in the same statement; clearing one side clears the other. The
--      trigger refuses a cross-org pair, which the foreign keys alone would
--      allow because a UUID is unique across the whole database.
--
-- No backfill. A historical asset created by a loan carries its serial in
-- `identity_keys`, but matching that JSON to a serial number is a guess, and a
-- guessed pair is worse than no pair: the conversion path (renders under the
-- rentals lane) links a real pair at the moment it converts a unit. The
-- uniqueness indexes make every linked pair unambiguous until then.
--
-- Additive-only: two indexes and two triggers. No column or row is rewritten.
--
-- The two unique indexes deliberately carry no `map_unique_constraint` arm in
-- the API error map: they guard an internal projection pair, not user input.
-- No route accepts a serial-already-paired value, the same posture as the
-- `uq_tool_checkout_open_unit` and `uq_client_loan_open_unit` indexes that
-- guard single-holder and single-issue invariants. A collision is a writer bug
-- and should surface as a sanitised 409, not a user-facing message.

CREATE UNIQUE INDEX uq_stock_serials_device_asset
  ON stock_serials (org_id, device_asset_id)
  WHERE device_asset_id IS NOT NULL;

CREATE UNIQUE INDEX uq_device_assets_serial
  ON device_assets (org_id, serial_id)
  WHERE serial_id IS NOT NULL;

CREATE FUNCTION inventory_serial_asset_pair_sync() RETURNS trigger AS $$
DECLARE
  other_class text;
BEGIN
  IF TG_TABLE_NAME = 'stock_serials' THEN
    IF NEW.device_asset_id IS NOT NULL THEN
      SELECT custody_class INTO other_class
        FROM device_assets
       WHERE id = NEW.device_asset_id AND org_id = NEW.org_id;
      IF other_class IS NULL THEN
        RAISE EXCEPTION
          'serial % cannot pair with asset % outside org %',
          NEW.id, NEW.device_asset_id, NEW.org_id;
      END IF;
      IF other_class <> NEW.custody_class THEN
        RAISE EXCEPTION
          'serial % is % and asset % is %: one unit carries one class, record a class movement first',
          NEW.id, NEW.custody_class, NEW.device_asset_id, other_class;
      END IF;
      UPDATE device_assets
         SET serial_id = NEW.id, updated_at = now()
       WHERE id = NEW.device_asset_id
         AND org_id = NEW.org_id
         AND serial_id IS DISTINCT FROM NEW.id;
    ELSIF TG_OP = 'UPDATE' AND OLD.device_asset_id IS NOT NULL THEN
      UPDATE device_assets
         SET serial_id = NULL, updated_at = now()
       WHERE id = OLD.device_asset_id
         AND org_id = NEW.org_id
         AND serial_id = NEW.id;
    END IF;
  ELSE
    IF NEW.serial_id IS NOT NULL THEN
      SELECT custody_class INTO other_class
        FROM stock_serials
       WHERE id = NEW.serial_id AND org_id = NEW.org_id;
      IF other_class IS NULL THEN
        RAISE EXCEPTION
          'asset % cannot pair with serial % outside org %',
          NEW.id, NEW.serial_id, NEW.org_id;
      END IF;
      IF other_class <> NEW.custody_class THEN
        RAISE EXCEPTION
          'asset % is % and serial % is %: one unit carries one class, record a class movement first',
          NEW.id, NEW.custody_class, NEW.serial_id, other_class;
      END IF;
      UPDATE stock_serials
         SET device_asset_id = NEW.id, updated_at = now()
       WHERE id = NEW.serial_id
         AND org_id = NEW.org_id
         AND device_asset_id IS DISTINCT FROM NEW.id;
    ELSIF TG_OP = 'UPDATE' AND OLD.serial_id IS NOT NULL THEN
      UPDATE stock_serials
         SET device_asset_id = NULL, updated_at = now()
       WHERE id = OLD.serial_id
         AND org_id = NEW.org_id
         AND device_asset_id = NEW.id;
    END IF;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- AFTER, not BEFORE: the custody class of the inserted row is final only once
-- its own derivation trigger has run, and the other side of the pair must exist
-- before its class can be compared. A BEFORE trigger would read a default and
-- refuse a legitimate pair.
CREATE TRIGGER stock_serials_device_asset_pair
  AFTER INSERT OR UPDATE OF device_asset_id ON stock_serials
  FOR EACH ROW EXECUTE FUNCTION inventory_serial_asset_pair_sync();

CREATE TRIGGER device_assets_serial_pair
  AFTER INSERT OR UPDATE OF serial_id ON device_assets
  FOR EACH ROW EXECUTE FUNCTION inventory_serial_asset_pair_sync();
