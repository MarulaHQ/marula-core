-- Contracts + recurring billing rules (the private build tracker Phase 3, Wave 8b lane
-- BILLING; migration 0029 is this lane's exclusive window).
--
-- Purpose: the commercial-agreement substrate. Four contract types
-- (recurring / time_and_materials / retainer / fixed_fee) with effective
-- dating and the configuration their recurring-billing rules evaluate:
--
--   * contracts — the agreement row. Money is integer minor units +
--     ISO 4217 currency (floats never touch amounts). Type-conditional
--     fields (amount / hourly_rate / included_minutes / overage_rate /
--     billing_frequency + billing_anchor) are validated in core-domain,
--     not by cross-column CHECKs: the rules stay testable in one place
--     and additive to evolve. billing_anchor is the FIRST billing
--     period's start; period boundaries derive deterministically from
--     anchor + frequency (core-billing period arithmetic) — periods are
--     computed, not stored, until Wave 9's invoice run needs persisted
--     period rows.
--
--   * contract_rates — T&M rate card keyed by time_entries.work_type
--     (the free work label that already exists on every entry, 0005).
--     Entries with no matching row bill at the contract's default
--     hourly_rate_minor.
--
--   * contract_fixed_fees — the fixed_fee dated schedule; each row
--     becomes a candidate line when due_on falls in an evaluation window
--     inside the effective window.
--
-- Non-purpose: NO invoice, invoice-line, payment, or dunning tables —
-- Wave 8b ships contract CRUD plus read-only rule-evaluation previews
-- only (no invoice persistence). No `billed` markers either: Wave 9's
-- invoice run owns that state. Also not a quote/catalog (separate
-- Phase-3 line).
--
-- RLS: org-scoped tenant tables following the 0001/0028 convention.

CREATE TABLE contracts (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  client_id uuid NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
  number text NOT NULL,
  name text NOT NULL,
  contract_type text NOT NULL
    CHECK (contract_type IN ('recurring','time_and_materials','retainer','fixed_fee')),
  status text NOT NULL DEFAULT 'draft'
    CHECK (status IN ('draft','active','ended','cancelled')),
  currency text NOT NULL,
  effective_from date NOT NULL,
  effective_until date,
  billing_frequency text
    CHECK (billing_frequency IS NULL OR billing_frequency IN
           ('weekly','monthly','quarterly','semiannual','annual')),
  billing_anchor date,
  amount_minor bigint,
  hourly_rate_minor bigint,
  included_minutes integer,
  overage_rate_minor bigint,
  notes text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, number)
);
CREATE INDEX idx_contracts_client ON contracts (org_id, client_id);
CREATE INDEX idx_contracts_status ON contracts (org_id, status);

CREATE TABLE contract_rates (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  contract_id uuid NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
  work_type text NOT NULL,
  rate_minor bigint NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (contract_id, work_type)
);

CREATE TABLE contract_fixed_fees (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  contract_id uuid NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
  label text NOT NULL,
  amount_minor bigint NOT NULL,
  due_on date NOT NULL,
  notes text,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_contract_fees_due ON contract_fixed_fees (contract_id, due_on);

ALTER TABLE contracts ENABLE ROW LEVEL SECURITY;
CREATE POLICY contracts_org ON contracts
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE contract_rates ENABLE ROW LEVEL SECURITY;
CREATE POLICY contract_rates_org ON contract_rates
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE contract_fixed_fees ENABLE ROW LEVEL SECURITY;
CREATE POLICY contract_fixed_fees_org ON contract_fixed_fees
  USING (org_id = current_setting('app.org_id', true)::uuid);
