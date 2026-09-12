-- Migration 0065: FX foundation (ADR 0002, Wave 12 lane 10, FX1 + FX2 + FX4).
--
-- Purpose: the org base currency and the conversion-snapshot columns at
-- the five lock moments. ADR 0002 doctrine: the transaction currency is
-- the fact; conversion is an immutable snapshot. Existing money columns
-- (contracts 0029, projects 0030, invoicing 0036, workflow costs/expenses
-- 0056, credit notes 0058, catalog/quotes/pipeline 0060, inventory 0061)
-- already store `Money` (transaction currency + integer minor units);
-- this migration does NOT rewrite them — it generalizes the convention
-- and adds the snapshot seam:
--
--   * FX1 — `orgs.base_currency`: one reporting currency per org. The
--     column defaults to 'USD' so the existing single-currency install
--     base upgrades without data movement; new orgs pass their base at
--     creation. Changing an existing base is governed by the API (a full
--     re-snapshot) — the core never revaluates in place.
--
--   * FX2 — Money column generalization: every `currency`/`*_currency`
--     column on a money-bearing table gets an ISO 4217 shape guard
--     (`^[A-Z]{3}$`). Transaction currency is stored everywhere; there
--     are no base-only money columns. Lines inherit their document
--     header's currency (single-currency documents), so no per-line
--     currency column is added.
--
--   * FX4 — conversion snapshots at the five lock moments. Uniform group:
--     `fx_rate_used numeric(24,12)` (BASE major units per one FACT major
--     unit — the fact→base direction, so base = original × rate_used),
--     `fx_rate_date`, `fx_source`, and the base-currency amount. The
--     group is atomic (all set or all NULL): single-currency facts
--     (currency = org base) carry no snapshot. Snapshots are immutable
--     once written (BEFORE UPDATE triggers reject any change to the
--     group); corrections are new rows in `fx_rates` (0066) plus new
--     facts, never rewrites.
--
--     The five lock moments and where they live in the installed base:
--       1. expense/purchase entry  → ticket_expenses, ticket_cost_lines
--          (0056) — snapshot at insert when currency ≠ org base.
--       2. invoice issue           → invoices (0036) — snapshot at the
--          issue leg, locking the document total in base currency.
--       3. time-entry costing      → invoice_lines (0036) — the moment a
--          run prices labor into `time_materials` (and any other) lines;
--          the line snapshot preserves what the priced amount was worth
--          in base currency at pricing time.
--       4. inventory receipt       → device_assets (0061) — snapshot on
--          the purchase cost when written in a non-base currency. Rental
--          rates stay transaction-currency facts; they lock at the
--          moment they become an expense/cost line, not here.
--       5. payment settlement      → invoice_payments (0036) — snapshot
--          at receipt when the payment currency ≠ org base.
--
--     Read-side: the reconciliation read model (0067) sums BASE amounts
--     from snapshots only; it never re-converts. Summing mixed original
--     currencies is a type error in core-domain, not a warning.
--
-- Additive-only: new columns + named CHECK constraints + immutability
-- triggers. Nothing existing is rewritten or dropped.

-- --- FX1: org base currency --------------------------------------------------

ALTER TABLE orgs ADD COLUMN base_currency text NOT NULL DEFAULT 'USD';
ALTER TABLE orgs ADD CONSTRAINT orgs_base_currency_iso
  CHECK (base_currency ~ '^[A-Z]{3}$');

-- --- FX2: ISO 4217 shape guards on every stored currency column --------------
-- Nullable columns keep their optionality (a CHECK only rejects non-NULL
-- values that don't match). Every row written by the app layer is already
-- an uppercase 3-letter code, so backfill is a no-op by construction.

ALTER TABLE contracts ADD CONSTRAINT contracts_currency_iso
  CHECK (currency ~ '^[A-Z]{3}$');
ALTER TABLE invoices ADD CONSTRAINT invoices_currency_iso
  CHECK (currency ~ '^[A-Z]{3}$');
ALTER TABLE invoice_payments ADD CONSTRAINT invoice_payments_currency_iso
  CHECK (currency ~ '^[A-Z]{3}$');
ALTER TABLE projects ADD CONSTRAINT projects_currency_iso
  CHECK (currency IS NULL OR currency ~ '^[A-Z]{3}$');
ALTER TABLE project_phases ADD CONSTRAINT project_phases_milestone_currency_iso
  CHECK (milestone_currency IS NULL OR milestone_currency ~ '^[A-Z]{3}$');
ALTER TABLE project_tasks ADD CONSTRAINT project_tasks_milestone_currency_iso
  CHECK (milestone_currency IS NULL OR milestone_currency ~ '^[A-Z]{3}$');
ALTER TABLE ticket_cost_lines ADD CONSTRAINT ticket_cost_lines_currency_iso
  CHECK (currency ~ '^[A-Z]{3}$');
ALTER TABLE ticket_expenses ADD CONSTRAINT ticket_expenses_currency_iso
  CHECK (currency ~ '^[A-Z]{3}$');
ALTER TABLE credit_notes ADD CONSTRAINT credit_notes_currency_iso
  CHECK (currency ~ '^[A-Z]{3}$');
ALTER TABLE catalog_items ADD CONSTRAINT catalog_items_currency_iso
  CHECK (currency ~ '^[A-Z]{3}$');
ALTER TABLE quotes ADD CONSTRAINT quotes_currency_iso
  CHECK (currency ~ '^[A-Z]{3}$');
ALTER TABLE pipeline_deals ADD CONSTRAINT pipeline_deals_currency_iso
  CHECK (currency ~ '^[A-Z]{3}$');
ALTER TABLE device_assets ADD CONSTRAINT device_assets_rental_currency_iso
  CHECK (rental_currency IS NULL OR rental_currency ~ '^[A-Z]{3}$');
ALTER TABLE device_assets ADD CONSTRAINT device_assets_purchase_currency_iso
  CHECK (purchase_currency IS NULL OR purchase_currency ~ '^[A-Z]{3}$');

-- --- FX4: snapshot columns at the five lock moments ---------------------------
-- Group shape (uniform): fx_rate_used (BASE per FACT, major units),
-- fx_rate_date, fx_source, base amount. Atomic all-or-nothing.

-- 1. expense entry (0056)
ALTER TABLE ticket_expenses
  ADD COLUMN fx_rate_used numeric(24,12)
    CHECK (fx_rate_used IS NULL OR fx_rate_used > 0),
  ADD COLUMN fx_rate_date date,
  ADD COLUMN fx_source text,
  ADD COLUMN base_amount_minor bigint CHECK (base_amount_minor IS NULL OR base_amount_minor >= 0),
  ADD CONSTRAINT ticket_expenses_fx_snapshot_atomic
    CHECK ((fx_rate_used IS NULL AND fx_rate_date IS NULL AND fx_source IS NULL AND base_amount_minor IS NULL)
        OR (fx_rate_used IS NOT NULL AND fx_rate_date IS NOT NULL AND fx_source IS NOT NULL AND base_amount_minor IS NOT NULL));

-- 1b. parts/flat purchase entry (0056)
ALTER TABLE ticket_cost_lines
  ADD COLUMN fx_rate_used numeric(24,12)
    CHECK (fx_rate_used IS NULL OR fx_rate_used > 0),
  ADD COLUMN fx_rate_date date,
  ADD COLUMN fx_source text,
  ADD COLUMN base_amount_minor bigint CHECK (base_amount_minor IS NULL OR base_amount_minor >= 0),
  ADD CONSTRAINT ticket_cost_lines_fx_snapshot_atomic
    CHECK ((fx_rate_used IS NULL AND fx_rate_date IS NULL AND fx_source IS NULL AND base_amount_minor IS NULL)
        OR (fx_rate_used IS NOT NULL AND fx_rate_date IS NOT NULL AND fx_source IS NOT NULL AND base_amount_minor IS NOT NULL));

-- 2. invoice issue (0036)
ALTER TABLE invoices
  ADD COLUMN fx_rate_used numeric(24,12)
    CHECK (fx_rate_used IS NULL OR fx_rate_used > 0),
  ADD COLUMN fx_rate_date date,
  ADD COLUMN fx_source text,
  ADD COLUMN base_total_minor bigint CHECK (base_total_minor IS NULL OR base_total_minor >= 0),
  ADD CONSTRAINT invoices_fx_snapshot_atomic
    CHECK ((fx_rate_used IS NULL AND fx_rate_date IS NULL AND fx_source IS NULL AND base_total_minor IS NULL)
        OR (fx_rate_used IS NOT NULL AND fx_rate_date IS NOT NULL AND fx_source IS NOT NULL AND base_total_minor IS NOT NULL));

-- 3. time-entry costing (labor pricing at run time, 0036)
ALTER TABLE invoice_lines
  ADD COLUMN fx_rate_used numeric(24,12)
    CHECK (fx_rate_used IS NULL OR fx_rate_used > 0),
  ADD COLUMN fx_rate_date date,
  ADD COLUMN fx_source text,
  ADD COLUMN base_amount_minor bigint CHECK (base_amount_minor IS NULL OR base_amount_minor >= 0),
  ADD CONSTRAINT invoice_lines_fx_snapshot_atomic
    CHECK ((fx_rate_used IS NULL AND fx_rate_date IS NULL AND fx_source IS NULL AND base_amount_minor IS NULL)
        OR (fx_rate_used IS NOT NULL AND fx_rate_date IS NOT NULL AND fx_source IS NOT NULL AND base_amount_minor IS NOT NULL));

-- 4. inventory receipt (purchase cost, 0061)
ALTER TABLE device_assets
  ADD COLUMN purchase_fx_rate_used numeric(24,12)
    CHECK (purchase_fx_rate_used IS NULL OR purchase_fx_rate_used > 0),
  ADD COLUMN purchase_fx_rate_date date,
  ADD COLUMN purchase_fx_source text,
  ADD COLUMN base_purchase_cost_minor bigint
    CHECK (base_purchase_cost_minor IS NULL OR base_purchase_cost_minor >= 0),
  ADD CONSTRAINT device_assets_purchase_fx_snapshot_atomic
    CHECK ((purchase_fx_rate_used IS NULL AND purchase_fx_rate_date IS NULL AND purchase_fx_source IS NULL AND base_purchase_cost_minor IS NULL)
        OR (purchase_fx_rate_used IS NOT NULL AND purchase_fx_rate_date IS NOT NULL AND purchase_fx_source IS NOT NULL AND base_purchase_cost_minor IS NOT NULL));

-- 5. payment settlement (0036)
ALTER TABLE invoice_payments
  ADD COLUMN fx_rate_used numeric(24,12)
    CHECK (fx_rate_used IS NULL OR fx_rate_used > 0),
  ADD COLUMN fx_rate_date date,
  ADD COLUMN fx_source text,
  ADD COLUMN base_amount_minor bigint CHECK (base_amount_minor IS NULL OR base_amount_minor >= 0),
  ADD CONSTRAINT invoice_payments_fx_snapshot_atomic
    CHECK ((fx_rate_used IS NULL AND fx_rate_date IS NULL AND fx_source IS NULL AND base_amount_minor IS NULL)
        OR (fx_rate_used IS NOT NULL AND fx_rate_date IS NOT NULL AND fx_source IS NOT NULL AND base_amount_minor IS NOT NULL));

-- --- FX4: snapshot immutability (once written, never rewritten) ---------------
-- BEFORE UPDATE guards: any change to the snapshot group raises. The app
-- layer never writes these columns after the locking write; this makes
-- the doctrine mechanical.

CREATE OR REPLACE FUNCTION fx_reject_snapshot_rewrite() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'fx snapshot is immutable once written (table %, id %)',
    TG_TABLE_NAME, OLD.id
    USING ERRCODE = 'check_violation';
END;
$$;

-- 1. expense entry
CREATE OR REPLACE FUNCTION fx_guard_ticket_expenses() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF OLD.fx_rate_used IS DISTINCT FROM NEW.fx_rate_used
     OR OLD.fx_rate_date IS DISTINCT FROM NEW.fx_rate_date
     OR OLD.fx_source IS DISTINCT FROM NEW.fx_source
     OR OLD.base_amount_minor IS DISTINCT FROM NEW.base_amount_minor THEN
    PERFORM fx_reject_snapshot_rewrite();
  END IF;
  RETURN NEW;
END;
$$;
CREATE TRIGGER ticket_expenses_fx_immutable
  BEFORE UPDATE ON ticket_expenses
  FOR EACH ROW EXECUTE FUNCTION fx_guard_ticket_expenses();

-- 1b. purchase entry
CREATE OR REPLACE FUNCTION fx_guard_ticket_cost_lines() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF OLD.fx_rate_used IS DISTINCT FROM NEW.fx_rate_used
     OR OLD.fx_rate_date IS DISTINCT FROM NEW.fx_rate_date
     OR OLD.fx_source IS DISTINCT FROM NEW.fx_source
     OR OLD.base_amount_minor IS DISTINCT FROM NEW.base_amount_minor THEN
    PERFORM fx_reject_snapshot_rewrite();
  END IF;
  RETURN NEW;
END;
$$;
CREATE TRIGGER ticket_cost_lines_fx_immutable
  BEFORE UPDATE ON ticket_cost_lines
  FOR EACH ROW EXECUTE FUNCTION fx_guard_ticket_cost_lines();

-- 2. invoice issue
CREATE OR REPLACE FUNCTION fx_guard_invoices() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF OLD.fx_rate_used IS DISTINCT FROM NEW.fx_rate_used
     OR OLD.fx_rate_date IS DISTINCT FROM NEW.fx_rate_date
     OR OLD.fx_source IS DISTINCT FROM NEW.fx_source
     OR OLD.base_total_minor IS DISTINCT FROM NEW.base_total_minor THEN
    PERFORM fx_reject_snapshot_rewrite();
  END IF;
  RETURN NEW;
END;
$$;
CREATE TRIGGER invoices_fx_immutable
  BEFORE UPDATE ON invoices
  FOR EACH ROW EXECUTE FUNCTION fx_guard_invoices();

-- 3. time-entry costing (invoice lines are insert-only; the guard exists
--    so a future write path cannot silently re-price history)
CREATE OR REPLACE FUNCTION fx_guard_invoice_lines() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF OLD.fx_rate_used IS DISTINCT FROM NEW.fx_rate_used
     OR OLD.fx_rate_date IS DISTINCT FROM NEW.fx_rate_date
     OR OLD.fx_source IS DISTINCT FROM NEW.fx_source
     OR OLD.base_amount_minor IS DISTINCT FROM NEW.base_amount_minor THEN
    PERFORM fx_reject_snapshot_rewrite();
  END IF;
  RETURN NEW;
END;
$$;
CREATE TRIGGER invoice_lines_fx_immutable
  BEFORE UPDATE ON invoice_lines
  FOR EACH ROW EXECUTE FUNCTION fx_guard_invoice_lines();

-- 4. inventory receipt
CREATE OR REPLACE FUNCTION fx_guard_device_assets() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF OLD.purchase_fx_rate_used IS DISTINCT FROM NEW.purchase_fx_rate_used
     OR OLD.purchase_fx_rate_date IS DISTINCT FROM NEW.purchase_fx_rate_date
     OR OLD.purchase_fx_source IS DISTINCT FROM NEW.purchase_fx_source
     OR OLD.base_purchase_cost_minor IS DISTINCT FROM NEW.base_purchase_cost_minor THEN
    PERFORM fx_reject_snapshot_rewrite();
  END IF;
  RETURN NEW;
END;
$$;
CREATE TRIGGER device_assets_fx_immutable
  BEFORE UPDATE ON device_assets
  FOR EACH ROW EXECUTE FUNCTION fx_guard_device_assets();

-- 5. payment settlement
CREATE OR REPLACE FUNCTION fx_guard_invoice_payments() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF OLD.fx_rate_used IS DISTINCT FROM NEW.fx_rate_used
     OR OLD.fx_rate_date IS DISTINCT FROM NEW.fx_rate_date
     OR OLD.fx_source IS DISTINCT FROM NEW.fx_source
     OR OLD.base_amount_minor IS DISTINCT FROM NEW.base_amount_minor THEN
    PERFORM fx_reject_snapshot_rewrite();
  END IF;
  RETURN NEW;
END;
$$;
CREATE TRIGGER invoice_payments_fx_immutable
  BEFORE UPDATE ON invoice_payments
  FOR EACH ROW EXECUTE FUNCTION fx_guard_invoice_payments();
