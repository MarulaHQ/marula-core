-- Wave 39 lane 3: contract-type breadth (TASKLIST Wave 39 lane
-- `p39-contract-types`; migration 0112 is this lane's exclusive window).
--
-- Purpose: carry every major and minor MSP-client relationship type the
-- billing engine evaluates. Where 0029 shipped four contract shapes,
-- this migration adds the two missing per-item shapes and the attached
-- configuration the existing shapes evaluate:
--
--   (A) contract_blocks — prepaid hour banks: sold minutes, sale price,
--       optional discounted block rate, expiry, rollover policy. Rows
--       are immutable once sold; rollover MINTS a successor block, it
--       never edits the old row (additive-ledger doctrine). Draw-downs
--       persist in contract_block_draws, so the remaining balance is
--       always computed, never stored.
--
--   (B) contract_caps — T&M not-to-exceed caps, per period or per
--       contract, with a TYPED cap-reached behavior (stop / warn /
--       overage_billable). Never a silent cutoff.
--
--   (C/D) contracts gains the per-unit (per-seat/per-device) and
--       per-incident (per closed ticket) type columns; unit counts read
--       the org's inventories at evaluation time (no unit snapshot
--       tables — the count IS the point).
--
--   (E) contract_minimums (per-period commitment + true-up),
--       contract_escalators (scheduled uplift bps per anniversary),
--       contract_change_orders (billable amendments with their own
--       lifecycle: draft → approved → billed, rejected/cancelled
--       terminal), contract_sla_credits (pending negative adjustments
--       applied at run time, never silent).
--
--   (F) contracts.po_number + invoices.po_number — the client PO
--       reference printed on the produced invoices. TAX IS NOT HERE BY
--       DESIGN: the 0036 boundary stands ("no tax/discount engine");
--       tax stays an accounting-export concern the operator configures
--       in their accounting system (documented, not built).
--
--   (G) invoice_lines.kind CHECK extends for the new candidate kinds
--       the engine emits; the unique indexes extend with it (the run
--       dedupe identities ride the same shapes as v1).
--
-- Non-purpose: no block-consumption read-model tables yet (lane 2's
-- verification workspace owns that surface, reading the draws ledger
-- directly), no project/phase links (lane 4), no tax engine (F above),
-- no `work_type` category FK (lane 1's slugs blend into `rates` reading
-- only — this lane renames nothing).

-- contracts: the two new relationship types ride the same row; their
-- type-conditional columns validate in core-domain like the existing
-- four (cross-column CHECKs stay out of SQL on purpose — 0029's rule).
ALTER TABLE contracts DROP CONSTRAINT contracts_contract_type_check;
ALTER TABLE contracts ADD CONSTRAINT contracts_contract_type_check
  CHECK (contract_type IN ('recurring','time_and_materials','retainer',
                           'fixed_fee','per_unit','per_incident'));

ALTER TABLE contracts ADD COLUMN unit_price_minor bigint;
ALTER TABLE contracts ADD COLUMN unit_basis text
  CHECK (unit_basis IS NULL OR unit_basis IN ('devices','users'));
ALTER TABLE contracts ADD COLUMN incident_price_minor bigint;
ALTER TABLE contracts ADD COLUMN incident_priority text
  CHECK (incident_priority IS NULL OR incident_priority = 'any'
         OR incident_priority IN ('p1','p2','p3','p4'));
ALTER TABLE contracts ADD COLUMN po_number text;

-- invoices: the PO reference moves onto the document at run time
-- (copied from the contract so each invoice carries what the client's
-- AP system matches against). Nullable forever: not every client
-- issues POs.
ALTER TABLE invoices ADD COLUMN po_number text;

-- (A) prepaid hour banks.
CREATE TABLE contract_blocks (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  contract_id uuid NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
  -- Optional successor link (rollover mints the next block and points
  -- it back at its source for the audit trail).
  rolled_over_from uuid REFERENCES contract_blocks(id) ON DELETE SET NULL,
  label text NOT NULL,
  total_minutes integer NOT NULL CHECK (total_minutes > 0),
  sale_price_minor bigint NOT NULL CHECK (sale_price_minor >= 0),
  block_rate_minor bigint CHECK (block_rate_minor IS NULL OR block_rate_minor >= 0),
  expires_on date,
  rollover_policy text NOT NULL DEFAULT 'expire'
    CHECK (rollover_policy IN ('expire','rollover','none')),
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, contract_id, label)
);
CREATE INDEX idx_contract_blocks_contract
  ON contract_blocks (org_id, contract_id);

-- The draw-down ledger: one row per (block, time entry) consumption.
-- INSERT-only — corrections add compensating rows, never UPDATE (the
-- additive rule applies to the ledger too). The invoice that consumed
-- the draw is recorded for the verification view.
CREATE TABLE contract_block_draws (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  block_id uuid NOT NULL REFERENCES contract_blocks(id) ON DELETE CASCADE,
  time_entry_id uuid NOT NULL REFERENCES time_entries(id) ON DELETE RESTRICT,
  invoice_id uuid REFERENCES invoices(id) ON DELETE SET NULL,
  minutes integer NOT NULL CHECK (minutes > 0),
  drawn_on date NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (block_id, time_entry_id)
);
CREATE INDEX idx_block_draws_block ON contract_block_draws (org_id, block_id);
CREATE INDEX idx_block_draws_entry ON contract_block_draws (time_entry_id);

ALTER TABLE contract_blocks ENABLE ROW LEVEL SECURITY;
CREATE POLICY contract_blocks_org ON contract_blocks
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE contract_block_draws ENABLE ROW LEVEL SECURITY;
CREATE POLICY contract_block_draws_org ON contract_block_draws
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- (B) NTE caps. At most one cap per (contract, scope, work_type): two
-- caps of the same scope on the same bucket would make cutoff behavior
-- ambiguous, and ambiguity in money is a defect.
CREATE TABLE contract_caps (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  contract_id uuid NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
  scope text NOT NULL CHECK (scope IN ('period','contract')),
  cap_minor bigint NOT NULL CHECK (cap_minor > 0),
  reached_behavior text NOT NULL
    CHECK (reached_behavior IN ('stop','warn','overage_billable')),
  overage_rate_minor bigint
    CHECK (overage_rate_minor IS NULL OR overage_rate_minor >= 0),
  work_type text,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, contract_id, scope, work_type)
);
CREATE INDEX idx_contract_caps_contract ON contract_caps (org_id, contract_id);
ALTER TABLE contract_caps ENABLE ROW LEVEL SECURITY;
CREATE POLICY contract_caps_org ON contract_caps
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- (E) minimum commitment + true-up: one row per contract. The true-up
-- line tops each period's billable lines up to the minimum when they
-- fall short.
CREATE TABLE contract_minimums (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  contract_id uuid NOT NULL UNIQUE REFERENCES contracts(id) ON DELETE CASCADE,
  minimum_commitment_minor bigint NOT NULL CHECK (minimum_commitment_minor >= 0),
  created_at timestamptz NOT NULL DEFAULT now()
);
ALTER TABLE contract_minimums ENABLE ROW LEVEL SECURITY;
CREATE POLICY contract_minimums_org ON contract_minimums
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- (E) escalators: uplift in basis points applied at each anniversary of
-- the billing anchor, starting when `first_uplift_on` says so (NULL =
-- the anchor anniversary itself).
CREATE TABLE contract_escalators (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  contract_id uuid NOT NULL UNIQUE REFERENCES contracts(id) ON DELETE CASCADE,
  uplift_bps integer NOT NULL CHECK (uplift_bps > 0 AND uplift_bps <= 5000),
  first_uplift_on date,
  created_at timestamptz NOT NULL DEFAULT now()
);
ALTER TABLE contract_escalators ENABLE ROW LEVEL SECURITY;
CREATE POLICY contract_escalators_org ON contract_escalators
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- (E) change orders: billable amendments with their own lifecycle.
-- Status vocabulary is a typed CHECK; the API layer enforces the
-- transitions (draft → approved → billed, rejected/cancelled terminal)
-- and rides the existing audit-trail pattern for every move.
CREATE TABLE contract_change_orders (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  contract_id uuid NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
  number text NOT NULL,
  title text NOT NULL,
  description text,
  amount_minor bigint NOT NULL CHECK (amount_minor >= 0),
  status text NOT NULL DEFAULT 'draft'
    CHECK (status IN ('draft','approved','rejected','billed','cancelled')),
  approved_by uuid REFERENCES users(id) ON DELETE SET NULL,
  approved_at timestamptz,
  billed_invoice_id uuid REFERENCES invoices(id) ON DELETE SET NULL,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, contract_id, number)
);
CREATE INDEX idx_change_orders_contract ON contract_change_orders (org_id, contract_id);
ALTER TABLE contract_change_orders ENABLE ROW LEVEL SECURITY;
CREATE POLICY contract_change_orders_org ON contract_change_orders
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- (E) SLA service credits: pending negative adjustments. Amount stored
-- POSITIVE; direction is the line kind's meaning at evaluation. The
-- ticket/process trail rides the reason text (never silent).
CREATE TABLE contract_sla_credits (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  contract_id uuid NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
  ticket_id uuid REFERENCES tickets(id) ON DELETE SET NULL,
  -- Contract-registry input: the reason is free text naming the missed
  -- target, but blank reasons are refused here (never silent).
  reason text NOT NULL CHECK (char_length(trim(reason)) BETWEEN 1 AND 500),
  amount_minor bigint NOT NULL CHECK (amount_minor > 0),
  currency text NOT NULL,
  applies_on date NOT NULL,
  status text NOT NULL DEFAULT 'pending'
    CHECK (status IN ('pending','applied','cancelled')),
  applied_invoice_id uuid REFERENCES invoices(id) ON DELETE SET NULL,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_sla_credits_contract ON contract_sla_credits (org_id, contract_id, status);
ALTER TABLE contract_sla_credits ENABLE ROW LEVEL SECURITY;
CREATE POLICY contract_sla_credits_org ON contract_sla_credits
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- (G) invoice line kinds + dedupe identities. The CHECK widens for the
-- new candidate kinds; the unique indexes restate their where-clauses
-- with the new period-identity kinds included. Pending SLA credits
-- applied by one run dedupe on (contract, period_start, description) —
-- the credit record's id rides the description so a re-run can never
-- double-apply.
ALTER TABLE invoice_lines DROP CONSTRAINT invoice_lines_kind_check;
ALTER TABLE invoice_lines ADD CONSTRAINT invoice_lines_kind_check
  CHECK (kind IN ('recurring_base','retainer_base','retainer_overage',
                  'time_materials','fixed_fee','per_unit_base','per_incident',
                  'block_sale','block_drawdown','cap_overage','minimum_trueup',
                  'change_order','sla_credit','escalator'));

DROP INDEX IF EXISTS uq_invoice_lines_period;
CREATE UNIQUE INDEX uq_invoice_lines_period
  ON invoice_lines (org_id, contract_id, kind, period_start)
  WHERE kind IN ('recurring_base','retainer_base','retainer_overage',
                 'per_unit_base','minimum_trueup','escalator');

DROP INDEX IF EXISTS uq_invoice_lines_fixed_fee;
CREATE UNIQUE INDEX uq_invoice_lines_fixed_fee
  ON invoice_lines (org_id, contract_id, kind, period_start, description)
  WHERE kind IN ('fixed_fee','block_sale','block_drawdown','cap_overage',
                 'change_order','sla_credit','per_incident');
