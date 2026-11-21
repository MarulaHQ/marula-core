-- E-waste custody (ADR 0035 decisions 1 and 5; migration 0238).
--
-- Client e-waste is custody, not stock. A received device belongs to the
-- client, sits in the organisation's care, and leaves through a recorded
-- pathway. Nothing here writes `stock_levels`, `stock_movements`, or a
-- valuation: an intake changes owned quantity by zero, and the only unit
-- tables this migration touches are the class-movement ledger and the two
-- projections it already governs.
--
-- Four tables, in the order the work happens:
--
--   * `ewaste_intakes` — the receipt of custody. Client, site, ticket,
--     received at and by, the signature document, the data-destruction
--     switch, and hazard flags. An intake is OPEN until every gram has a
--     pathway; closing it is the only status change it has.
--
--   * `ewaste_intake_lines` — one serialized unit or one lot. A serial line
--     names a `stock_serial` (quantity one); a lot line is its own identity
--     plus a quantity, which is exactly how ADR 0035 §1 describes a lot. It
--     also carries the e-waste category, the line's total weight in integer
--     grams, the data-bearing flag, and the optional client asset it came
--     from.
--
--   * `ewaste_dispositions` — the way out: recycled, destroyed, resold,
--     returned to the client, or transferred to a partner. Partner, date,
--     weight, certificate reference, and an attached document. The
--     certificate is required where the pathway has a downstream custodian;
--     the attached document is either a reference or a document row.
--
--   * `ewaste_events` — the append-only custody ledger: intake, store,
--     move, handoff, disposition. A correction is a new event. Intake and
--     disposition events are written by the records that produce them, not
--     by a caller.
--
-- Guards the schema enforces, because a concurrent writer can bypass any
-- check the API makes:
--
--   * a disposition is refused while the line's required data destruction is
--     pending (destruction state lives on the line and is completed once, by
--     the custodian);
--   * a serialized unit is disposed of exactly once, in one pathway, at its
--     full weight (one unit, one disposition — the line cannot be split);
--   * a lot cannot be over-disposed: the sum of its dispositions may not
--     exceed the line weight, and the line row is locked while the sum is
--     checked;
--   * an intake cannot close with any weight remaining, cannot close with a
--     pending destruction, and cannot close empty;
--   * a line cannot be added to a closed intake, and the identity fields of a
--     disposed line are facts (no update after the first disposition);
--   * a client asset received into custody retires through
--     `inventory_class_movements` in the same statement that creates the
--     line, so a race cannot leave the client's fleet claiming a unit the
--     bench is holding. A terminal disposition moves the unit out of
--     `ewaste_custody` into `recycled`, `destroyed`, `sold`, or `returned`.
--
-- Vocabulary: `inventory_class_movements.source_kind` gains `ewaste_intake`,
-- additively (the 0054/0078 CHECK-widening pattern, and the extension 0232's
-- header sanctions). The intake is the document that retires the unit and
-- `ewaste_disposition` already exists for the way out, so the ledger can now
-- name both ends of an e-waste unit's life.
--
-- Non-purpose: no lot class movement. A lot never has a custody class of its
-- own — it exists inside one intake — so widening `unit_kind` to `lot` would
-- add state nobody reads. `UnitReference::Lot` in the domain stays available
-- for the day a lot does need its own class.
--
-- Additive-only: new tables, new functions, new triggers, and one CHECK
-- widened to a superset. No existing column, row, or index is rewritten. The
-- one lock on an existing table is the CHECK swap on
-- `inventory_class_movements` (ACCESS EXCLUSIVE for two statements); that table
-- is created by 0232 with no rows on a fresh install, and at MSP sizes the swap
-- is a metadata change, not a scan. Every index below is on a table this
-- migration creates, so none of them takes a lock on a hot table.
--
-- Tenant erasure (core-db/src/erasure.rs). One `DELETE FROM orgs` must cascade
-- through every org-scoped row, so three things are true by construction, and
-- they mirror the fix migration 0232's ledger guard carries:
--
--   * The append-only guards let a DELETE pass only when the parent org row is
--     already gone, and an UPDATE pass only when its sole effect is a nullable
--     actor column cleared by `ON DELETE SET NULL`. Everything else raises.
--   * The intake guard's DELETE branch uses the same org-gone exception.
--   * Only intra-e-waste children are foreign keys (`ON DELETE CASCADE`), so
--     they cannot outlive their intake. The ids that name another org-scoped
--     row — client, serial, device asset, signature, destruction and disposal
--     document — are plain uuids with a write-time org check, because a
--     RESTRICT reference blocks the cascade and a CASCADE would erase the
--     history the ledger exists to keep.

-- --- ledger vocabulary (0232 CHECK, widened additively) ----------------------

ALTER TABLE inventory_class_movements
  DROP CONSTRAINT inventory_class_movements_source_kind_check;
ALTER TABLE inventory_class_movements
  ADD CONSTRAINT inventory_class_movements_source_kind_check
  CHECK (source_kind IN ('purchase_receipt_line', 'stock_movement',
                         'device_asset', 'client_loan', 'tool_checkout',
                         'credit_line', 'ewaste_intake', 'ewaste_disposition',
                         'manual'));

-- --- intake -----------------------------------------------------------------

CREATE TABLE ewaste_intakes (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- Custody is held for a client; there is no org-owned e-waste intake. The
  -- client id is a plain uuid, not a foreign key: the write path proves the
  -- client is in the org, and the receipt survives a client deletion rather
  -- than blocking the tenant-erasure cascade.
  client_id uuid NOT NULL,
  site_id uuid REFERENCES sites(id) ON DELETE SET NULL,
  ticket_id uuid REFERENCES tickets(id) ON DELETE SET NULL,
  -- Operator reference from the receiving docket ("EW-2026-041"). A label,
  -- not an identity; indexed for lookup.
  reference text CHECK (reference IS NULL OR char_length(btrim(reference)) BETWEEN 1 AND 100),
  received_at timestamptz NOT NULL,
  received_by_name text NOT NULL
    CHECK (char_length(btrim(received_by_name)) BETWEEN 1 AND 200),
  received_by_user_id uuid REFERENCES users(id) ON DELETE SET NULL,
  -- The signed intake sheet, when the client signed one, and its free-text
  -- reference for a paper docket. Either is the evidence that they let go. The
  -- document id is a plain uuid, validated against the org when it is written.
  signature_document_id uuid,
  document_reference text
    CHECK (document_reference IS NULL OR char_length(document_reference) <= 500),
  -- The client's instruction: data must be destroyed before disposal.
  data_destruction_required boolean NOT NULL DEFAULT false,
  -- A closed vocabulary so a flag is queryable, not a free-text sentence.
  hazard_flags text[] NOT NULL DEFAULT '{}'
    CHECK (hazard_flags <@ ARRAY['battery', 'cracked', 'liquid_damage',
                                 'biohazard', 'loose_media', 'pressurised',
                                 'unlabeled']::text[]),
  notes text CHECK (notes IS NULL OR char_length(notes) <= 4000),
  status text NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'closed')),
  closed_at timestamptz,
  closed_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT ewaste_intakes_closed_shape
    CHECK ((status = 'closed') = (closed_at IS NOT NULL))
);

-- The docket reference is a label, not an identity: the same number can
-- legitimately repeat across sites and years, and the intake id is what the
-- API and the ledger cite. Indexed for lookup, not unique.
CREATE INDEX idx_ewaste_intakes_reference
  ON ewaste_intakes (org_id, reference)
  WHERE reference IS NOT NULL;
CREATE INDEX idx_ewaste_intakes_org_status
  ON ewaste_intakes (org_id, status, received_at DESC);
CREATE INDEX idx_ewaste_intakes_client
  ON ewaste_intakes (org_id, client_id, received_at DESC);

-- --- lines ------------------------------------------------------------------

CREATE TABLE ewaste_intake_lines (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- An intra-e-waste child: it cannot outlive its intake, so the tenant
  -- erasure cascade reaches it. A direct line delete is refused by the guard.
  intake_id uuid NOT NULL REFERENCES ewaste_intakes(id) ON DELETE CASCADE,
  -- serial: one physical unit, quantity one. lot: this row IS the identity,
  -- with a quantity and an optional operator label.
  line_kind text NOT NULL CHECK (line_kind IN ('serial', 'lot')),
  -- The unit ids are plain uuids: RESTRICT blocks the org cascade, and CASCADE
  -- would erase a line's history when a unit is deleted. The line guard and the
  -- retirement trigger prove both are in this org when the line is written.
  serial_id uuid,
  device_asset_id uuid,
  lot_label text CHECK (lot_label IS NULL OR char_length(lot_label) <= 200),
  category text NOT NULL CHECK (char_length(btrim(category)) BETWEEN 1 AND 120),
  -- The line's TOTAL weight in integer grams. Dispositions allocate against
  -- it, which is what makes the balance exact: grams never pass through a
  -- float, and a line's total cannot be inferred wrongly from a per-unit
  -- figure.
  weight_g bigint NOT NULL CHECK (weight_g > 0),
  quantity integer NOT NULL DEFAULT 1 CHECK (quantity >= 1),
  -- Does the unit hold storage? A data-bearing line is the one a destruction
  -- requirement can block.
  data_bearing boolean NOT NULL DEFAULT false,
  destruction_status text NOT NULL DEFAULT 'not_required'
    CHECK (destruction_status IN ('not_required', 'pending', 'completed')),
  destruction_completed_at timestamptz,
  destruction_completed_by uuid REFERENCES users(id) ON DELETE SET NULL,
  destruction_document_id uuid,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT ewaste_intake_lines_identity CHECK (
    (line_kind = 'serial' AND serial_id IS NOT NULL AND quantity = 1)
    OR (line_kind = 'lot' AND serial_id IS NULL)
  ),
  -- A line that names one physical asset is one unit, whatever its kind.
  CONSTRAINT ewaste_intake_lines_unit_quantity
    CHECK (device_asset_id IS NULL OR quantity = 1),
  -- A non-data-bearing line cannot claim a destruction state.
  CONSTRAINT ewaste_intake_lines_destruction_data
    CHECK (data_bearing OR destruction_status = 'not_required'),
  -- Completing destruction is one act: the timestamp and the actor arrive
  -- together, and a pending line has neither.
  CONSTRAINT ewaste_intake_lines_destruction_shape CHECK (
    (destruction_status = 'completed') = (destruction_completed_at IS NOT NULL)
  )
);

CREATE INDEX idx_ewaste_intake_lines_intake
  ON ewaste_intake_lines (org_id, intake_id);
CREATE INDEX idx_ewaste_intake_lines_serial
  ON ewaste_intake_lines (org_id, serial_id)
  WHERE serial_id IS NOT NULL;
CREATE INDEX idx_ewaste_intake_lines_asset
  ON ewaste_intake_lines (org_id, device_asset_id)
  WHERE device_asset_id IS NOT NULL;

-- --- dispositions -----------------------------------------------------------

CREATE TABLE ewaste_dispositions (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  intake_id uuid NOT NULL REFERENCES ewaste_intakes(id) ON DELETE CASCADE,
  line_id uuid NOT NULL REFERENCES ewaste_intake_lines(id) ON DELETE CASCADE,
  pathway text NOT NULL CHECK (pathway IN (
    'recycled', 'destroyed', 'resold', 'returned_to_client',
    'transferred_to_partner')),
  -- A recycler or a partner is named; a resale buyer is useful but not
  -- required, and a return goes to the client already on the intake.
  partner_name text
    CHECK (partner_name IS NULL OR char_length(btrim(partner_name)) BETWEEN 1 AND 200),
  disposed_at date NOT NULL,
  weight_g bigint NOT NULL CHECK (weight_g > 0),
  -- The certificate number an auditor reads off the paper. The attached
  -- document is the scan of that paper; the id is validated against the org
  -- when the disposition is written.
  certificate_reference text
    CHECK (certificate_reference IS NULL
           OR char_length(btrim(certificate_reference)) BETWEEN 1 AND 300),
  document_id uuid,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT ewaste_dispositions_partner CHECK (
    pathway NOT IN ('recycled', 'transferred_to_partner')
    OR partner_name IS NOT NULL
  ),
  -- Where a downstream custodian takes the goods, the certificate is the
  -- evidence: either its number or the document. Resale and return to the
  -- client are not downstream custody, so they need neither.
  CONSTRAINT ewaste_dispositions_certificate CHECK (
    pathway NOT IN ('recycled', 'destroyed', 'transferred_to_partner')
    OR certificate_reference IS NOT NULL
    OR document_id IS NOT NULL
  )
);

CREATE INDEX idx_ewaste_dispositions_intake
  ON ewaste_dispositions (org_id, intake_id, disposed_at DESC);
CREATE INDEX idx_ewaste_dispositions_line
  ON ewaste_dispositions (org_id, line_id);
CREATE INDEX idx_ewaste_dispositions_pathway
  ON ewaste_dispositions (org_id, pathway, disposed_at DESC);

-- --- custody event ledger ---------------------------------------------------

CREATE TABLE ewaste_events (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  intake_id uuid NOT NULL REFERENCES ewaste_intakes(id) ON DELETE CASCADE,
  -- An intake-wide movement (store, handoff of the whole consignment) has no
  -- line; a per-unit event names one.
  line_id uuid REFERENCES ewaste_intake_lines(id) ON DELETE CASCADE,
  event_kind text NOT NULL CHECK (event_kind IN (
    'intake', 'store', 'move', 'handoff', 'disposition')),
  occurred_at timestamptz NOT NULL DEFAULT now(),
  -- Where the goods are now, in the vocabulary the bench uses.
  storage_class text CHECK (storage_class IS NULL OR storage_class IN (
    'quarantine', 'secure_cage', 'warehouse', 'van', 'client_site',
    'partner_site')),
  location_note text CHECK (location_note IS NULL OR char_length(location_note) <= 300),
  partner_name text
    CHECK (partner_name IS NULL OR char_length(btrim(partner_name)) BETWEEN 1 AND 200),
  -- Set by the disposition that produced this event, never by a caller.
  disposition_id uuid REFERENCES ewaste_dispositions(id) ON DELETE CASCADE,
  actor_user_id uuid REFERENCES users(id) ON DELETE SET NULL,
  notes text CHECK (notes IS NULL OR char_length(notes) <= 2000),
  created_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT ewaste_events_disposition_shape
    CHECK ((event_kind = 'disposition') = (disposition_id IS NOT NULL)),
  CONSTRAINT ewaste_events_storage_shape CHECK (
    event_kind NOT IN ('store', 'move', 'handoff')
    OR storage_class IS NOT NULL
  ),
  CONSTRAINT ewaste_events_handoff_shape
    CHECK (event_kind <> 'handoff' OR partner_name IS NOT NULL)
);

CREATE INDEX idx_ewaste_events_intake
  ON ewaste_events (org_id, intake_id, occurred_at DESC);

-- --- row-level security -----------------------------------------------------

ALTER TABLE ewaste_intakes ENABLE ROW LEVEL SECURITY;
CREATE POLICY ewaste_intakes_org ON ewaste_intakes
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE ewaste_intake_lines ENABLE ROW LEVEL SECURITY;
CREATE POLICY ewaste_intake_lines_org ON ewaste_intake_lines
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE ewaste_dispositions ENABLE ROW LEVEL SECURITY;
CREATE POLICY ewaste_dispositions_org ON ewaste_dispositions
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE ewaste_events ENABLE ROW LEVEL SECURITY;
CREATE POLICY ewaste_events_org ON ewaste_events
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- --- append-only ledger -----------------------------------------------------

-- The custody ledger and the disposition record are append-only. A correction
-- is a new event; a wrong disposition is reversed by a new one, not by editing
-- history.
--
-- Two removals are not edits, and each table's guard admits only its own
-- nullable actor columns:
--
--   * The tenant erasure contract deletes the `orgs` row and lets the cascade
--     remove every org-scoped row. The parent org is already gone when the
--     child row is deleted, so a DELETE that cannot see its org is the cascade.
--   * `ON DELETE SET NULL` on an actor column is a user hard-delete, not a
--     history edit. Every other column must be unchanged.
--
-- A direct UPDATE or DELETE by any writer still raises. This mirrors the
-- ledger guard migration 0232 carries.

CREATE FUNCTION ewaste_intake_lines_append_only() RETURNS trigger AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN
    IF NOT EXISTS (SELECT 1 FROM orgs o WHERE o.id = OLD.org_id) THEN
      RETURN OLD;
    END IF;
    RAISE EXCEPTION
      'ewaste: an intake line is a fact; append a new line instead';
  END IF;

  RAISE EXCEPTION 'ewaste: an intake line is a fact; append a new line instead';
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION ewaste_dispositions_append_only() RETURNS trigger AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN
    IF NOT EXISTS (SELECT 1 FROM orgs o WHERE o.id = OLD.org_id) THEN
      RETURN OLD;
    END IF;
    RAISE EXCEPTION
      'ewaste: a disposition is a record; append a reversing disposition instead';
  END IF;

  IF NEW.created_by IS NULL
     AND (to_jsonb(NEW) - 'created_by') = (to_jsonb(OLD) - 'created_by') THEN
    RETURN NEW;
  END IF;

  RAISE EXCEPTION
    'ewaste: a disposition is a record; append a reversing disposition instead';
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION ewaste_events_append_only() RETURNS trigger AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN
    IF NOT EXISTS (SELECT 1 FROM orgs o WHERE o.id = OLD.org_id) THEN
      RETURN OLD;
    END IF;
    RAISE EXCEPTION 'ewaste: the custody ledger is append-only';
  END IF;

  IF NEW.actor_user_id IS NULL
     AND (to_jsonb(NEW) - 'actor_user_id') = (to_jsonb(OLD) - 'actor_user_id') THEN
    RETURN NEW;
  END IF;

  RAISE EXCEPTION 'ewaste: the custody ledger is append-only';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER ewaste_dispositions_append_only_guard
  BEFORE UPDATE OR DELETE ON ewaste_dispositions
  FOR EACH ROW EXECUTE FUNCTION ewaste_dispositions_append_only();

CREATE TRIGGER ewaste_events_append_only_guard
  BEFORE UPDATE OR DELETE ON ewaste_events
  FOR EACH ROW EXECUTE FUNCTION ewaste_events_append_only();

CREATE TRIGGER ewaste_intake_lines_append_only_guard
  BEFORE DELETE ON ewaste_intake_lines
  FOR EACH ROW EXECUTE FUNCTION ewaste_intake_lines_append_only();

-- --- line guards ------------------------------------------------------------

-- Runs on INSERT and UPDATE. On insert: the intake must be open, the line
-- must not contradict the intake's destruction instruction, and the client
-- asset it names must be the intake's own client's. On update: identity and
-- weight are facts once a gram has been disposed, and a closed intake
-- accepts no line edit at all.
--
-- Two kinds of update are not edits and pass before the intake is read: the
-- tenant-erasure cascade (the org is gone) and `ON DELETE SET NULL` clearing
-- `created_by` or `destruction_completed_by` when a user is hard-deleted.
CREATE FUNCTION ewaste_intake_line_guard() RETURNS trigger AS $$
DECLARE
  intake_status text;
  intake_client uuid;
  destruction_required boolean;
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
        OR OLD.data_bearing IS DISTINCT FROM NEW.data_bearing)
       AND EXISTS (
         SELECT 1 FROM ewaste_dispositions d
          WHERE d.line_id = NEW.id AND d.org_id = NEW.org_id
       )
    THEN
      RAISE EXCEPTION
        'ewaste: line % already has a disposition; its identity and weight are facts',
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

  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER ewaste_intake_lines_guard
  BEFORE INSERT OR UPDATE ON ewaste_intake_lines
  FOR EACH ROW EXECUTE FUNCTION ewaste_intake_line_guard();

-- --- intake guards ----------------------------------------------------------

-- Refuses deleting a receipt of custody, refuses rewriting the facts of the
-- handover, proves the signature document is the org's own, and makes closing
-- a balance check rather than a status flag.
--
-- A DELETE passes only when the parent org row is already gone (tenant
-- erasure's cascade); an UPDATE passes untouched when the same cascade is
-- running, because a site or ticket deletion sets its id to NULL on the way
-- through.
CREATE FUNCTION ewaste_intake_guard() RETURNS trigger AS $$
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

CREATE TRIGGER ewaste_intakes_guard
  BEFORE INSERT OR UPDATE OR DELETE ON ewaste_intakes
  FOR EACH ROW EXECUTE FUNCTION ewaste_intake_guard();

-- --- disposition guards -----------------------------------------------------

-- Locks the line, checks the destruction state, and refuses a disposition
-- that overdraws the line or splits a serialized unit across pathways. The
-- row lock is what makes two concurrent dispositions serialize instead of
-- both reading the same sum.
CREATE FUNCTION ewaste_disposition_guard() RETURNS trigger AS $$
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
    IF already_g + NEW.weight_g > v_line.weight_g THEN
      RAISE EXCEPTION
        'ewaste: disposition of % g overdraws line % (% of % g already disposed)',
        NEW.weight_g, NEW.line_id, already_g, v_line.weight_g;
    END IF;
  END IF;

  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER ewaste_dispositions_guard
  BEFORE INSERT ON ewaste_dispositions
  FOR EACH ROW EXECUTE FUNCTION ewaste_disposition_guard();

-- Every disposition is a custody event, and the unit it names leaves the live
-- class when the last gram of the line is disposed. Both are consequences of
-- the disposition row, so a writer cannot record one without the other.
CREATE FUNCTION ewaste_disposition_apply() RETURNS trigger AS $$
DECLARE
  v_line record;
  disposed_g bigint;
  terminal text;
  current_class text;
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

  terminal := CASE NEW.pathway
    WHEN 'recycled' THEN 'recycled'
    WHEN 'destroyed' THEN 'destroyed'
    WHEN 'resold' THEN 'sold'
    WHEN 'returned_to_client' THEN 'returned'
    ELSE NULL
  END;
  -- A transfer to a partner keeps the unit in custody vocabulary; the handoff
  -- event is the record that it left the bench.
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
        current_class, terminal,
        'e-waste disposition ' || NEW.pathway,
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
        current_class, terminal,
        'e-waste disposition ' || NEW.pathway,
        'ewaste_disposition', NEW.id, NEW.created_by
      );
    END IF;
  END IF;

  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER ewaste_dispositions_apply
  AFTER INSERT ON ewaste_dispositions
  FOR EACH ROW EXECUTE FUNCTION ewaste_disposition_apply();

-- --- custody events ---------------------------------------------------------

-- The intake's arrival event is produced by the intake, not written by a
-- caller. Store, move and handoff are the caller's to record.
CREATE FUNCTION ewaste_intake_event_on_arrival() RETURNS trigger AS $$
BEGIN
  INSERT INTO ewaste_events (
    id, org_id, intake_id, line_id, event_kind, occurred_at, storage_class,
    partner_name, actor_user_id, notes
  ) VALUES (
    gen_random_uuid(), NEW.org_id, NEW.id, NULL, 'intake', NEW.received_at,
    NULL, NULL, NEW.created_by, NEW.reference
  );
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER ewaste_intakes_arrival_event
  AFTER INSERT ON ewaste_intakes
  FOR EACH ROW EXECUTE FUNCTION ewaste_intake_event_on_arrival();

CREATE FUNCTION ewaste_event_guard() RETURNS trigger AS $$
DECLARE
  intake_status text;
BEGIN
  -- Intake and disposition events belong to the row that produced them. A
  -- direct insert would be a second, uncaused event; `pg_trigger_depth` is
  -- the check, not a setting a caller could provide for itself.
  IF NEW.event_kind IN ('intake', 'disposition') AND pg_trigger_depth() <= 1 THEN
    RAISE EXCEPTION
      'ewaste: % events are written by the record that produces them',
      NEW.event_kind;
  END IF;

  IF NEW.event_kind IN ('store', 'move', 'handoff') THEN
    -- SHARE blocks a concurrent close (which takes UPDATE) until this event's
    -- transaction commits, so an event cannot land on an intake that is
    -- closing in another writer.
    SELECT status INTO intake_status
      FROM ewaste_intakes
     WHERE id = NEW.intake_id AND org_id = NEW.org_id
     FOR SHARE;
    IF intake_status IS NULL OR intake_status <> 'open' THEN
      RAISE EXCEPTION 'ewaste: intake % is closed and takes no custody event',
        NEW.intake_id;
    END IF;
    IF NEW.line_id IS NOT NULL AND NOT EXISTS (
      SELECT 1 FROM ewaste_intake_lines
       WHERE id = NEW.line_id AND org_id = NEW.org_id
         AND intake_id = NEW.intake_id
    ) THEN
      RAISE EXCEPTION 'ewaste: line % is not on intake %', NEW.line_id, NEW.intake_id;
    END IF;
  END IF;

  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER ewaste_events_guard
  BEFORE INSERT ON ewaste_events
  FOR EACH ROW EXECUTE FUNCTION ewaste_event_guard();

-- --- client-asset retirement ------------------------------------------------

-- A client asset received into custody retires from the client's fleet in the
-- same statement that records the line. A unit already in `ewaste_custody` is
-- a re-intake and needs no second movement. Anything that is not the client's
-- goods is refused: owned stock and vendor rentals have their own disposal
-- paths, and e-waste must never become a quiet way to remove owned quantity.
CREATE FUNCTION ewaste_intake_line_retire_unit() RETURNS trigger AS $$
DECLARE
  intake_client uuid;
  intake_ref text;
  current_class text;
  unit_serial uuid;
  unit_asset uuid;
  unit_asset_client uuid;
  asset_serial uuid;
BEGIN
  SELECT client_id, coalesce(reference, id::text)
    INTO intake_client, intake_ref
    FROM ewaste_intakes
   WHERE id = NEW.intake_id AND org_id = NEW.org_id;

  IF NEW.serial_id IS NOT NULL AND NEW.device_asset_id IS NOT NULL THEN
    SELECT serial_id INTO asset_serial
      FROM device_assets
     WHERE id = NEW.device_asset_id AND org_id = NEW.org_id;
    IF asset_serial IS DISTINCT FROM NEW.serial_id THEN
      RAISE EXCEPTION
        'ewaste: line % names serial % and asset % that are not one unit',
        NEW.id, NEW.serial_id, NEW.device_asset_id;
    END IF;
  END IF;

  IF NEW.serial_id IS NOT NULL THEN
    SELECT custody_class, device_asset_id, client_id
      INTO current_class, unit_asset, unit_asset_client
      FROM stock_serials
     WHERE id = NEW.serial_id AND org_id = NEW.org_id;
    unit_serial := NEW.serial_id;
    -- A serial that names a client is that client's device. Receiving
    -- another client's unit would move it out of their fleet.
    IF unit_asset_client IS NOT NULL AND unit_asset_client <> intake_client THEN
      RAISE EXCEPTION
        'ewaste: serial % belongs to another client', NEW.serial_id;
    END IF;
  ELSIF NEW.device_asset_id IS NOT NULL THEN
    SELECT custody_class, serial_id INTO current_class, unit_serial
      FROM device_assets
     WHERE id = NEW.device_asset_id AND org_id = NEW.org_id;
    IF NOT FOUND THEN
      RAISE EXCEPTION 'ewaste: asset % not found in org %',
        NEW.device_asset_id, NEW.org_id;
    END IF;
    -- The line guard has already proved the asset belongs to this client.
    unit_asset := NEW.device_asset_id;
  ELSE
    RETURN NEW;
  END IF;

  IF current_class IS NULL THEN
    RAISE EXCEPTION 'ewaste: unit on line % not found in org %', NEW.id, NEW.org_id;
  END IF;
  IF current_class IN ('sale_stock', 'internal_fleet', 'rental_in', 'rental_out', 'tools') THEN
    RAISE EXCEPTION
      'ewaste: unit on line % is % (organisation goods); e-waste custody takes client goods only',
      NEW.id, current_class;
  END IF;
  IF current_class IN ('sold', 'recycled', 'destroyed', 'returned') THEN
    RAISE EXCEPTION 'ewaste: unit on line % is already disposed (%)',
      NEW.id, current_class;
  END IF;

  IF current_class = 'ewaste_custody' THEN
    RETURN NEW;
  END IF;

  IF unit_serial IS NOT NULL THEN
    INSERT INTO inventory_class_movements (
      id, org_id, unit_kind, serial_id, from_class, to_class, reason,
      source_kind, source_id, created_by
    ) VALUES (
      gen_random_uuid(), NEW.org_id, 'serial', unit_serial, current_class,
      'ewaste_custody',
      'e-waste intake ' || intake_ref || ' entered custody',
      'ewaste_intake', NEW.intake_id, NEW.created_by
    );
  ELSE
    INSERT INTO inventory_class_movements (
      id, org_id, unit_kind, device_asset_id, from_class, to_class, reason,
      source_kind, source_id, created_by
    ) VALUES (
      gen_random_uuid(), NEW.org_id, 'asset', unit_asset, current_class,
      'ewaste_custody',
      'e-waste intake ' || intake_ref || ' entered custody',
      'ewaste_intake', NEW.intake_id, NEW.created_by
    );
  END IF;

  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER ewaste_intake_lines_retire_unit
  AFTER INSERT ON ewaste_intake_lines
  FOR EACH ROW EXECUTE FUNCTION ewaste_intake_line_retire_unit();
