-- Credit notes become documents with lines.
--
-- A credit is applied against an invoice as an amount today, so a correction
-- cannot say which line it corrects or what happened to the goods. This table
-- gives a credit one row per corrected line, each naming its source (a cost
-- line, an expense, a rental charge, or a contract line) and, when the
-- correction gives a physical unit back, a disposition.
--
-- Append-only: a credit is issued once. A wrong line is a second credit, never
-- an edit. Tenant erasure still passes, by the exemption every append-only
-- table from 0232 on carries.

CREATE TABLE credit_note_lines (
  id               uuid PRIMARY KEY,
  org_id           uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  credit_note_id   uuid NOT NULL REFERENCES credit_notes(id) ON DELETE CASCADE,
  -- What this line corrects. The pair is the same vocabulary invoice_lines
  -- already carries in migration 0234, so a credit and the charge it corrects
  -- name the same thing the same way.
  source_kind      text NOT NULL,
  source_id        uuid NOT NULL,
  -- The invoice line it credits, when the credit is line-level rather than
  -- against the document total.
  invoice_line_id  uuid REFERENCES invoice_lines(id) ON DELETE SET NULL,
  description      text NOT NULL,
  quantity_minutes bigint NOT NULL DEFAULT 0,
  amount_minor     bigint NOT NULL,
  base_amount_minor bigint,
  -- What happened to the physical unit this correction gives back. NULL for a
  -- service credit. The vocabulary is the return path's, so one correction
  -- reads the same as one return.
  disposition      text,
  -- Whether this correction gives a physical unit back. A service credit has
  -- no disposition; a physical credit must name one.
  physical         boolean NOT NULL DEFAULT false,
  position         integer NOT NULL,
  created_at       timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT credit_note_lines_source_kind
    CHECK (source_kind IN ('ticket_cost_line', 'ticket_expense', 'rental_charge', 'contract_line')),
  CONSTRAINT credit_note_lines_amount
    CHECK (amount_minor <> 0),
  CONSTRAINT credit_note_lines_quantity
    CHECK (quantity_minutes >= 0),
  CONSTRAINT credit_note_lines_disposition
    CHECK (disposition IS NULL
           OR disposition IN ('sale_stock', 'pool', 'refurbishment', 'ewaste')),
  CONSTRAINT credit_note_lines_physical
    CHECK (physical = (disposition IS NOT NULL))
);

CREATE INDEX credit_note_lines_note
  ON credit_note_lines (org_id, credit_note_id, position);
CREATE INDEX credit_note_lines_source
  ON credit_note_lines (org_id, source_kind, source_id);

ALTER TABLE credit_note_lines ENABLE ROW LEVEL SECURITY;
CREATE POLICY credit_note_lines_org ON credit_note_lines
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

CREATE OR REPLACE FUNCTION credit_note_lines_append_only() RETURNS trigger AS $$
BEGIN
  IF TG_OP = 'DELETE'
     AND NOT EXISTS (SELECT 1 FROM orgs o WHERE o.id = OLD.org_id) THEN
    RETURN OLD;
  END IF;
  RAISE EXCEPTION 'credit note lines are append-only: % rejected', TG_OP;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER credit_note_lines_no_update
  BEFORE UPDATE ON credit_note_lines
  FOR EACH ROW EXECUTE FUNCTION credit_note_lines_append_only();
CREATE TRIGGER credit_note_lines_no_delete
  BEFORE DELETE ON credit_note_lines
  FOR EACH ROW EXECUTE FUNCTION credit_note_lines_append_only();

-- A billable ticket expense is its own line kind. The fixed-fee unique index
-- keys on (contract, kind, period_start, description), so two expenses that
-- share a description no longer collide: the kind and the source tell them
-- apart.
ALTER TABLE invoice_lines DROP CONSTRAINT invoice_lines_kind_check;
ALTER TABLE invoice_lines
  ADD CONSTRAINT invoice_lines_kind_check
  CHECK (kind IN ('recurring_base','retainer_base','retainer_overage',
                  'time_materials','fixed_fee','per_unit_base','per_incident',
                  'block_sale','block_drawdown','cap_overage','minimum_trueup',
                  'change_order','sla_credit','escalator','project_release',
                  'rental','expense')) NOT VALID;
-- The validation scan runs in 0261, outside this transaction: this file holds
-- an ACCESS EXCLUSIVE lock on invoice_lines from the statement above until it
-- commits, and validating here would block reads for the length of the scan.

-- The invoice vocabulary gains two words. A ticket expense billed onto an
-- invoice is a distinct source from the cost line beside it, and a line the
-- contract itself produced (a recurring base, a fixed fee) names its contract
-- as `contract_line`, which is what the backfill below writes.
ALTER TABLE invoice_lines DROP CONSTRAINT invoice_lines_source_kind_check;
ALTER TABLE invoice_lines
  ADD CONSTRAINT invoice_lines_source_kind_check
  CHECK (source_kind IS NULL OR source_kind IN (
    'stock_movement', 'purchase_receipt_line', 'ticket_cost_line', 'ticket_expense',
    'contract_line', 'invoice_line', 'credit_line', 'acquisition_cost',
    'rental_charge', 'manual'));

-- A written invoice line always names its source from now on. The columns
-- exist since 0234 and only the rental path filled them; this backfills the
-- lines that already carry the information in `contract_id`, then makes the
-- rule explicit for every writer that follows.
UPDATE invoice_lines
   SET source_kind = 'contract_line', source_id = contract_id
 WHERE source_kind IS NULL;

-- A written invoice line names its source. Two shapes are tolerated on
-- INSERT because a rolling deploy runs this migration before the new release
-- starts: the previous release writes a sourceless line, and stamping it with
-- the contract keeps invoice creation working until it drains. An UPDATE is
-- always refused when it would take the source away, so nothing can strip a
-- line once it is written.
CREATE OR REPLACE FUNCTION invoice_lines_source_required() RETURNS trigger AS $$
BEGIN
  IF NEW.source_kind IS NULL AND NEW.source_id IS NULL THEN
    IF TG_OP = 'UPDATE' THEN
      RAISE EXCEPTION 'an invoice line cannot lose its source: invoice_line %',
        NEW.id
        USING ERRCODE = 'check_violation';
    END IF;
    NEW.source_kind := 'contract_line';
    NEW.source_id := NEW.contract_id;
  END IF;
  IF NEW.source_kind IS NULL OR NEW.source_id IS NULL THEN
    RAISE EXCEPTION 'an invoice line names its source: invoice_line %',
      NEW.id
      USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER invoice_lines_source_before_write
  BEFORE INSERT OR UPDATE ON invoice_lines
  FOR EACH ROW EXECUTE FUNCTION invoice_lines_source_required();
