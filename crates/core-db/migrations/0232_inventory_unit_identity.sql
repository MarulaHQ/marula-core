-- Inventory unit identity: one custody class per serial and asset, and an
-- append-only ledger for a class change.
--
-- This is the first of three migrations (0232-0234) that add the inventory
-- identity, pair and money vocabulary; 0231 was the head before it.
--
-- Purpose: one physical unit, one identity, one custody class (ADR 0035
-- decisions 1 and 2). Nothing here changes how stock, assets, tools or loans
-- behave today: the columns are additive and their defaults reproduce the
-- current meaning, and the trigger only refuses a change that today's code
-- never makes.
--
--   * `stock_serials.custody_class` and `device_assets.custody_class` — the
--     class the unit is in right now, one of the seven live classes plus the
--     four terminal dispositions in ADR 0035 §1. NOT NULL, so a unit is in
--     exactly one class by construction.
--
--   * `stock_serials.device_asset_id` and `device_assets.serial_id` — the
--     cross-reference between the two projections of the same unit. Both are
--     nullable; a conversion fills one and migration 0233 makes the pair agree.
--
--   * `inventory_class_movements` — the append-only record of a class change.
--     A class change is a row with a reason and the document that caused it,
--     never a silent update of a status column. The insert trigger applies the
--     new class to the unit (and to its paired projection), and a guard on the
--     unit tables refuses a direct `custody_class` update. That is what makes
--     "a class transition writes one movement and no duplicate unit" a schema
--     fact rather than a convention.
--
-- The backfill reads the vocabulary the rows already carry, never a guess:
-- a tool serial is `tools`, an out serial with a client is `rental_out`, and
-- everything else is `sale_stock`. An asset follows its existing `scope` and
-- `ownership`: internal and owned is `internal_fleet`, internal and rented is
-- `rental_in`, client-scoped and owned is `client_custody`, client-scoped and
-- not owned is `rental_out`.
--
-- Non-purpose: no lot identity. A lot is a line plus a quantity (ADR 0035 §1);
-- that table arrives with the e-waste work and extends this ledger's
-- `unit_kind` vocabulary when it does, so no free-text lot key is invented here.
--
-- Additive-only: new nullable columns, new table, new triggers. Existing rows
-- are backfilled from their own columns; nothing is dropped or rewritten.

-- --- custody class, one per unit ---------------------------------------------

ALTER TABLE stock_serials ADD COLUMN custody_class text;

UPDATE stock_serials s
   SET custody_class = CASE
         WHEN i.is_tool THEN 'tools'
         WHEN s.client_id IS NOT NULL THEN 'rental_out'
         ELSE 'sale_stock'
       END
  FROM stock_items i
 WHERE i.id = s.stock_item_id
   AND i.org_id = s.org_id;

-- A serial whose item row did not resolve cannot exist (the FK cascades), but
-- the column is about to become NOT NULL and a missed join must not fail the
-- migration on an install that has one.
UPDATE stock_serials
   SET custody_class = 'sale_stock'
 WHERE custody_class IS NULL;

ALTER TABLE stock_serials
  ALTER COLUMN custody_class SET DEFAULT 'sale_stock',
  ALTER COLUMN custody_class SET NOT NULL;

ALTER TABLE stock_serials
  ADD CONSTRAINT stock_serials_custody_class_check
  CHECK (custody_class IN ('sale_stock', 'internal_fleet', 'rental_in',
                           'rental_out', 'client_custody', 'ewaste_custody',
                           'tools', 'sold', 'recycled', 'destroyed', 'returned'));

-- Plain (not CONCURRENTLY) indexes: sqlx runs each migration inside a
-- transaction, where CONCURRENTLY is illegal. Every lock this file takes is
-- held until the transaction commits, including through the backfills below,
-- so the honest cost is one transaction-length lock on each table. At MSP fleet
-- and stock sizes these tables are thousands of rows, which keeps that window
-- to a statement-scale pause rather than an outage.
CREATE INDEX idx_stock_serials_custody
  ON stock_serials (org_id, custody_class);
ALTER TABLE device_assets ADD COLUMN custody_class text;

UPDATE device_assets
   SET custody_class = CASE
         WHEN scope = 'internal_org' AND ownership = 'owned' THEN 'internal_fleet'
         WHEN scope = 'internal_org' THEN 'rental_in'
         WHEN ownership = 'owned' THEN 'client_custody'
         ELSE 'rental_out'
       END
 WHERE custody_class IS NULL;

ALTER TABLE device_assets
  ALTER COLUMN custody_class SET DEFAULT 'client_custody',
  ALTER COLUMN custody_class SET NOT NULL;

ALTER TABLE device_assets
  ADD CONSTRAINT device_assets_custody_class_check
  CHECK (custody_class IN ('sale_stock', 'internal_fleet', 'rental_in',
                           'rental_out', 'client_custody', 'ewaste_custody',
                           'tools', 'sold', 'recycled', 'destroyed', 'returned'));

CREATE INDEX idx_device_assets_custody
  ON device_assets (org_id, custody_class);

-- A new row that does not name a class gets the one its own columns imply.
-- A caller that names one keeps it; the derivation only replaces the column
-- default, which is the value a writer who did not think about class leaves
-- behind.
CREATE FUNCTION inventory_custody_class_on_insert() RETURNS trigger AS $$
BEGIN
  IF TG_TABLE_NAME = 'stock_serials' THEN
    IF NEW.custody_class = 'sale_stock'
       AND EXISTS (
         SELECT 1 FROM stock_items i
          WHERE i.id = NEW.stock_item_id
            AND i.org_id = NEW.org_id
            AND i.is_tool
       )
    THEN
      NEW.custody_class := 'tools';
    ELSIF NEW.custody_class = 'sale_stock' AND NEW.client_id IS NOT NULL THEN
      -- The backfill calls a serial that already names a client `rental_out`;
      -- a new row that omits the class must land in the same place.
      NEW.custody_class := 'rental_out';
    END IF;
  ELSIF NEW.custody_class = 'client_custody' THEN
    NEW.custody_class := CASE
      WHEN NEW.scope = 'internal_org' AND NEW.ownership = 'owned' THEN 'internal_fleet'
      WHEN NEW.scope = 'internal_org' THEN 'rental_in'
      WHEN NEW.ownership = 'owned' THEN 'client_custody'
      ELSE 'rental_out'
    END;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER stock_serials_custody_class_on_insert
  BEFORE INSERT ON stock_serials
  FOR EACH ROW EXECUTE FUNCTION inventory_custody_class_on_insert();

CREATE TRIGGER device_assets_custody_class_on_insert
  BEFORE INSERT ON device_assets
  FOR EACH ROW EXECUTE FUNCTION inventory_custody_class_on_insert();

-- --- serial <-> asset cross-reference ---------------------------------------

ALTER TABLE stock_serials
  ADD COLUMN device_asset_id uuid REFERENCES device_assets(id) ON DELETE SET NULL;
ALTER TABLE device_assets
  ADD COLUMN serial_id uuid REFERENCES stock_serials(id) ON DELETE SET NULL;

-- --- class-transition ledger -------------------------------------------------

CREATE TABLE inventory_class_movements (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- Which projection of the unit this movement names. Lots join when their
  -- table lands; the CHECK is widened additively then. These are not foreign
  -- keys on purpose: the ledger records that a unit with this id changed
  -- class, and deleting the unit must not erase that history or block the
  -- org cascade that tenant erasure performs. The apply trigger checks the
  -- unit exists, in this org, when the movement is written.
  unit_kind text NOT NULL CHECK (unit_kind IN ('serial', 'asset')),
  serial_id uuid,
  device_asset_id uuid,
  from_class text NOT NULL
    CHECK (from_class IN ('sale_stock', 'internal_fleet', 'rental_in',
                          'rental_out', 'client_custody', 'ewaste_custody',
                          'tools', 'sold', 'recycled', 'destroyed', 'returned')),
  to_class text NOT NULL
    CHECK (to_class IN ('sale_stock', 'internal_fleet', 'rental_in',
                        'rental_out', 'client_custody', 'ewaste_custody',
                        'tools', 'sold', 'recycled', 'destroyed', 'returned')),
  -- Why the unit moved; a class change with no reason is not a record.
  -- Why the unit moved; a class change with no reason is not a record.
  -- btrim keeps a single space from passing as a reason.
  reason text NOT NULL
    CHECK (btrim(reason) <> '' AND char_length(reason) <= 500),
  -- The document that caused the move. `manual` is the honest source for an
  -- operator correction and carries no source id; every other kind must.
  source_kind text NOT NULL
    CHECK (source_kind IN ('purchase_receipt_line', 'stock_movement',
                           'device_asset', 'client_loan', 'tool_checkout',
                           'credit_line', 'ewaste_disposition', 'manual')),
  source_id uuid,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT inventory_class_movements_unit_ref CHECK (
    (unit_kind = 'serial' AND serial_id IS NOT NULL AND device_asset_id IS NULL)
    OR
    (unit_kind = 'asset' AND device_asset_id IS NOT NULL AND serial_id IS NULL)
  ),
  CONSTRAINT inventory_class_movements_moves CHECK (from_class <> to_class),
  CONSTRAINT inventory_class_movements_source CHECK (
    (source_kind = 'manual' AND source_id IS NULL)
    OR (source_kind <> 'manual' AND source_id IS NOT NULL)
  )
);

CREATE INDEX idx_inventory_class_movements_serial
  ON inventory_class_movements (org_id, serial_id, created_at DESC)
  WHERE serial_id IS NOT NULL;
CREATE INDEX idx_inventory_class_movements_asset
  ON inventory_class_movements (org_id, device_asset_id, created_at DESC)
  WHERE device_asset_id IS NOT NULL;

ALTER TABLE inventory_class_movements ENABLE ROW LEVEL SECURITY;
CREATE POLICY inventory_class_movements_org ON inventory_class_movements
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- The ledger is append-only, like stock_movements. Two removals are not
-- edits: the tenant erasure contract deletes `orgs` and lets the cascade
-- remove every org-scoped row, and `created_by` is ON DELETE SET NULL so a
-- user hard-delete nulls the column. Both must pass. A direct UPDATE or
-- DELETE by any writer must not.
CREATE FUNCTION inventory_class_movements_append_only() RETURNS trigger AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN
    -- In a cascade the parent org row is already gone when the child row is
    -- deleted. A direct delete still sees its org and is refused.
    IF NOT EXISTS (SELECT 1 FROM orgs o WHERE o.id = OLD.org_id) THEN
      RETURN OLD;
    END IF;
  ELSIF TG_OP = 'UPDATE'
        AND NEW.created_by IS NULL
        AND OLD.created_by IS NOT NULL
        AND NOT EXISTS (SELECT 1 FROM users u WHERE u.id = OLD.created_by)
        AND (to_jsonb(NEW) - 'created_by') = (to_jsonb(OLD) - 'created_by') THEN
    -- Only the foreign key's SET NULL after the creator row itself is gone.
    RETURN NEW;
  END IF;
  RAISE EXCEPTION 'inventory_class_movements is append-only: % rejected', TG_OP;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER inventory_class_movements_append_only_guard
  BEFORE UPDATE OR DELETE ON inventory_class_movements
  FOR EACH ROW EXECUTE FUNCTION inventory_class_movements_append_only();

-- Applying a movement is the only way a class changes. The trigger refuses a
-- movement whose stated `from_class` is not the unit's current class (a stale
-- writer cannot silently overwrite a later move), refuses a move while the unit
-- is out on an open agreement, checkout or ticket, then applies the new class
-- to the unit and to its paired projection.
CREATE FUNCTION inventory_class_movement_apply() RETURNS trigger AS $$
DECLARE
  current_class text;
  current_serial uuid;
  current_asset uuid;
  unit_serial uuid;
  unit_asset uuid;
BEGIN
  -- Resolve both projections first, then lock them in one order (asset row,
  -- then serial row) before reading the class. Without the lock two movements
  -- can both read the old class under READ COMMITTED and both claim the same
  -- `from_class`. Without one order the two projections can deadlock.
  IF NEW.unit_kind = 'serial' THEN
    unit_serial := NEW.serial_id;
    SELECT device_asset_id INTO unit_asset
      FROM stock_serials
     WHERE id = NEW.serial_id AND org_id = NEW.org_id;
  ELSE
    unit_asset := NEW.device_asset_id;
    SELECT serial_id INTO unit_serial
      FROM device_assets
     WHERE id = NEW.device_asset_id AND org_id = NEW.org_id;
  END IF;

  IF unit_asset IS NOT NULL THEN
    PERFORM 1 FROM device_assets
      WHERE id = unit_asset AND org_id = NEW.org_id
      FOR UPDATE;
  END IF;
  IF unit_serial IS NOT NULL THEN
    PERFORM 1 FROM stock_serials
      WHERE id = unit_serial AND org_id = NEW.org_id
      FOR UPDATE;
  END IF;

  IF NEW.unit_kind = 'serial' THEN
    SELECT custody_class, device_asset_id INTO current_class, current_asset
      FROM stock_serials
     WHERE id = NEW.serial_id AND org_id = NEW.org_id;
  ELSE
    SELECT custody_class, serial_id INTO current_class, current_serial
      FROM device_assets
     WHERE id = NEW.device_asset_id AND org_id = NEW.org_id;
  END IF;

  -- The pair was read before the locks. If it changed in that window, the
  -- open-agreement checks below would test the wrong partner, so refuse and
  -- let the writer retry against the pair that is current under the locks.
  IF (NEW.unit_kind = 'serial' AND current_asset IS DISTINCT FROM unit_asset)
     OR (NEW.unit_kind = 'asset' AND current_serial IS DISTINCT FROM unit_serial) THEN
    RAISE EXCEPTION
      'inventory class movement: unit % paired or unpaired while the movement was recorded; retry',
      COALESCE(NEW.serial_id, NEW.device_asset_id);
  END IF;

  IF current_class IS NULL THEN
    RAISE EXCEPTION 'inventory class movement: unit % not found in org %',
      COALESCE(NEW.serial_id, NEW.device_asset_id), NEW.org_id;
  END IF;
  IF current_class IN ('sold', 'recycled', 'destroyed', 'returned') THEN
    RAISE EXCEPTION
      'inventory class movement: unit % is in terminal class % and cannot move again',
      COALESCE(NEW.serial_id, NEW.device_asset_id), current_class;
  END IF;
  IF current_class <> NEW.from_class THEN
    RAISE EXCEPTION
      'inventory class movement: from_class % is not the current class % (unit %)',
      NEW.from_class, current_class, COALESCE(NEW.serial_id, NEW.device_asset_id);
  END IF;

  -- Both projections of a unit are checked, because a class change moves the
  -- pair: a serial that is out under a loan on its asset side is just as stuck.
  IF unit_serial IS NOT NULL THEN
    IF EXISTS (
      SELECT 1
        FROM client_loan_lines l
        JOIN client_loans h ON h.id = l.loan_id
       WHERE l.serial_id = unit_serial
         AND h.org_id = NEW.org_id
         AND l.returned_at IS NULL
    ) THEN
      RAISE EXCEPTION 'inventory class movement: unit % is on an open client loan',
        unit_serial;
    END IF;
    IF EXISTS (
      SELECT 1 FROM tool_checkout_lines t
       WHERE t.serial_id = unit_serial
         AND t.org_id = NEW.org_id
         AND t.returned_at IS NULL
    ) THEN
      RAISE EXCEPTION 'inventory class movement: unit % is on an open tool checkout',
        unit_serial;
    END IF;
    IF EXISTS (
      SELECT 1 FROM stock_serials s
       WHERE s.id = unit_serial
         AND s.org_id = NEW.org_id
         AND s.ticket_id IS NOT NULL
    ) THEN
      RAISE EXCEPTION 'inventory class movement: unit % is consumed on a ticket',
        unit_serial;
    END IF;
  END IF;

  IF unit_asset IS NOT NULL THEN
    IF EXISTS (
      SELECT 1
        FROM client_loan_lines l
        JOIN client_loans h ON h.id = l.loan_id
       WHERE l.device_asset_id = unit_asset
         AND h.org_id = NEW.org_id
         AND l.returned_at IS NULL
    ) THEN
      RAISE EXCEPTION 'inventory class movement: asset % is on an open client loan',
        unit_asset;
    END IF;
  END IF;

  IF NEW.unit_kind = 'serial' THEN
    UPDATE stock_serials
       SET custody_class = NEW.to_class, updated_at = now()
     WHERE id = NEW.serial_id AND org_id = NEW.org_id;
    UPDATE device_assets a
       SET custody_class = NEW.to_class, updated_at = now()
      FROM stock_serials s
     WHERE s.id = NEW.serial_id
       AND s.org_id = NEW.org_id
       AND a.id = s.device_asset_id
       AND a.org_id = NEW.org_id;
  ELSE
    UPDATE device_assets
       SET custody_class = NEW.to_class, updated_at = now()
     WHERE id = NEW.device_asset_id AND org_id = NEW.org_id;
    UPDATE stock_serials s
       SET custody_class = NEW.to_class, updated_at = now()
      FROM device_assets a
     WHERE a.id = NEW.device_asset_id
       AND a.org_id = NEW.org_id
       AND s.id = a.serial_id
       AND s.org_id = NEW.org_id;
  END IF;

  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- BEFORE INSERT is safe because nothing upserts this ledger: every writer
-- inserts a fresh UUID with no `ON CONFLICT`, so the row cannot be skipped
-- after the trigger has applied the change. A future upsert writer must move
-- this trigger AFTER INSERT first.
CREATE TRIGGER inventory_class_movements_apply
  BEFORE INSERT ON inventory_class_movements
  FOR EACH ROW EXECUTE FUNCTION inventory_class_movement_apply();

-- The column is derived; a direct UPDATE would be a class change with no
-- movement row, which is the silent status string the ADR forbids. The only
-- writer allowed through is the movement trigger above, which runs one trigger
-- level deeper than any statement the application can issue; `pg_trigger_depth`
-- is therefore the check, not a setting the caller could set for itself.
CREATE FUNCTION inventory_custody_class_guard() RETURNS trigger AS $$
BEGIN
  IF pg_trigger_depth() > 1 THEN
    RETURN NEW;
  END IF;
  IF NEW.custody_class IS DISTINCT FROM OLD.custody_class THEN
    RAISE EXCEPTION
      'custody_class is derived from inventory_class_movements: record a class transition instead of updating the column';
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER stock_serials_custody_class_guard
  BEFORE UPDATE OF custody_class ON stock_serials
  FOR EACH ROW EXECUTE FUNCTION inventory_custody_class_guard();

CREATE TRIGGER device_assets_custody_class_guard
  BEFORE UPDATE OF custody_class ON device_assets
  FOR EACH ROW EXECUTE FUNCTION inventory_custody_class_guard();
