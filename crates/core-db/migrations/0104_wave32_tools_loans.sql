-- Tools checkout + client loans (Wave 32 lane 3, `p32-tools-loans`;
-- migration 0104 is this lane's exclusive window; head was 0103 when the
-- lane branches merged — lane 2's 0103 reservation is untouched).
--
-- Purpose: two flows on top of lane 1's stock core (0102) and the asset
-- identity model (0061/ADR 0003):
--
--   * TOOLS + TOOLBOXES — a tool is a lane-1 stock item that is
--     `serialized` AND flagged `is_tool` (each physical tool is one
--     `stock_serials` row). A toolbox is a KIT (`tool_kits` +
--     `tool_kit_members`) whose member tools are checked out together, as
--     one transaction. Check-out/check-in ride the append-only ledger:
--     every leg is a `stock_movements` row (kinds 'checkout'/'checkin'),
--     and `tool_checkout_lines` references those rows — never copies
--     facts. Single-holder invariant ("one tool, one person at a time")
--     is DB-enforced by a partial unique index on open lines plus the
--     serial status gate (`checked_out`). Overdue is DERIVED at read time
--     (`returned_at IS NULL AND due_at < now()`), never a stale flag.
--
--   * CLIENT LOANS/RENTALS — devices issued to a client under an
--     agreement: `client_loans` (client, contract FK or free-text
--     agreement ref, out date, expected return, return condition) with
--     `client_loan_lines` per device. A stock-sourced line rides the
--     ledger ('loan'/'loan_return' movement legs) and on issue
--     instantiates a `device_assets` row in the client's fleet carrying
--     identity keys ({serial}) — the ADR 0003 dedupe path then converges
--     a later M365/RMM feed of the same phone onto that same asset
--     (serial signal = 0.95, above the auto-adopt threshold). When the
--     loan IS a lease the 0061 rental fields on the asset are stamped
--     from the lease terms (`is_lease` + `lease_cycle`/`lease_cost_minor`
--     /`lease_currency` on the loan; `rental_*` on the asset).
--
-- Additive only: new tables, new nullable/flagged columns, and CHECK
-- widenings (the sanctioned extension path from lane 1's landing record:
-- more movement kinds, richer serial statuses). Nothing existing is
-- rewritten or dropped; the ledger stays append-only.

-- 1. tools are serialized lane-1 stock items, flagged -------------------------

ALTER TABLE stock_items
  ADD COLUMN is_tool boolean NOT NULL DEFAULT false;
-- Serialized-only: a tool is individually tracked, counts alone are a
-- consumable's shape.
ALTER TABLE stock_items
  ADD CONSTRAINT stock_items_tool_serialized
  CHECK (NOT is_tool OR kind = 'serialized');

-- 2. richer WHERE-IT-IS-NOW vocabulary on serialized units --------------------
-- lane-1 values stay; 'checked_out' = with a holder (open tool checkout),
-- 'loaned' = issued to a client (open client-loan line). Both clear back
-- to 'in_stock' on the return leg.

ALTER TABLE stock_serials DROP CONSTRAINT stock_serials_status_check;
ALTER TABLE stock_serials ADD CONSTRAINT stock_serials_status_check
  CHECK (status IN ('in_stock', 'out', 'checked_out', 'loaned'));

-- 3. ledger kinds extended additively (direction discipline lives in the
-- domain validator + the write path; the DB check stays the gate) ----------

ALTER TABLE stock_movements DROP CONSTRAINT stock_movements_kind_check;
ALTER TABLE stock_movements ADD CONSTRAINT stock_movements_kind_check
  CHECK (kind IN ('receive', 'consume', 'adjust', 'transfer', 'ship_out',
                  'checkout', 'checkin', 'loan', 'loan_return'));

-- 4. toolboxes: a kit of member tools, checked out as one ---------------------

CREATE TABLE tool_kits (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  -- Optional human short code (stencil/shelf label).
  code text,
  note text CHECK (note IS NULL OR char_length(note) <= 500),
  active boolean NOT NULL DEFAULT true,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, name)
);
CREATE INDEX idx_tool_kits_org ON tool_kits (org_id, active);

CREATE TABLE tool_kit_members (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  kit_id uuid NOT NULL REFERENCES tool_kits(id) ON DELETE CASCADE,
  stock_item_id uuid NOT NULL REFERENCES stock_items(id) ON DELETE CASCADE,
  -- Milli counts (quote-line convention). Tools are serialized, so a
  -- member quantity is whole units of 1000 (validated in the domain).
  qty_milli bigint NOT NULL DEFAULT 1000 CHECK (qty_milli > 0),
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, kit_id, stock_item_id)
);
CREATE INDEX idx_tool_kit_members_item ON tool_kit_members (org_id, stock_item_id);

-- 5. check-out/check-in transactions ------------------------------------------

CREATE TABLE tool_checkouts (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- The single holder of everything in this transaction (the
  -- single-holder invariant itself is per physical unit, on the lines).
  holder_user_id uuid NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  -- Set when the checkout came from a kit (a toolbox checked out as one).
  kit_id uuid REFERENCES tool_kits(id) ON DELETE SET NULL,
  -- Where the tools came from; check-in may name a different destination
  -- on each leg, the movements carry the real from/to.
  from_location_id uuid NOT NULL REFERENCES stock_locations(id),
  due_at timestamptz NOT NULL,
  notes text CHECK (notes IS NULL OR char_length(notes) <= 500),
  status text NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'returned')),
  returned_at timestamptz,
  return_notes text CHECK (return_notes IS NULL OR char_length(return_notes) <= 500),
  -- Who performed the transaction (may differ from the holder).
  checked_out_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_tool_checkouts_holder ON tool_checkouts (org_id, holder_user_id, status);
CREATE INDEX idx_tool_checkouts_open_due ON tool_checkouts (org_id, due_at)
  WHERE returned_at IS NULL;

CREATE TABLE tool_checkout_lines (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  checkout_id uuid NOT NULL REFERENCES tool_checkouts(id) ON DELETE CASCADE,
  stock_item_id uuid NOT NULL REFERENCES stock_items(id) ON DELETE CASCADE,
  serial_id uuid NOT NULL REFERENCES stock_serials(id) ON DELETE CASCADE,
  -- The ledger legs this line rode: OUT ('checkout') always, IN
  -- ('checkin') once returned. The checkout row references the ledger;
  -- the ledger never references back and stays append-only.
  movement_out_id uuid NOT NULL REFERENCES stock_movements(id),
  movement_in_id uuid REFERENCES stock_movements(id),
  returned_at timestamptz,
  return_condition text
    CHECK (return_condition IS NULL OR return_condition IN ('good', 'fair', 'damaged')),
  return_notes text CHECK (return_notes IS NULL OR char_length(return_notes) <= 500),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
-- THE single-holder invariant, mechanically: one physical unit has at
-- most one OPEN checkout line. A unit frees up when its line returns.
CREATE UNIQUE INDEX uq_tool_checkout_open_unit
  ON tool_checkout_lines (org_id, serial_id) WHERE returned_at IS NULL;
CREATE INDEX idx_tool_checkout_lines_checkout ON tool_checkout_lines (org_id, checkout_id);

-- 6. client loans / rentals ----------------------------------------------------

CREATE TABLE client_loans (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  client_id uuid NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
  -- Agreement anchor: a contract row where one exists, else the free-form
  -- agreement/contract reference (scope wording: "contract/agreement ref").
  contract_id uuid REFERENCES contracts(id) ON DELETE SET NULL,
  agreement_ref text,
  out_at timestamptz NOT NULL DEFAULT now(),
  expected_return_at date NOT NULL,
  returned_at timestamptz,
  status text NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'returned')),
  -- Lease terms (0061 rental vocabulary) when the loan IS a lease; the
  -- issue leg stamps them onto the instantiated asset's rental fields.
  is_lease boolean NOT NULL DEFAULT false,
  lease_cycle text
    CHECK (lease_cycle IS NULL OR lease_cycle IN ('monthly', 'quarterly', 'annual')),
  lease_cost_minor bigint CHECK (lease_cost_minor IS NULL OR lease_cost_minor >= 0),
  lease_currency text,
  notes text CHECK (notes IS NULL OR char_length(notes) <= 500),
  return_notes text CHECK (return_notes IS NULL OR char_length(return_notes) <= 500),
  issued_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT client_loans_lease_terms CHECK (
    NOT is_lease
    OR (lease_cycle IS NOT NULL AND lease_cost_minor IS NOT NULL AND lease_currency IS NOT NULL)
  )
);
CREATE INDEX idx_client_loans_client ON client_loans (org_id, client_id, status);
CREATE INDEX idx_client_loans_open_due ON client_loans (org_id, expected_return_at)
  WHERE returned_at IS NULL;

CREATE TABLE client_loan_lines (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  loan_id uuid NOT NULL REFERENCES client_loans(id) ON DELETE CASCADE,
  -- Stock-sourced line: which org stock item + which physical serial left
  -- stock (both set together). Asset-sourced line: an asset already in the
  -- fleet being issued (stock pair NULL).
  stock_item_id uuid REFERENCES stock_items(id) ON DELETE SET NULL,
  serial_id uuid REFERENCES stock_serials(id) ON DELETE SET NULL,
  -- The device's row in the client's fleet: the asset we instantiated on
  -- issue (stock-sourced) or the asset that was issued (asset-sourced). A
  -- later M365/RMM feed converges onto THIS row via ADR 0003 identity
  -- keys, never a twin.
  device_asset_id uuid REFERENCES device_assets(id) ON DELETE SET NULL,
  -- Ledger legs: OUT ('loan') always for stock-sourced lines; IN
  -- ('loan_return') once returned. Asset-sourced lines have neither.
  movement_out_id uuid REFERENCES stock_movements(id),
  movement_in_id uuid REFERENCES stock_movements(id),
  returned_at timestamptz,
  return_condition text
    CHECK (return_condition IS NULL OR return_condition IN ('good', 'fair', 'damaged')),
  return_notes text CHECK (return_notes IS NULL OR char_length(return_notes) <= 500),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT client_loan_lines_source
    CHECK (stock_item_id IS NOT NULL OR device_asset_id IS NOT NULL),
  -- A stock-sourced line always carries its serial (loan devices are
  -- serialized physical units).
  CONSTRAINT client_loan_lines_serial_pair
    CHECK ((stock_item_id IS NULL) = (serial_id IS NULL))
);
-- Same single-issue invariant as tool checkouts, for client loans.
CREATE UNIQUE INDEX uq_client_loan_open_unit
  ON client_loan_lines (org_id, serial_id)
  WHERE returned_at IS NULL AND serial_id IS NOT NULL;
CREATE INDEX idx_client_loan_lines_loan ON client_loan_lines (org_id, loan_id);

-- 7. RLS on every new readable table ------------------------------------------

ALTER TABLE tool_kits ENABLE ROW LEVEL SECURITY;
CREATE POLICY tool_kits_org ON tool_kits
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE tool_kit_members ENABLE ROW LEVEL SECURITY;
CREATE POLICY tool_kit_members_org ON tool_kit_members
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE tool_checkouts ENABLE ROW LEVEL SECURITY;
CREATE POLICY tool_checkouts_org ON tool_checkouts
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE tool_checkout_lines ENABLE ROW LEVEL SECURITY;
CREATE POLICY tool_checkout_lines_org ON tool_checkout_lines
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE client_loans ENABLE ROW LEVEL SECURITY;
CREATE POLICY client_loans_org ON client_loans
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE client_loan_lines ENABLE ROW LEVEL SECURITY;
CREATE POLICY client_loan_lines_org ON client_loan_lines
  USING (org_id = current_setting('app.org_id', true)::uuid);
