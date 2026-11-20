-- Inventory policy and money vocabulary (Wave 76 lane 1,
-- `p76-inventory-foundation`; migration 0234 closes this lane's window).
--
-- The identity half of the foundation is 0232/0233. This file adds the
-- vocabulary the money rows will carry, as nullable columns plus the CHECK that
-- names the allowed values. Nothing here changes a behaviour and no route reads
-- these columns yet: the purchasing and billing lanes write them, which is why
-- every one of them is optional and defaults to today's meaning.
--
--   * `orgs.landed_cost_mode` — `invoice_values` (default, today's behaviour) or
--     `landed_costs`. Switching applies to new receipts; a historical layer is
--     never rewritten (ADR 0035 §3).
--
--   * `orgs.recharge_*` — the organisation default recharge rule for a purchase
--     made on a client's behalf: kind, percentage, fixed amount, currency. The
--     percentage kind is the default, and the percentage starts at zero so an
--     org that never configures it recharges at cost rather than inventing a
--     markup.
--
--   * `weight_g` — integer grams on an item and on a receipt line. Weight is a
--     real allocation basis and the only one e-waste can use, so it lives on
--     both shapes. On a receipt line the value is per unit; allocation
--     multiplies it by the line quantity.
--
--   * `reporting_class` — the eight classes in ADR 0035 §6.4, on a ticket cost
--     line and a ticket expense. NULL means "not classified yet": the column
--     does not guess a class from a free-text description, and the billing lane
--     fills it when a line gains product identity.
--
--   * Source identity on cost, invoice and credit rows — `source_kind` plus
--     `source_id`, both set or both NULL. This is the `§9` chain: a billed line
--     names the line it consumed, and a credit names what it corrects, so a
--     later reversal can reach the unit. The pair is nullable because every
--     pre-existing row was created before the chain existed.
--
-- Why `credit_notes` and not a lines table: a credit note is a total today
-- (migration 0058) and gains lines with the billing-join lane. The note-level
-- pair here lets a note cite the document it corrects in the meantime; the line
-- table reuses the same vocabulary when it lands.
--
-- Additive-only: nullable columns, one defaulted enum column, CHECK constraints
-- on values that do not exist yet. No index on a hot path is rebuilt.

-- --- organisation policy -----------------------------------------------------

ALTER TABLE orgs
  ADD COLUMN landed_cost_mode text NOT NULL DEFAULT 'invoice_values'
    CHECK (landed_cost_mode IN ('invoice_values', 'landed_costs')),
  ADD COLUMN recharge_rule_kind text NOT NULL DEFAULT 'markup_bps'
    CHECK (recharge_rule_kind IN ('at_cost', 'markup_bps', 'fixed', 'none')),
  ADD COLUMN recharge_markup_bps integer NOT NULL DEFAULT 0
    CHECK (recharge_markup_bps >= 0 AND recharge_markup_bps <= 100000),
  ADD COLUMN recharge_fixed_minor bigint
    CHECK (recharge_fixed_minor IS NULL OR recharge_fixed_minor >= 0),
  ADD COLUMN recharge_currency text
    CHECK (recharge_currency IS NULL OR recharge_currency ~ '^[A-Z]{3}$');

ALTER TABLE orgs
  ADD CONSTRAINT orgs_recharge_rule_shape CHECK (
    CASE recharge_rule_kind
      WHEN 'fixed' THEN recharge_fixed_minor IS NOT NULL AND recharge_currency IS NOT NULL
      WHEN 'markup_bps' THEN recharge_markup_bps IS NOT NULL
      ELSE true
    END
  );

-- --- weight ------------------------------------------------------------------

ALTER TABLE stock_items
  ADD COLUMN weight_g integer
    CHECK (weight_g IS NULL OR weight_g > 0);

ALTER TABLE purchase_receipt_lines
  ADD COLUMN weight_g integer
    CHECK (weight_g IS NULL OR weight_g > 0);

-- --- reporting class ---------------------------------------------------------

ALTER TABLE ticket_cost_lines
  ADD COLUMN reporting_class text
    CHECK (reporting_class IS NULL OR reporting_class IN (
      'inventory', 'freight_in', 'duty', 'service_cost',
      'rental_expense', 'ewaste_cost', 'overhead', 'cogs'));

ALTER TABLE ticket_expenses
  ADD COLUMN reporting_class text
    CHECK (reporting_class IS NULL OR reporting_class IN (
      'inventory', 'freight_in', 'duty', 'service_cost',
      'rental_expense', 'ewaste_cost', 'overhead', 'cogs'));

CREATE INDEX idx_ticket_cost_lines_reporting_class
  ON ticket_cost_lines (org_id, reporting_class)
  WHERE reporting_class IS NOT NULL;

-- --- source identity ---------------------------------------------------------

ALTER TABLE ticket_cost_lines
  ADD COLUMN source_kind text
    CHECK (source_kind IS NULL OR source_kind IN (
      'stock_movement', 'purchase_receipt_line', 'ticket_cost_line',
      'invoice_line', 'credit_line', 'acquisition_cost', 'rental_charge',
      'manual')),
  ADD COLUMN source_id uuid;

ALTER TABLE ticket_cost_lines
  ADD CONSTRAINT ticket_cost_lines_source_pair CHECK (
    (source_kind IS NULL AND source_id IS NULL)
    OR (source_kind = 'manual' AND source_id IS NULL)
    OR (source_kind IS NOT NULL AND source_kind <> 'manual' AND source_id IS NOT NULL)
  );

ALTER TABLE invoice_lines
  ADD COLUMN source_kind text
    CHECK (source_kind IS NULL OR source_kind IN (
      'stock_movement', 'purchase_receipt_line', 'ticket_cost_line',
      'invoice_line', 'credit_line', 'acquisition_cost', 'rental_charge',
      'manual')),
  ADD COLUMN source_id uuid;

ALTER TABLE invoice_lines
  ADD CONSTRAINT invoice_lines_source_pair CHECK (
    (source_kind IS NULL AND source_id IS NULL)
    OR (source_kind = 'manual' AND source_id IS NULL)
    OR (source_kind IS NOT NULL AND source_kind <> 'manual' AND source_id IS NOT NULL)
  );

ALTER TABLE credit_notes
  ADD COLUMN source_kind text
    CHECK (source_kind IS NULL OR source_kind IN (
      'stock_movement', 'purchase_receipt_line', 'ticket_cost_line',
      'invoice_line', 'credit_line', 'acquisition_cost', 'rental_charge',
      'manual')),
  ADD COLUMN source_id uuid;

ALTER TABLE credit_notes
  ADD CONSTRAINT credit_notes_source_pair CHECK (
    (source_kind IS NULL AND source_id IS NULL)
    OR (source_kind = 'manual' AND source_id IS NULL)
    OR (source_kind IS NOT NULL AND source_kind <> 'manual' AND source_id IS NOT NULL)
  );

CREATE INDEX idx_ticket_cost_lines_source
  ON ticket_cost_lines (org_id, source_kind, source_id)
  WHERE source_id IS NOT NULL;
CREATE INDEX idx_invoice_lines_source
  ON invoice_lines (org_id, source_kind, source_id)
  WHERE source_id IS NOT NULL;
CREATE INDEX idx_credit_notes_source
  ON credit_notes (org_id, source_kind, source_id)
  WHERE source_id IS NOT NULL;
