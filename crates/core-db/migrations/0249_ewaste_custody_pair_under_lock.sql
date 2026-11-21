-- E-waste custody, review cycle six (migration 0249).
--
-- The line guard read the serial <-> asset pair before it took the unit row
-- locks. If another transaction paired asset A with serial S in that window,
-- the reservation query could miss A's line and two lines could stay open for
-- one physical unit.
--
-- This migration replaces `ewaste_intake_line_guard` and re-reads the pair
-- after both rows are locked, in the order migration 0232 uses (asset, then
-- serial). A pair that differs from the value read before the locks raises a
-- retryable refusal, the same shape `inventory_class_movement_apply` uses; the
-- API maps the SQLSTATE to a conflict.
--
-- Every other rule is unchanged: insert-time reservation, immutability of
-- `serial_id` and `device_asset_id` on update, independent anonymous lots,
-- frozen destruction evidence, and the actor-only erasure exception.
--
-- Additive-only: one `CREATE OR REPLACE FUNCTION` statement.

CREATE OR REPLACE FUNCTION ewaste_intake_line_guard() RETURNS trigger AS $$
DECLARE
  intake_status text;
  intake_client uuid;
  destruction_required boolean;
  check_unit boolean := false;
  unit_serial uuid;
  unit_asset uuid;
  pair_asset uuid;
  pair_asset_after uuid;
  pair_serial_after uuid;
BEGIN
  IF TG_OP = 'UPDATE' THEN
    IF NOT EXISTS (SELECT 1 FROM orgs o WHERE o.id = NEW.org_id) THEN
      RETURN NEW;
    END IF;
    IF NEW.created_by IS NULL
       AND (to_jsonb(NEW) - 'created_by') = (to_jsonb(OLD) - 'created_by') THEN
      RETURN NEW;
    END IF;
    IF NEW.destruction_completed_by IS NULL
       AND (to_jsonb(NEW) - 'destruction_completed_by')
         = (to_jsonb(OLD) - 'destruction_completed_by') THEN
      RETURN NEW;
    END IF;
    IF NEW.created_by IS NULL
       AND NEW.destruction_completed_by IS NULL
       AND (to_jsonb(NEW) - 'created_by' - 'destruction_completed_by')
         = (to_jsonb(OLD) - 'created_by' - 'destruction_completed_by') THEN
      RETURN NEW;
    END IF;
  END IF;

  SELECT status, client_id, data_destruction_required
    INTO intake_status, intake_client, destruction_required
    FROM ewaste_intakes
   WHERE id = NEW.intake_id AND org_id = NEW.org_id
   FOR SHARE;

  IF intake_status IS NULL THEN
    RAISE EXCEPTION 'ewaste: intake % not found in org %',
      NEW.intake_id, NEW.org_id;
  END IF;

  IF TG_OP = 'INSERT' THEN
    IF intake_status <> 'open' THEN
      RAISE EXCEPTION 'ewaste: intake % is closed and accepts no new line',
        NEW.intake_id;
    END IF;
    -- A data-bearing unit on an intake that requires destruction starts
    -- pending; the custodian completes it, and only then may the unit leave.
    IF NEW.data_bearing
       AND destruction_required
       AND NEW.destruction_status = 'not_required'
    THEN
      NEW.destruction_status := 'pending';
    END IF;
    IF NEW.destruction_document_id IS NOT NULL AND NOT EXISTS (
      SELECT 1 FROM documents d
       WHERE d.id = NEW.destruction_document_id AND d.org_id = NEW.org_id
    ) THEN
      RAISE EXCEPTION
        'ewaste: destruction document % is not in org % evidence',
        NEW.destruction_document_id, NEW.org_id;
    END IF;
    IF NEW.device_asset_id IS NOT NULL AND NOT EXISTS (
      SELECT 1 FROM device_assets a
       WHERE a.id = NEW.device_asset_id
         AND a.org_id = NEW.org_id
         AND a.scope = 'client'
         AND a.client_id = intake_client
    ) THEN
      RAISE EXCEPTION
        'ewaste: asset % is not a client asset of this intake''s client',
        NEW.device_asset_id;
    END IF;
    check_unit := true;
  ELSE
    -- A line's unit is a fact, chosen when the line is received. The
    -- retirement trigger runs on insert only, so a unit attached later would
    -- never leave the client's fleet, and its disposal would skip the
    -- terminal class movement.
    IF NEW.serial_id IS DISTINCT FROM OLD.serial_id
       OR NEW.device_asset_id IS DISTINCT FROM OLD.device_asset_id THEN
      RAISE EXCEPTION
        'ewaste: a line''s unit is a fact; serial_id and device_asset_id cannot change after insert';
    END IF;
    IF OLD.intake_id <> NEW.intake_id OR OLD.org_id <> NEW.org_id THEN
      RAISE EXCEPTION 'ewaste: a line cannot move between intakes or orgs';
    END IF;
    IF intake_status <> 'open' THEN
      RAISE EXCEPTION 'ewaste: intake % is closed and its lines are facts',
        NEW.intake_id;
    END IF;
    IF (OLD.line_kind IS DISTINCT FROM NEW.line_kind
        OR OLD.category IS DISTINCT FROM NEW.category
        OR OLD.weight_g IS DISTINCT FROM NEW.weight_g
        OR OLD.quantity IS DISTINCT FROM NEW.quantity
        OR OLD.data_bearing IS DISTINCT FROM NEW.data_bearing
        OR OLD.destruction_status IS DISTINCT FROM NEW.destruction_status
        OR OLD.destruction_completed_at IS DISTINCT FROM NEW.destruction_completed_at
        OR OLD.destruction_completed_by IS DISTINCT FROM NEW.destruction_completed_by
        OR OLD.destruction_document_id IS DISTINCT FROM NEW.destruction_document_id)
       AND EXISTS (
         SELECT 1 FROM ewaste_dispositions d
          WHERE d.line_id = NEW.id AND d.org_id = NEW.org_id
       )
    THEN
      RAISE EXCEPTION
        'ewaste: line % already has a disposition; its identity, weight and destruction evidence are facts',
        NEW.id;
    END IF;
    -- A document that was deleted leaves its id behind (it is not a foreign
    -- key); only a newly named document is validated.
    IF NEW.destruction_document_id IS NOT NULL
       AND NEW.destruction_document_id IS DISTINCT FROM OLD.destruction_document_id
       AND NOT EXISTS (
         SELECT 1 FROM documents d
          WHERE d.id = NEW.destruction_document_id AND d.org_id = NEW.org_id
       )
    THEN
      RAISE EXCEPTION
        'ewaste: destruction document % is not in org % evidence',
        NEW.destruction_document_id, NEW.org_id;
    END IF;
    -- An asset that was deleted leaves its id behind; only a newly named asset
    -- is validated against the intake's client.
    IF NEW.device_asset_id IS NOT NULL
       AND NEW.device_asset_id IS DISTINCT FROM OLD.device_asset_id
       AND NOT EXISTS (
         SELECT 1 FROM device_assets a
          WHERE a.id = NEW.device_asset_id
            AND a.org_id = NEW.org_id
            AND a.scope = 'client'
            AND a.client_id = intake_client
       )
    THEN
      RAISE EXCEPTION
        'ewaste: asset % is not a client asset of this intake''s client',
        NEW.device_asset_id;
    END IF;
  END IF;

  -- One physical unit, one unresolved custody line. The canonical identity is
  -- the pair the migration knows: a serial names its asset, an asset names its
  -- serial, so an asset-only line cannot slip past a serial reservation or the
  -- reverse. The unit rows are locked in the order migration 0232's movement
  -- trigger uses (asset, then serial), so two intakes receiving the same unit
  -- serialize instead of both reading "no reservation". A line whose weight is
  -- fully disposed (or whose intake is therefore closed) no longer reserves
  -- the unit. Whether it can come back is the class guard's call, and a
  -- terminal class refuses. Anonymous lots carry no unit ids and stay
  -- independent.
  IF check_unit THEN
    unit_serial := NEW.serial_id;
    unit_asset := NEW.device_asset_id;
    IF unit_serial IS NOT NULL THEN
      SELECT s.device_asset_id INTO pair_asset
        FROM stock_serials s
       WHERE s.id = unit_serial AND s.org_id = NEW.org_id;
      unit_asset := coalesce(unit_asset, pair_asset);
    ELSIF unit_asset IS NOT NULL THEN
      SELECT a.serial_id INTO unit_serial
        FROM device_assets a
       WHERE a.id = unit_asset AND a.org_id = NEW.org_id;
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

    -- The pair was read before the locks. A concurrent pairing can attach the
    -- counterpart in that window, and the reservation below would then test
    -- the wrong identity. Re-read the pair under the locks and refuse a change
    -- so the writer retries against the pair that is current.
    IF unit_serial IS NOT NULL THEN
      SELECT s.device_asset_id INTO pair_asset_after
        FROM stock_serials s
       WHERE s.id = unit_serial AND s.org_id = NEW.org_id;
      IF pair_asset_after IS DISTINCT FROM unit_asset THEN
        RAISE EXCEPTION
          'ewaste: unit % paired or unpaired while the line was recorded; retry',
          unit_serial;
      END IF;
    END IF;
    IF unit_asset IS NOT NULL THEN
      SELECT a.serial_id INTO pair_serial_after
        FROM device_assets a
       WHERE a.id = unit_asset AND a.org_id = NEW.org_id;
      IF pair_serial_after IS DISTINCT FROM unit_serial THEN
        RAISE EXCEPTION
          'ewaste: unit % paired or unpaired while the line was recorded; retry',
          unit_asset;
      END IF;
    END IF;

    IF EXISTS (
      SELECT 1
        FROM ewaste_intake_lines l
       WHERE l.org_id = NEW.org_id
         AND l.id <> NEW.id
         AND ((unit_serial IS NOT NULL AND l.serial_id = unit_serial)
              OR (unit_asset IS NOT NULL AND l.device_asset_id = unit_asset))
         AND coalesce((
               SELECT sum(d.weight_g)
                 FROM ewaste_dispositions d
                WHERE d.line_id = l.id AND d.org_id = l.org_id), 0) < l.weight_g
    ) THEN
      RAISE EXCEPTION
        'ewaste: unit (serial %, asset %) already has an unresolved custody line',
        unit_serial, unit_asset;
    END IF;
  END IF;

  RETURN NEW;
END;
$$ LANGUAGE plpgsql;
