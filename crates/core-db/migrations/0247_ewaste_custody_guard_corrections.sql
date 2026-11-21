-- E-waste custody, review cycle five (migration 0247).
--
-- Three guard corrections on top of 0238, plus two notes that correct
-- comments in 0238 (its bytes are applied, so a note lives here instead).
--
--   * `ewaste_intake_guard` refuses an intake inserted closed. The balance and
--     destruction checks run when an open intake closes, so an insert that
--     arrived closed skipped them.
--
--   * `ewaste_disposition_guard` compares a lot disposition with the line's
--     remaining weight instead of adding the two weights, which could overflow
--     before the overdraw refusal.
--
--   * `ewaste_dispositions_append_only` no longer tells an operator to append a
--     reversing disposition. The ledger refuses a second disposition for a
--     unit and the table refuses negative weights, so there is no in-product
--     reversal. 0238's comment ("a wrong disposition is reversed by a new one")
--     is withdrawn with this note; a mis-recorded disposition needs a
--     support-issued correction, tracked outside this lane.
--
--   * 0238's comment called the `source_kind` CHECK swap metadata-only. The
--     replacement CHECK is validated against every existing ledger row and the
--     lock is held until commit. On an empty ledger the scan is cheap, which is
--     why it is acceptable here, but it is a scan and not a metadata change.
--
-- The register's keyset cursor walks `(disposed_at, created_at, id)` in
-- descending order. 0238 indexed the per-intake and per-pathway reads; this
-- index serves the org-wide page without a sort.
--
-- Additive-only: three `CREATE OR REPLACE FUNCTION` statements, one index, and
-- notes.

CREATE OR REPLACE FUNCTION ewaste_intake_guard() RETURNS trigger AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN
    IF NOT EXISTS (SELECT 1 FROM orgs o WHERE o.id = OLD.org_id) THEN
      RETURN OLD;
    END IF;
    RAISE EXCEPTION 'ewaste: an intake is a receipt of custody and cannot be deleted';
  END IF;

  IF TG_OP = 'UPDATE'
     AND NOT EXISTS (SELECT 1 FROM orgs o WHERE o.id = NEW.org_id) THEN
    RETURN NEW;
  END IF;

  IF TG_OP = 'INSERT' THEN
    -- A receipt starts open. The balance and destruction checks run on the
    -- open-to-closed transition, so an insert that arrives closed would skip
    -- them entirely.
    IF NEW.status <> 'open' THEN
      RAISE EXCEPTION
        'ewaste: an intake is recorded open and closed by the close operation';
    END IF;
    IF NEW.signature_document_id IS NOT NULL AND NOT EXISTS (
      SELECT 1 FROM documents d
       WHERE d.id = NEW.signature_document_id AND d.org_id = NEW.org_id
    ) THEN
      RAISE EXCEPTION
        'ewaste: signature document % is not in org % evidence',
        NEW.signature_document_id, NEW.org_id;
    END IF;
    RETURN NEW;
  END IF;

  -- A document that was deleted leaves its id behind (it is not a foreign key);
  -- only a newly named signature is validated.
  IF NEW.signature_document_id IS NOT NULL
     AND NEW.signature_document_id IS DISTINCT FROM OLD.signature_document_id
     AND NOT EXISTS (
       SELECT 1 FROM documents d
        WHERE d.id = NEW.signature_document_id AND d.org_id = NEW.org_id
     )
  THEN
    RAISE EXCEPTION
      'ewaste: signature document % is not in org % evidence',
      NEW.signature_document_id, NEW.org_id;
  END IF;

  IF OLD.client_id <> NEW.client_id
     OR OLD.received_at <> NEW.received_at
     OR OLD.received_by_name <> NEW.received_by_name
     OR OLD.data_destruction_required <> NEW.data_destruction_required
  THEN
    RAISE EXCEPTION
      'ewaste: client, receipt time, receiver and the destruction instruction are facts';
  END IF;

  IF OLD.status = 'closed' AND NEW.status = 'open' THEN
    RAISE EXCEPTION 'ewaste: a closed intake cannot be reopened; record a new intake';
  END IF;

  IF NEW.status = 'closed' AND OLD.status = 'open' THEN
    IF NOT EXISTS (
      SELECT 1 FROM ewaste_intake_lines
       WHERE intake_id = NEW.id AND org_id = NEW.org_id
    ) THEN
      RAISE EXCEPTION 'ewaste: an intake with no lines cannot close';
    END IF;
    IF EXISTS (
      SELECT 1
        FROM ewaste_intake_lines l
        LEFT JOIN ewaste_dispositions d
          ON d.line_id = l.id AND d.org_id = l.org_id
       WHERE l.intake_id = NEW.id AND l.org_id = NEW.org_id
       GROUP BY l.id, l.weight_g
      HAVING coalesce(sum(d.weight_g), 0) <> l.weight_g
    ) THEN
      RAISE EXCEPTION
        'ewaste: intake % still holds weight; every gram needs a pathway',
        NEW.id;
    END IF;
    IF EXISTS (
      SELECT 1 FROM ewaste_intake_lines
       WHERE intake_id = NEW.id AND org_id = NEW.org_id
         AND destruction_status = 'pending'
    ) THEN
      RAISE EXCEPTION
        'ewaste: intake % has data destruction pending', NEW.id;
    END IF;
    IF NEW.data_destruction_required AND EXISTS (
      SELECT 1 FROM ewaste_intake_lines
       WHERE intake_id = NEW.id AND org_id = NEW.org_id
         AND data_bearing
         AND destruction_status <> 'completed'
    ) THEN
      RAISE EXCEPTION
        'ewaste: intake % requires destruction of every data-bearing unit first',
        NEW.id;
    END IF;
    NEW.closed_at := coalesce(NEW.closed_at, now());
  END IF;

  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION ewaste_disposition_guard() RETURNS trigger AS $$
DECLARE
  v_line record;
  v_intake record;
  already_g bigint;
BEGIN
  SELECT id, intake_id, line_kind, weight_g, data_bearing, destruction_status,
         serial_id, device_asset_id
    INTO v_line
    FROM ewaste_intake_lines
   WHERE id = NEW.line_id AND org_id = NEW.org_id
   FOR UPDATE;

  IF NOT FOUND THEN
    RAISE EXCEPTION 'ewaste: line % not found in org %', NEW.line_id, NEW.org_id;
  END IF;
  IF v_line.intake_id <> NEW.intake_id THEN
    RAISE EXCEPTION 'ewaste: line % belongs to another intake', NEW.line_id;
  END IF;

  SELECT status, data_destruction_required INTO v_intake
    FROM ewaste_intakes
   WHERE id = NEW.intake_id AND org_id = NEW.org_id
   FOR SHARE;
  IF NOT FOUND THEN
    RAISE EXCEPTION 'ewaste: intake % not found in org %', NEW.intake_id, NEW.org_id;
  END IF;
  IF v_intake.status <> 'open' THEN
    RAISE EXCEPTION 'ewaste: intake % is closed and accepts no disposition',
      NEW.intake_id;
  END IF;

  IF v_line.destruction_status = 'pending' THEN
    RAISE EXCEPTION
      'ewaste: data destruction is pending for line %; dispose after it completes',
      NEW.line_id;
  END IF;
  IF v_line.data_bearing
     AND v_intake.data_destruction_required
     AND v_line.destruction_status <> 'completed'
  THEN
    RAISE EXCEPTION
      'ewaste: line % requires completed data destruction before disposition',
      NEW.line_id;
  END IF;

  IF NEW.document_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM documents d
     WHERE d.id = NEW.document_id AND d.org_id = NEW.org_id
  ) THEN
    RAISE EXCEPTION
      'ewaste: document % is not in org % evidence', NEW.document_id, NEW.org_id;
  END IF;

  SELECT coalesce(sum(weight_g), 0) INTO already_g
    FROM ewaste_dispositions
   WHERE line_id = NEW.line_id AND org_id = NEW.org_id;

  -- A line that names a physical unit (a serial, or an asset without one)
  -- leaves in one disposition at its full weight. A lot without a unit can be
  -- split across pathways.
  IF v_line.serial_id IS NOT NULL OR v_line.device_asset_id IS NOT NULL
     OR v_line.line_kind = 'serial'
  THEN
    IF already_g > 0 THEN
      RAISE EXCEPTION
        'ewaste: the unit on line % is disposed once', NEW.line_id;
    END IF;
    IF NEW.weight_g <> v_line.weight_g THEN
      RAISE EXCEPTION
        'ewaste: a unit leaves at its full weight (% g, got % g)',
        v_line.weight_g, NEW.weight_g;
    END IF;
  ELSE
    -- Compare against what is left rather than adding the two weights, which
    -- can overflow before the guard can refuse an overdraw.
    IF NEW.weight_g > v_line.weight_g - already_g THEN
      RAISE EXCEPTION
        'ewaste: disposition of % g overdraws line % (% of % g already disposed)',
        NEW.weight_g, NEW.line_id, already_g, v_line.weight_g;
    END IF;
  END IF;

  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION ewaste_dispositions_append_only() RETURNS trigger AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN
    IF NOT EXISTS (SELECT 1 FROM orgs o WHERE o.id = OLD.org_id) THEN
      RETURN OLD;
    END IF;
    RAISE EXCEPTION
      'ewaste: a disposition is a record; it cannot be edited or deleted';
  END IF;

  IF NEW.created_by IS NULL
     AND (to_jsonb(NEW) - 'created_by') = (to_jsonb(OLD) - 'created_by') THEN
    RETURN NEW;
  END IF;

  RAISE EXCEPTION
    'ewaste: a disposition is a record; it cannot be edited or deleted';
END;
$$ LANGUAGE plpgsql;

CREATE INDEX idx_ewaste_dispositions_register
  ON ewaste_dispositions (org_id, disposed_at DESC, created_at DESC, id DESC);
