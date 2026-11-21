-- Rentals: the owned pool, client leases that bill, and vendor rentals.
-- Migration 0237 follows the inventory identity, pair and money vocabulary in
-- 0232-0234 and the receipt routing in 0235.
--
-- Purpose (ADR 0035 decision 4, §7 of the custody plan): owned rental gear is
-- a distinct pool that moves in both directions; a lease to a client records an
-- agreement and produces charges that bill through the ordinary invoice run;
-- a rental from a vendor records the agreement, its expense and its return, and
-- never depreciates.
--
--   * `inventory_class_movements.condition_grade`, `data_wipe_ref`,
--     `data_wipe_at` — the two facts a pool move has to carry. Commission into
--     the pool and decommission out of it are class movements, so the grade and
--     the wipe evidence ride the movement that caused them instead of a second
--     record beside it. Both are nullable: a loan issue is a class movement too
--     and carries neither.
--
--   * `rental_charges` — one row per billable line a lease agreement produces.
--     The agreement is `client_loans` (ADR 0035 §4: leases reuse the loan
--     vocabulary, they do not get a fourth document). The charge cites the
--     agreement and the contract the run bills under, and the invoice line it
--     became. A billed charge is immutable: the amount is computed once, when
--     it is written, and corrections are credit notes, never a recomputation.
--
--   * `vendor_rentals` — the agreement under which the organisation rents a
--     unit from a vendor: terms, cycle, expense, deposit, and the return with
--     its condition, damage and loss. The asset stays a `device_assets` row
--     (`ownership = rented`, class `rental_in`); this table is its agreement,
--     not a second asset model.
--
-- Why `rental_charges` and not the invoice line alone: the line is the
-- document, the charge is the fact that will bill. A period that has not been
-- invoiced yet needs a record, and the unique index is what stops two runs from
-- billing it twice. The line cites its charge through the `source_kind` /
-- `source_id` pair migration 0234 already put on `invoice_lines`.
--
-- Vocabulary: `invoice_lines.kind` gains `rental` — the line kind the plan
-- names. It is deliberately NOT added to the period-dedupe indexes: one
-- contract can lease several units over one period, so period identity cannot
-- name a rental line. `rental_charges` owns that identity instead.
--
-- Tenant erasure: both new tables pass the contract in the lane contract. A
-- direct delete raises; `DELETE FROM orgs` succeeds because the guard first
-- asks whether the org row still exists. The invoice ids a charge carries have
-- no foreign key (an evidence id naming another org-scoped row keeps the value,
-- per the erasure rule), and every parent it does reference cascades.
--
-- Lock cost: two new tables and four new columns on an append-only ledger. Two
-- CHECK constraints land `NOT VALID` — the `rental_in` shape on `device_assets`
-- and the wipe shape on the class ledger — because they enforce every new write
-- without scanning rows that predate them, and a validation that refuses
-- historical rows must not fail the upgrade. The widened invoice-kind CHECK is
-- also `NOT VALID`: it is dropped and re-added here, and adding it validated
-- would scan a populated `invoice_lines` while this transaction still holds the
-- lock the header DDL took. Migration 0250 validates the invoice kind and the
-- wipe shape as its own step, where a scan takes SHARE UPDATE EXCLUSIVE and does
-- not block writes. The `rental_in` shape stays NOT VALID: refusing an upgrade
-- over a historical row would be worse than leaving it to police new writes.

-- --- what a pool move carries ------------------------------------------------

ALTER TABLE inventory_class_movements
  ADD COLUMN condition_grade text
    CHECK (condition_grade IS NULL OR condition_grade IN ('good', 'fair', 'damaged')),
  ADD COLUMN data_wipe_ref text
    CHECK (data_wipe_ref IS NULL OR (btrim(data_wipe_ref) <> '' AND char_length(data_wipe_ref) <= 200)),
  ADD COLUMN data_wipe_at timestamptz;

-- A wipe timestamp without the evidence it points at is a claim, not a record.
-- NOT VALID: the constraint enforces every row written from here on, while the
-- scan of existing rows waits for 0250, so the ledger keeps its shared lock for
-- a statement instead of a table pass.
ALTER TABLE inventory_class_movements
  ADD CONSTRAINT inventory_class_movements_wipe_shape
  CHECK (data_wipe_at IS NULL OR data_wipe_ref IS NOT NULL) NOT VALID;

-- --- rental charges ----------------------------------------------------------

CREATE TABLE rental_charges (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- The agreement (a lease is a client loan; ADR 0035 §4). A charge cannot
  -- outlive the agreement that produced it, so the child cascades.
  loan_id uuid NOT NULL REFERENCES client_loans(id) ON DELETE CASCADE,
  -- The billing anchor: the run evaluates contracts, so a charge that should
  -- bill names the one it bills under. NO ACTION, not RESTRICT: the org
  -- cascade deletes the contract and the charge in one statement, and a
  -- restriction checked mid-cascade would block tenant erasure.
  contract_id uuid NOT NULL REFERENCES contracts(id) ON DELETE NO ACTION,
  client_id uuid NOT NULL REFERENCES clients(id) ON DELETE NO ACTION,
  -- `rent` is a period of the lease; `damage` and `loss` are one-off charges
  -- raised when the unit comes back (or does not).
  kind text NOT NULL CHECK (kind IN ('rent', 'damage', 'loss')),
  -- The loan line a one-off charge names. Rent covers the agreement, so it
  -- carries no line: a lease bills whether or not one unit of it is out.
  source_line_id uuid,
  period_start date NOT NULL,
  period_end date NOT NULL,
  -- Proration is stored, not re-derived: the days billed and the days the
  -- period holds are what the amount was computed from.
  days_billed integer NOT NULL CHECK (days_billed > 0),
  period_days integer NOT NULL CHECK (period_days > 0),
  CONSTRAINT rental_charges_days_fit CHECK (days_billed <= period_days),
  CONSTRAINT rental_charges_period CHECK (period_end > period_start),
  -- The full cycle price the charge was prorated from, and what it came to.
  unit_amount_minor bigint NOT NULL CHECK (unit_amount_minor >= 0),
  amount_minor bigint NOT NULL CHECK (amount_minor >= 0),
  currency text NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
  -- FX snapshot (ADR 0002 lock moment: the charge is priced once). NULL when
  -- the charge currency is the organisation base.
  fx_rate_used text,
  fx_rate_date date,
  fx_source text,
  base_amount_minor bigint,
  description text NOT NULL CHECK (btrim(description) <> ''),
  status text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'billed')),
  -- The document the charge became. No foreign keys: these are evidence ids
  -- naming org-scoped rows, and the lane contract keeps the value without a
  -- constraint so history survives a parent delete and the org cascade still
  -- removes the charge.
  invoice_id uuid,
  invoice_line_id uuid,
  billed_at timestamptz,
  CONSTRAINT rental_charges_billed_shape CHECK (
    (status = 'pending' AND invoice_id IS NULL AND invoice_line_id IS NULL AND billed_at IS NULL)
    OR (status = 'billed' AND invoice_id IS NOT NULL AND invoice_line_id IS NOT NULL AND billed_at IS NOT NULL)
  ),
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);

-- One rent charge per agreement per period, ever. A re-run sees the row and
-- skips it; a concurrent re-run hits this index and rolls back.
CREATE UNIQUE INDEX uq_rental_charges_rent_period
  ON rental_charges (org_id, loan_id, period_start, period_end)
  WHERE kind = 'rent';
-- One damage and one loss charge per loan line, ever.
CREATE UNIQUE INDEX uq_rental_charges_line_kind
  ON rental_charges (org_id, loan_id, kind, source_line_id)
  WHERE kind IN ('damage', 'loss');
CREATE INDEX idx_rental_charges_loan
  ON rental_charges (org_id, loan_id, period_start DESC);
CREATE INDEX idx_rental_charges_contract
  ON rental_charges (org_id, contract_id, status);

ALTER TABLE rental_charges ENABLE ROW LEVEL SECURITY;
CREATE POLICY rental_charges_org ON rental_charges
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

-- A charge is a fact: it may move pending → billed exactly once, and nothing
-- else about it may change. The tenant-erasure cascade and the `created_by`
-- SET NULL pass, as everywhere else in this wave.
CREATE FUNCTION rental_charges_append_only() RETURNS trigger AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN
    IF NOT EXISTS (SELECT 1 FROM orgs o WHERE o.id = OLD.org_id) THEN
      RETURN OLD;
    END IF;
    RAISE EXCEPTION
      'rental_charges is append-only: a charge is billed once and never removed';
  END IF;

  IF NEW.created_by IS NULL
     AND (to_jsonb(NEW) - 'created_by') = (to_jsonb(OLD) - 'created_by') THEN
    RETURN NEW;
  END IF;

  -- The one legitimate write after insert: pending → billed, with the document
  -- it became. The four moving columns are excluded from the comparison and the
  -- two ids are required to be present; a NULL-blind equality would reject the
  -- very transition this guard exists to permit.
  IF OLD.status = 'pending'
     AND NEW.status = 'billed'
     AND NEW.invoice_id IS NOT NULL
     AND NEW.invoice_line_id IS NOT NULL
     AND NEW.billed_at IS NOT NULL
     AND (to_jsonb(NEW) - 'status' - 'billed_at' - 'invoice_id' - 'invoice_line_id')
         = (to_jsonb(OLD) - 'status' - 'billed_at' - 'invoice_id' - 'invoice_line_id') THEN
    RETURN NEW;
  END IF;

  RAISE EXCEPTION
    'rental_charges is append-only: a charge is billed once and never recomputed';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER rental_charges_append_only_guard
  BEFORE UPDATE OR DELETE ON rental_charges
  FOR EACH ROW EXECUTE FUNCTION rental_charges_append_only();

-- --- vendor rentals ----------------------------------------------------------

CREATE TABLE vendor_rentals (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- The vendor's unit. It is a `device_assets` row with `ownership = rented`
  -- and class `rental_in`; deleting the asset ends the agreement with it.
  device_asset_id uuid NOT NULL REFERENCES device_assets(id) ON DELETE CASCADE,
  vendor_id uuid REFERENCES vendors(id) ON DELETE SET NULL,
  -- The vendor's own name for the agreement, when the vendor is not in the
  -- directory. One of the two identifies the counterparty.
  vendor_name text CHECK (vendor_name IS NULL OR btrim(vendor_name) <> ''),
  agreement_ref text CHECK (agreement_ref IS NULL OR char_length(agreement_ref) <= 200),
  start_on date NOT NULL,
  -- NULL = open-ended.
  end_on date,
  CONSTRAINT vendor_rentals_dates CHECK (end_on IS NULL OR end_on >= start_on),
  -- The rental expense, per cycle. Integer minor units plus the ISO code the
  -- org signed the agreement in.
  cost_minor bigint NOT NULL CHECK (cost_minor >= 0),
  currency text NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
  cycle text NOT NULL CHECK (cycle IN ('monthly', 'quarterly', 'annual')),
  deposit_minor bigint CHECK (deposit_minor IS NULL OR deposit_minor >= 0),
  deposit_currency text CHECK (deposit_currency IS NULL OR deposit_currency ~ '^[A-Z]{3}$'),
  CONSTRAINT vendor_rentals_deposit_shape CHECK (
    (deposit_minor IS NULL) = (deposit_currency IS NULL)
    AND (deposit_minor IS NULL OR deposit_minor > 0)
  ),
  fx_rate_used text,
  fx_rate_date date,
  fx_source text,
  base_cost_minor bigint,
  status text NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'returned')),
  returned_at timestamptz,
  return_condition text CHECK (return_condition IS NULL OR return_condition IN ('good', 'fair', 'damaged')),
  damage_minor bigint CHECK (damage_minor IS NULL OR damage_minor >= 0),
  -- True when the unit never came back. A loss is not a return: the agreement
  -- closes, the charge stands, and the asset keeps its class because no
  -- terminal class in ADR 0035 names "lost".
  lost boolean NOT NULL DEFAULT false,
  loss_minor bigint CHECK (loss_minor IS NULL OR loss_minor >= 0),
  notes text CHECK (notes IS NULL OR char_length(notes) <= 2000),
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT vendor_rentals_return_shape CHECK (
    (status = 'open'
      AND returned_at IS NULL
      AND return_condition IS NULL
      AND damage_minor IS NULL
      AND NOT lost
      AND loss_minor IS NULL)
    OR (status = 'returned'
      AND returned_at IS NOT NULL
      AND ((lost AND return_condition IS NULL) OR (NOT lost AND return_condition IS NOT NULL)))
  )
);

-- One open agreement per unit: the vendor's unit cannot be promised to two
-- agreements at once, which is the inbound half of "a unit cannot be in two
-- pools".
CREATE UNIQUE INDEX uq_vendor_rentals_open_asset
  ON vendor_rentals (org_id, device_asset_id)
  WHERE status = 'open';
CREATE INDEX idx_vendor_rentals_org_status
  ON vendor_rentals (org_id, status, start_on DESC);
CREATE INDEX idx_vendor_rentals_vendor
  ON vendor_rentals (org_id, vendor_id)
  WHERE vendor_id IS NOT NULL;

ALTER TABLE vendor_rentals ENABLE ROW LEVEL SECURITY;
CREATE POLICY vendor_rentals_org ON vendor_rentals
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

-- An agreement is closed once, by its return, and never deleted.
CREATE FUNCTION vendor_rentals_guard() RETURNS trigger AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN
    IF NOT EXISTS (SELECT 1 FROM orgs o WHERE o.id = OLD.org_id) THEN
      RETURN OLD;
    END IF;
    RAISE EXCEPTION
      'vendor_rentals is a record: close the agreement with its return instead of deleting it';
  END IF;

  IF NEW.created_by IS NULL
     AND (to_jsonb(NEW) - 'created_by') = (to_jsonb(OLD) - 'created_by') THEN
    RETURN NEW;
  END IF;
  -- Deleting a vendor clears the reference and nothing else. That is not an
  -- edit to the agreement: the terms, dates and return stay exactly as they
  -- were, and the row keeps the vendor's name where it recorded one. Without
  -- this arm a closed agreement would block the delete of the vendor row.
  IF NEW.vendor_id IS NULL
     AND (to_jsonb(NEW) - 'vendor_id') = (to_jsonb(OLD) - 'vendor_id') THEN
    RETURN NEW;
  END IF;

  IF OLD.status = 'returned' THEN
    RAISE EXCEPTION
      'vendor_rentals is closed: a returned agreement is a record and does not change';
  END IF;

  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER vendor_rentals_guard_trigger
  BEFORE UPDATE OR DELETE ON vendor_rentals
  FOR EACH ROW EXECUTE FUNCTION vendor_rentals_guard();

-- --- a rented-in unit never depreciates --------------------------------------

-- ADR 0035 §4.1: `rental_in` is the vendor's gear, valued as rental expense.
-- Its asset must say so in the two columns that decide valuation, and a
-- depreciation method on it would put somebody else's laptop into our book
-- value. NOT VALID: every new or rewritten row is checked, while a historical
-- row a backfill left inconsistent is not refused by the migration itself.
ALTER TABLE device_assets
  ADD CONSTRAINT device_assets_rented_in_shape CHECK (
    custody_class <> 'rental_in'
    OR (ownership IN ('leased', 'rented') AND depreciation_method = 'none')
  ) NOT VALID;

-- --- an open vendor rental locks the unit ------------------------------------

-- The migration-0232 apply trigger refuses a class movement while the unit is on
-- an open client loan, an open tool checkout, or a ticket. A vendor rental is
-- the fourth kind of open agreement, and it was added after that trigger, so the
-- function is replaced here with the same body plus that one check. Nothing else
-- changes: the lock order, the pair re-read, the terminal and stale-from
-- refusals, and the two projection updates are byte-for-byte the 0232 shape.
CREATE OR REPLACE FUNCTION inventory_class_movement_apply() RETURNS trigger AS $$
DECLARE
  current_class text;
  current_serial uuid;
  current_asset uuid;
  unit_serial uuid;
  unit_asset uuid;
BEGIN
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
    -- The inbound half (0237): a unit rented FROM a vendor is under an open
    -- agreement until it goes back, and it cannot change class before then.
    IF EXISTS (
      SELECT 1 FROM vendor_rentals v
       WHERE v.device_asset_id = unit_asset
         AND v.org_id = NEW.org_id
         AND v.status = 'open'
    ) THEN
      RAISE EXCEPTION 'inventory class movement: asset % is on an open vendor rental',
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

-- --- the rental line kind ----------------------------------------------------

-- NOT VALID: every row this transaction writes is checked, and the scan of the
-- invoice lines already on disk runs in 0250 instead of holding ACCESS EXCLUSIVE
-- on a populated table through this migration.
ALTER TABLE invoice_lines DROP CONSTRAINT invoice_lines_kind_check;
ALTER TABLE invoice_lines ADD CONSTRAINT invoice_lines_kind_check
  CHECK (kind IN ('recurring_base','retainer_base','retainer_overage',
                  'time_materials','fixed_fee','per_unit_base','per_incident',
                  'block_sale','block_drawdown','cap_overage','minimum_trueup',
                  'change_order','sla_credit','escalator','project_release',
                  'rental')) NOT VALID;
