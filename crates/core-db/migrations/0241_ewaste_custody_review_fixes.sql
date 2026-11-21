-- E-waste custody review fixes (migration 0241).
--
-- 0239 is reserved for the billing-join work and 0240 stays reserved, so a
-- lane that needs a second migration takes the next free number after 0240.
-- This file does not reuse a reservation.
--
-- Migration 0238 is already in the integration branch, so a database that
-- applied it must receive these corrections additively. The triggers created
-- by 0238 already point at the function names below; replacing the function
-- bodies is the whole change. No table, index, column or policy is touched.
--
--   * `ewaste_intake_line_guard` now enforces one unresolved custody line per
--     physical unit. It resolves the canonical pair (a serial names its asset,
--     an asset names its serial), locks the unit rows in the order 0232's
--     movement trigger uses, and refuses a second line whose weight is not
--     fully disposed. Anonymous lots carry no unit ids and stay independent.
--     A fully disposed line stops reserving the unit, but the class guard
--     still governs whether it can come back: a terminal class cannot. The
--     same
--     guard freezes the destruction state, completion timestamp, actor and
--     document once the line has a disposition; corrections before the first
--     disposition and the actor-only erasure exception still pass.
--
--   * `ewaste_disposition_apply` sends a unit transferred to a partner out of
--     live custody. The model has no partner class, so
--     `transferred_to_partner` maps to the terminal `returned` class and the
--     movement reason names the partner and the certificate. The disposition
--     event still records the true pathway.
--
-- Additive-only: two `CREATE OR REPLACE FUNCTION` statements.

CREATE OR REPLACE FUNCTION ewaste_intake_line_guard() RETURNS trigger AS $$
DECLARE
  intake_status text;
  intake_client uuid;
  destruction_required boolean;
  check_unit boolean := false;
  unit_serial uuid;
  unit_asset uuid;
  pair_asset uuid;
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
    IF OLD.intake_id <> NEW.intake_id OR OLD.org_id <> NEW.org_id THEN
      RAISE EXCEPTION 'ewaste: a line cannot move between intakes or orgs';
    END IF;
    IF intake_status <> 'open' THEN
      RAISE EXCEPTION 'ewaste: intake % is closed and its lines are facts',
        NEW.intake_id;
    END IF;
    IF (OLD.line_kind IS DISTINCT FROM NEW.line_kind
        OR OLD.serial_id IS DISTINCT FROM NEW.serial_id
        OR OLD.device_asset_id IS DISTINCT FROM NEW.device_asset_id
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
    -- A line can change which unit it names only before any disposition; the
    -- new unit is reserved exactly as a fresh line reserves one.
    IF NEW.serial_id IS DISTINCT FROM OLD.serial_id
       OR NEW.device_asset_id IS DISTINCT FROM OLD.device_asset_id THEN
      check_unit := true;
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

CREATE OR REPLACE FUNCTION ewaste_disposition_apply() RETURNS trigger AS $$
DECLARE
  v_line record;
  disposed_g bigint;
  terminal text;
  current_class text;
  v_reason text;
BEGIN
  INSERT INTO ewaste_events (
    id, org_id, intake_id, line_id, event_kind, occurred_at, storage_class,
    partner_name, disposition_id, actor_user_id, notes
  ) VALUES (
    gen_random_uuid(), NEW.org_id, NEW.intake_id, NEW.line_id, 'disposition',
    now(), NULL, NEW.partner_name, NEW.id,
    NEW.created_by,
    'disposition: ' || NEW.pathway
  );

  SELECT id, line_kind, weight_g, serial_id, device_asset_id
    INTO v_line
    FROM ewaste_intake_lines
   WHERE id = NEW.line_id AND org_id = NEW.org_id;

  SELECT coalesce(sum(weight_g), 0) INTO disposed_g
    FROM ewaste_dispositions
   WHERE line_id = NEW.line_id AND org_id = NEW.org_id;

  -- A unit leaves the live class only when its whole weight has a pathway.
  IF disposed_g <> v_line.weight_g THEN
    RETURN NEW;
  END IF;

  -- The model has no partner-custody class, and a unit handed to a partner is
  -- gone from the bench: it leaves live custody through the terminal `returned`
  -- disposition, whose reason names the partner and the certificate. `resold`
  -- is the sale disposition, `returned_to_client` is the client's return, and
  -- the other two map to their own terminal class.
  terminal := CASE NEW.pathway
    WHEN 'recycled' THEN 'recycled'
    WHEN 'destroyed' THEN 'destroyed'
    WHEN 'resold' THEN 'sold'
    WHEN 'returned_to_client' THEN 'returned'
    WHEN 'transferred_to_partner' THEN 'returned'
    ELSE NULL
  END;
  v_reason := CASE NEW.pathway
    WHEN 'transferred_to_partner' THEN
      left('e-waste transferred to partner '
           || coalesce(NEW.partner_name, 'unnamed')
           || coalesce(' (certificate ' || NEW.certificate_reference || ')', ''), 500)
    ELSE 'e-waste disposition ' || NEW.pathway
  END;
  IF terminal IS NULL THEN
    RETURN NEW;
  END IF;

  IF v_line.serial_id IS NOT NULL THEN
    SELECT custody_class INTO current_class
      FROM stock_serials
     WHERE id = v_line.serial_id AND org_id = NEW.org_id;
    IF current_class = 'ewaste_custody' THEN
      INSERT INTO inventory_class_movements (
        id, org_id, unit_kind, serial_id, from_class, to_class, reason,
        source_kind, source_id, created_by
      ) VALUES (
        gen_random_uuid(), NEW.org_id, 'serial', v_line.serial_id,
        current_class, terminal, v_reason,
        'ewaste_disposition', NEW.id, NEW.created_by
      );
    END IF;
  ELSIF v_line.device_asset_id IS NOT NULL THEN
    SELECT custody_class INTO current_class
      FROM device_assets
     WHERE id = v_line.device_asset_id AND org_id = NEW.org_id;
    IF current_class = 'ewaste_custody' THEN
      INSERT INTO inventory_class_movements (
        id, org_id, unit_kind, device_asset_id, from_class, to_class, reason,
        source_kind, source_id, created_by
      ) VALUES (
        gen_random_uuid(), NEW.org_id, 'asset', v_line.device_asset_id,
        current_class, terminal, v_reason,
        'ewaste_disposition', NEW.id, NEW.created_by
      );
    END IF;
  END IF;

  RETURN NEW;
END;
$$ LANGUAGE plpgsql;
