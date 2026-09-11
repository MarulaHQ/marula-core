-- Quote builder + product/service catalog + sales pipeline (TASKLIST
-- Phase 3, Wave 12 lane 2; migration 0060 is this lane's exclusive
-- window).
--
-- Purpose: the commercial front-of-funnel the invoicing surface lacked —
--
--   * catalog_items — org-scoped products/services: name, SKU, unit,
--     unit price, cost, taxable flag, category/group, active flag. Money
--     is integer minor units + ISO 4217 (floats never touch amounts).
--     SKU is optional; when present it is unique per org (case
--     insensitive). Archival is a data flag (`active = false`), never a
--     delete — quotes snapshot their lines, so history survives.
--
--   * quotes + quote_lines — the quote document. One lifecycle as data,
--     never side effects:
--       status: 'draft' → 'sent' → 'accepted' | 'declined' | 'expired'.
--     A quote is client-scoped and numbered per org, carries a currency
--     and validity/expiry window, notes/terms, and money rolled up with
--     the existing Money conventions: subtotal = sum(line amount),
--     tax = half-even tax_rate_bps applied to the TAXABLE subtotal,
--     total = subtotal + tax. Every amount is integer minor units.
--
--     Immutability once sent: header/lines are only editable while the
--     quote is 'draft'. Sending locks the document; the lifecycle legs
--     (send/accept/decline/expire/convert) are the only later writes.
--
--     Quote lines snapshot catalog context (description/unit/unit price)
--     at write time; catalog_item_id is a soft reference (SET NULL on
--     catalog removal) so a quote never changes meaning when the catalog
--     does. quantity is stored as `quantity_milli` (thousandths of a
--     unit) so fractional quantities stay integer-only; amount_minor is
--     the half-even product of quantity × unit price.
--
--   * pipeline_stages + pipeline_deals + pipeline_deal_transitions —
--     the sales pipeline. Stages are org-scoped, ordered rows with a
--     kind ('open' | 'won' | 'lost'); deals (opportunities) link a
--     client and OPTIONALLY a quote, carry a value in the client-facing
--     currency, and move between stages. Every stage move appends an
--     immutable transition row (from → to, actor, note, time) — the
--     audit trail is the transition ledger, not a mutable column.
--
-- Non-purpose: NO FX conversion or currency generalization (Wave 12 lane
-- 10, ADR 0002) — every document is single-currency. NO tax engine: the
-- quote's tax is one flat rate (basis points) over the taxable subtotal;
-- the converted invoice reuses invoicing's line-sum total. NO UI (Wave 12
-- lane 11).
--
-- Additive-only: new tables plus one per-org sequence column; nothing
-- existing is rewritten or dropped.

ALTER TABLE orgs ADD COLUMN quote_sequence bigint NOT NULL DEFAULT 0;

-- --- catalog -----------------------------------------------------------------

CREATE TABLE catalog_items (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  -- Optional stock keeping unit; unique per org when present.
  sku text,
  description text,
  -- Unit of measure the price applies to ("each", "hour", "month", ...).
  unit text NOT NULL DEFAULT 'each',
  currency text NOT NULL,
  unit_price_minor bigint NOT NULL DEFAULT 0 CHECK (unit_price_minor >= 0),
  cost_minor bigint NOT NULL DEFAULT 0 CHECK (cost_minor >= 0),
  taxable boolean NOT NULL DEFAULT true,
  category text,
  group_name text,
  -- Archival is data: inactive items stay readable for quote history.
  active boolean NOT NULL DEFAULT true,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX uq_catalog_items_sku
  ON catalog_items (org_id, lower(sku))
  WHERE sku IS NOT NULL AND sku <> '';
CREATE INDEX idx_catalog_items_active ON catalog_items (org_id, active);
CREATE INDEX idx_catalog_items_category ON catalog_items (org_id, category);

-- --- quotes ------------------------------------------------------------------

CREATE TABLE quotes (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  number text NOT NULL,
  client_id uuid NOT NULL REFERENCES clients(id) ON DELETE RESTRICT,
  status text NOT NULL DEFAULT 'draft'
    CHECK (status IN ('draft','sent','accepted','declined','expired')),
  currency text NOT NULL,
  -- Validity/expiry window: the quote is convertible while today <=
  -- valid_until; the explicit expire leg refuses before that date.
  issued_on date NOT NULL DEFAULT CURRENT_DATE,
  valid_until date NOT NULL,
  notes text,
  terms text,
  -- Roll-ups (integer minor units). subtotal = sum(line amount_minor);
  -- tax = half-even tax_rate_bps over the taxable subtotal.
  subtotal_minor bigint NOT NULL DEFAULT 0 CHECK (subtotal_minor >= 0),
  tax_rate_bps integer NOT NULL DEFAULT 0 CHECK (tax_rate_bps >= 0 AND tax_rate_bps <= 10000),
  tax_minor bigint NOT NULL DEFAULT 0 CHECK (tax_minor >= 0),
  total_minor bigint NOT NULL DEFAULT 0 CHECK (total_minor >= 0),
  sent_at timestamptz,
  decided_at timestamptz,
  -- The convert-to-invoice seam's materialized targets (both NULL until
  -- conversion). The invoice is a DRAFT built through the existing
  -- invoicing persistence; contract_id is the seam into native invoicing.
  converted_contract_id uuid REFERENCES contracts(id) ON DELETE SET NULL,
  converted_invoice_id uuid REFERENCES invoices(id) ON DELETE SET NULL,
  converted_at timestamptz,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, number)
);
CREATE INDEX idx_quotes_client ON quotes (org_id, client_id);
CREATE INDEX idx_quotes_status ON quotes (org_id, status);

CREATE TABLE quote_lines (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  quote_id uuid NOT NULL REFERENCES quotes(id) ON DELETE CASCADE,
  -- Soft reference: catalog removal (SET NULL) never rewrites history.
  catalog_item_id uuid REFERENCES catalog_items(id) ON DELETE SET NULL,
  description text NOT NULL,
  unit text NOT NULL DEFAULT 'each',
  -- Quantity in thousandths of a unit (1.5 hours = 1500). Integer-only.
  quantity_milli bigint NOT NULL CHECK (quantity_milli > 0),
  unit_amount_minor bigint NOT NULL CHECK (unit_amount_minor >= 0),
  taxable boolean NOT NULL DEFAULT true,
  -- half-even(quantity_milli × unit_amount_minor / 1000).
  amount_minor bigint NOT NULL CHECK (amount_minor >= 0),
  position integer NOT NULL DEFAULT 0
);
CREATE INDEX idx_quote_lines_quote ON quote_lines (quote_id, position);

-- --- sales pipeline ----------------------------------------------------------

CREATE TABLE pipeline_stages (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  position integer NOT NULL DEFAULT 0,
  -- 'open' picks the default probability; 'won' / 'lost' are terminal.
  stage_type text NOT NULL DEFAULT 'open'
    CHECK (stage_type IN ('open','won','lost')),
  active boolean NOT NULL DEFAULT true,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, name)
);
CREATE INDEX idx_pipeline_stages_order ON pipeline_stages (org_id, position);

CREATE TABLE pipeline_deals (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  client_id uuid NOT NULL REFERENCES clients(id) ON DELETE RESTRICT,
  -- Optional originating/accepted quote.
  quote_id uuid REFERENCES quotes(id) ON DELETE SET NULL,
  stage_id uuid NOT NULL REFERENCES pipeline_stages(id) ON DELETE RESTRICT,
  name text NOT NULL,
  amount_minor bigint NOT NULL DEFAULT 0 CHECK (amount_minor >= 0),
  currency text NOT NULL,
  -- Mirrors the stage's kind: open | won | lost. Moved with the stage.
  status text NOT NULL DEFAULT 'open'
    CHECK (status IN ('open','won','lost')),
  expected_close_on date,
  owner_user_id uuid REFERENCES users(id) ON DELETE SET NULL,
  notes text,
  closed_at timestamptz,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_pipeline_deals_client ON pipeline_deals (org_id, client_id);
CREATE INDEX idx_pipeline_deals_stage ON pipeline_deals (org_id, stage_id);
CREATE INDEX idx_pipeline_deals_status ON pipeline_deals (org_id, status);

-- Append-only stage-move ledger: every transition is one immutable row.
CREATE TABLE pipeline_deal_transitions (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  deal_id uuid NOT NULL REFERENCES pipeline_deals(id) ON DELETE CASCADE,
  -- NULL for the initial placement recorded at deal creation.
  from_stage_id uuid REFERENCES pipeline_stages(id) ON DELETE SET NULL,
  to_stage_id uuid NOT NULL REFERENCES pipeline_stages(id) ON DELETE RESTRICT,
  note text,
  actor_id uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_pipeline_transitions_deal
  ON pipeline_deal_transitions (deal_id, created_at);

-- --- RLS ---------------------------------------------------------------------

ALTER TABLE catalog_items ENABLE ROW LEVEL SECURITY;
CREATE POLICY catalog_items_org ON catalog_items
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE quotes ENABLE ROW LEVEL SECURITY;
CREATE POLICY quotes_org ON quotes
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE quote_lines ENABLE ROW LEVEL SECURITY;
CREATE POLICY quote_lines_org ON quote_lines
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE pipeline_stages ENABLE ROW LEVEL SECURITY;
CREATE POLICY pipeline_stages_org ON pipeline_stages
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE pipeline_deals ENABLE ROW LEVEL SECURITY;
CREATE POLICY pipeline_deals_org ON pipeline_deals
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE pipeline_deal_transitions ENABLE ROW LEVEL SECURITY;
CREATE POLICY pipeline_deal_transitions_org ON pipeline_deal_transitions
  USING (org_id = current_setting('app.org_id', true)::uuid);
