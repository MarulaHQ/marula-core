-- Native invoicing v1 (TASKLIST Phase 3, Wave 9 lane INVOICING; migration
-- 0036 is this lane's exclusive window).
--
-- Purpose: the invoice-run persistence the Wave 8b billing lane
-- deliberately left out. One invoice run evaluates every billable
-- contract over a window (core-billing rules engine, the same evaluation
-- the previews expose) and persists one invoice PER CONTRACT that
-- produced at least one candidate line:
--
--   * invoice_runs — the batch record: window, due-in-days policy, and
--     roll-up counters. Purely informational; the invoices it created
--     survive independently (runs are never deleted in v1).
--
--   * invoices — the billing document. Money is integer minor units +
--     ISO 4217 currency copied from the contract (no conversion). Two
--     state machines as data, never side effects:
--       - status: 'issued' → 'paid' | 'void' (document lifecycle;
--         'draft' is reserved for a future manual-invoice flow).
--       - dunning_state: 'none' → 'reminder_sent' → 'overdue' →
--         'final_notice' → 'collections' (forward-only; transitions are
--         plain API writes that emit events and land in the write trail).
--
--   * invoice_lines — the persisted candidate lines (kind mirrors
--     core_billing BillingLineKind). Line dedupe ("billed once, ever")
--     is enforced three ways, matching how the engine derives each kind:
--       - period lines (recurring_base / retainer_base /
--         retainer_overage): UNIQUE (org_id, contract_id, kind,
--         period_start) — one line per contract per period, ever.
--       - fixed_fee lines: UNIQUE on (contract, due date, label) — the
--         human identity of a schedule entry, stable across fee-id churn
--         when the contract's schedule is replaced.
--       - T&M lines: no period uniqueness (several buckets share one
--         window); the invoice_time_entries link table guarantees every
--         time entry bills at most once. Retainer invoices link the
--         entries their billed periods consumed; recurring/fixed-fee
--         invoices price by period/schedule and link none.
--     The API layer skips already-billed candidates before insert; the
--     unique indexes are the concurrency backstop, not the primary flow.
--
--   * invoice_payments — receipts recorded through the PaymentProvider
--     seam. provider stays a string ('none' in v1 — no external PSP);
--     provider_ref carries whatever the provider handed back.
--
-- Non-purpose: no tax/discount engine (total_minor is the line sum), no
-- credit notes, no payment retries/refunds, no scheduled auto-dunning
-- (dunning transitions are explicit API writes). Voiding releases
-- nothing: consumption links and billed-period markers are permanent, so
-- re-running a window can never double-bill (v1 gap, recorded in
-- TASKLIST).
--
-- Additive-only (U9 gate): new tables plus one counter column on orgs;
-- nothing existing is rewritten or dropped.

ALTER TABLE orgs ADD COLUMN invoice_sequence bigint NOT NULL DEFAULT 0;

CREATE TABLE invoice_runs (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  number text NOT NULL,
  period_from date NOT NULL,
  period_to date NOT NULL,
  invoice_count integer NOT NULL DEFAULT 0,
  total_minor bigint NOT NULL DEFAULT 0,
  currency text,
  due_in_days integer NOT NULL DEFAULT 30,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, number)
);
CREATE INDEX idx_invoice_runs_created ON invoice_runs (org_id, created_at DESC);

CREATE TABLE invoices (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  run_id uuid REFERENCES invoice_runs(id) ON DELETE SET NULL,
  client_id uuid NOT NULL REFERENCES clients(id) ON DELETE RESTRICT,
  contract_id uuid NOT NULL REFERENCES contracts(id) ON DELETE RESTRICT,
  number text NOT NULL,
  status text NOT NULL DEFAULT 'issued'
    CHECK (status IN ('draft','issued','paid','void')),
  currency text NOT NULL,
  period_from date,
  period_to date,
  issued_at date NOT NULL,
  due_on date NOT NULL,
  total_minor bigint NOT NULL,
  total_paid_minor bigint NOT NULL DEFAULT 0,
  dunning_state text NOT NULL DEFAULT 'none'
    CHECK (dunning_state IN ('none','reminder_sent','overdue','final_notice','collections')),
  dunning_updated_at timestamptz,
  dunning_note text,
  notes text,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, number)
);
CREATE INDEX idx_invoices_client ON invoices (org_id, client_id);
CREATE INDEX idx_invoices_contract ON invoices (org_id, contract_id);
CREATE INDEX idx_invoices_status ON invoices (org_id, status);
CREATE INDEX idx_invoices_dunning ON invoices (org_id, dunning_state);

CREATE TABLE invoice_lines (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  invoice_id uuid NOT NULL REFERENCES invoices(id) ON DELETE CASCADE,
  contract_id uuid NOT NULL REFERENCES contracts(id) ON DELETE RESTRICT,
  kind text NOT NULL
    CHECK (kind IN ('recurring_base','retainer_base','retainer_overage',
                    'time_materials','fixed_fee')),
  description text NOT NULL,
  period_start date NOT NULL,
  period_end date NOT NULL,
  quantity_minutes bigint NOT NULL DEFAULT 0,
  unit_amount_minor bigint NOT NULL,
  amount_minor bigint NOT NULL,
  position integer NOT NULL DEFAULT 0
);

-- One base/overage line per contract per period, ever (run-time skips
-- make this a concurrency backstop, not the primary flow).
CREATE UNIQUE INDEX uq_invoice_lines_period
  ON invoice_lines (org_id, contract_id, kind, period_start)
  WHERE kind IN ('recurring_base','retainer_base','retainer_overage');

-- One fee line per (contract, due date, label) — the schedule entry's
-- human identity survives fee-card replacement (ids churn on edit).
CREATE UNIQUE INDEX uq_invoice_lines_fixed_fee
  ON invoice_lines (org_id, contract_id, kind, period_start, description)
  WHERE kind = 'fixed_fee';
CREATE INDEX idx_invoice_lines_invoice ON invoice_lines (invoice_id, position);

CREATE TABLE invoice_time_entries (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  invoice_id uuid NOT NULL REFERENCES invoices(id) ON DELETE CASCADE,
  time_entry_id uuid NOT NULL REFERENCES time_entries(id) ON DELETE RESTRICT,
  UNIQUE (time_entry_id)
);
CREATE INDEX idx_invoice_time_entries_invoice ON invoice_time_entries (invoice_id);

CREATE TABLE invoice_payments (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  invoice_id uuid NOT NULL REFERENCES invoices(id) ON DELETE RESTRICT,
  amount_minor bigint NOT NULL CHECK (amount_minor > 0),
  currency text NOT NULL,
  provider text NOT NULL DEFAULT 'none',
  provider_ref text,
  notes text,
  received_at timestamptz NOT NULL DEFAULT now(),
  created_by uuid REFERENCES users(id) ON DELETE SET NULL
);
CREATE INDEX idx_invoice_payments_invoice ON invoice_payments (invoice_id, received_at);

ALTER TABLE invoice_runs ENABLE ROW LEVEL SECURITY;
CREATE POLICY invoice_runs_org ON invoice_runs
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE invoices ENABLE ROW LEVEL SECURITY;
CREATE POLICY invoices_org ON invoices
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE invoice_lines ENABLE ROW LEVEL SECURITY;
CREATE POLICY invoice_lines_org ON invoice_lines
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE invoice_time_entries ENABLE ROW LEVEL SECURITY;
CREATE POLICY invoice_time_entries_org ON invoice_time_entries
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE invoice_payments ENABLE ROW LEVEL SECURITY;
CREATE POLICY invoice_payments_org ON invoice_payments
  USING (org_id = current_setting('app.org_id', true)::uuid);
