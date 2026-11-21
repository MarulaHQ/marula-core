-- Purchase intent, receipt routing and client recharge (ADR 0035 decisions 2
-- and 3, plan §5). Migration 0235 continues the window 0232-0234 opened: the
-- identity and the vocabulary exist there, and this file is where a purchase
-- order states what it buys and where the goods must land.
--
--   * `purchase_orders` gains the header intent: what the purchase is for
--     (`acquired_for`), the client or ticket it serves, the ownership word for
--     a rented unit, the delivery mode, and an optional recharge rule. The
--     header carries defaults; a line overrides the destination, never the
--     intent the money is recharged under (plan §5.1).
--
--   * `purchase_order_lines` gains the line intent: what one line buys
--     (`line_kind`), its destination class when it differs from the header's,
--     the asset layout a receipt should instantiate, whether a serial is
--     expected, the unit weight, the sell price and the billable flag.
--     `destination_class IS NULL` means "inherit the header intent", which is
--     what makes a draft react to a header change instead of freezing a
--     resolution the operator never made.
--
--   * `purchase_receipt_lines` names its one consequence: the destination
--     class it landed in and, when the consequence was an asset row, the asset
--     it created. A stock consequence already names its ledger row through
--     `movement_id`; the pair is the whole chain in §4.3 for one receipt line.
--
--   * `purchase_recharge_rule_overrides` is the ticket/project/client rung of
--     the resolution order in §5.3, so "ticket or project overrides client
--     which overrides the organisation default" is a lookup rather than a
--     convention. One rule per scope row per org.
--
--   * `purchase_recharges` is the issued recharge: the resolved rule, the
--     basis it was applied to, the cost and charge currencies with both FX
--     snapshots, the cost line it produced, and the receipt line it came from.
--     It is append-only, because an issued recharge is never recomputed —
--     corrections are credit notes (ADR 0035 §6).
--
-- Every column is nullable or defaulted to today's meaning, and every
-- constraint is satisfied by an existing row: a purchase order that says
-- nothing is `sale_stock`, delivered `to_stock`, with no client, no ticket and
-- no recharge rule. No behaviour changes until a caller writes the new words.
--
-- Lock cost: the defaulted columns are metadata-only (Postgres 11+), and the
-- tables that gain CHECK constraints — `purchase_orders`, `purchase_order_lines`
-- and `purchase_receipt_lines` — hold thousands of rows at fleet sizes, so each
-- validation scan is a statement rather than an outage. Indexes are plain (not
-- CONCURRENTLY) because sqlx runs a migration inside a transaction, where the
-- concurrent form is illegal; the same note applies to 0232.

-- --- header intent ------------------------------------------------------------

ALTER TABLE purchase_orders
  ADD COLUMN acquired_for text NOT NULL DEFAULT 'sale_stock',
  ADD COLUMN client_id uuid REFERENCES clients(id) ON DELETE SET NULL,
  ADD COLUMN ticket_id uuid REFERENCES tickets(id) ON DELETE SET NULL,
  ADD COLUMN ownership text,
  ADD COLUMN delivery_mode text NOT NULL DEFAULT 'to_stock',
  ADD COLUMN recharge_rule_kind text,
  ADD COLUMN recharge_markup_bps integer,
  ADD COLUMN recharge_fixed_minor bigint,
  ADD COLUMN recharge_currency text;

ALTER TABLE purchase_orders
  ADD CONSTRAINT purchase_orders_acquired_for_check
  CHECK (acquired_for IN ('sale_stock', 'internal_fleet', 'rental_pool',
                          'vendor_rental', 'client_stocked', 'client_drop_ship',
                          'project_consumable', 'ewaste_service')),
  ADD CONSTRAINT purchase_orders_delivery_mode_check
  CHECK (delivery_mode IN ('to_stock', 'drop_ship_to_client', 'to_site')),
  ADD CONSTRAINT purchase_orders_ownership_check
  CHECK (ownership IS NULL OR ownership IN ('owned', 'leased', 'rented'));

-- A purchase made on a client's behalf names the client. The implication runs
-- one way on purpose: an order for our own stock may still name a client,
-- because §5.1 lets one purchase order mix intents and a line decides where it
-- lands. The line-level rule (a client-custody line needs a client) is enforced
-- where both words are known.
ALTER TABLE purchase_orders
  ADD CONSTRAINT purchase_orders_client_shape
  CHECK (acquired_for NOT IN ('client_stocked', 'client_drop_ship')
         OR client_id IS NOT NULL);

-- A drop-ship delivers to the client, and only a client purchase can drop-ship.
-- Written as two implications so NULL never makes the comparison vacuous.
ALTER TABLE purchase_orders
  ADD CONSTRAINT purchase_orders_drop_ship_needs_client
  CHECK (delivery_mode <> 'drop_ship_to_client'
         OR acquired_for IN ('client_stocked', 'client_drop_ship')),
  ADD CONSTRAINT purchase_orders_client_drop_ship_shape
  CHECK (acquired_for <> 'client_drop_ship' OR delivery_mode = 'drop_ship_to_client');

-- A rented unit is a vendor rental and a vendor rental is rented; every other
-- intent is owned or leased by us.
ALTER TABLE purchase_orders
  ADD CONSTRAINT purchase_orders_vendor_rental_shape
  CHECK ((acquired_for = 'vendor_rental' AND ownership = 'rented')
         OR (acquired_for <> 'vendor_rental' AND ownership IS DISTINCT FROM 'rented'));

-- The header rule is optional; when present it must be a complete rule of one
-- kind, and it exists only for a client purchase.
ALTER TABLE purchase_orders
  ADD CONSTRAINT purchase_orders_recharge_rule_shape
  CHECK (
    CASE
      WHEN recharge_rule_kind IS NULL THEN
        recharge_markup_bps IS NULL AND recharge_fixed_minor IS NULL
        AND recharge_currency IS NULL
      WHEN recharge_rule_kind IN ('at_cost', 'none') THEN
        recharge_markup_bps IS NULL AND recharge_fixed_minor IS NULL
        AND recharge_currency IS NULL
      WHEN recharge_rule_kind = 'markup_bps' THEN
        recharge_markup_bps IS NOT NULL AND recharge_markup_bps BETWEEN 0 AND 100000
        AND recharge_fixed_minor IS NULL AND recharge_currency IS NULL
      WHEN recharge_rule_kind = 'fixed' THEN
        recharge_fixed_minor IS NOT NULL AND recharge_fixed_minor >= 0
        AND recharge_currency IS NOT NULL AND recharge_currency ~ '^[A-Z]{3}$'
        AND recharge_markup_bps IS NULL
      ELSE false
    END
  ),
  ADD CONSTRAINT purchase_orders_recharge_needs_client
  CHECK (recharge_rule_kind IS NULL
         OR acquired_for IN ('client_stocked', 'client_drop_ship'));

-- Foreign keys are not indexed by Postgres, and both of these are read by a
-- client or ticket page.
CREATE INDEX idx_purchase_orders_client
  ON purchase_orders (org_id, client_id) WHERE client_id IS NOT NULL;
CREATE INDEX idx_purchase_orders_ticket
  ON purchase_orders (org_id, ticket_id) WHERE ticket_id IS NOT NULL;

-- --- line intent --------------------------------------------------------------

ALTER TABLE purchase_order_lines
  ADD COLUMN line_kind text NOT NULL DEFAULT 'stock',
  ADD COLUMN destination_class text,
  ADD COLUMN asset_layout_id uuid REFERENCES document_layouts(id) ON DELETE SET NULL,
  ADD COLUMN serial_expected boolean NOT NULL DEFAULT false,
  ADD COLUMN weight_g integer,
  ADD COLUMN sell_price_minor bigint,
  ADD COLUMN billable boolean NOT NULL DEFAULT false;

ALTER TABLE purchase_order_lines
  ADD CONSTRAINT purchase_order_lines_line_kind_check
  CHECK (line_kind IN ('stock', 'serialized_unit', 'asset', 'rental_unit',
                       'service', 'pass_through')),
  -- A receipt destination is a live class; terminal dispositions are reached by
  -- a movement after the unit exists, never by a purchase.
  ADD CONSTRAINT purchase_order_lines_destination_class_check
  CHECK (destination_class IS NULL
         OR destination_class IN ('sale_stock', 'internal_fleet', 'rental_in',
                                  'rental_out', 'client_custody',
                                  'ewaste_custody', 'tools')),
  ADD CONSTRAINT purchase_order_lines_weight_check
  CHECK (weight_g IS NULL OR weight_g > 0),
  ADD CONSTRAINT purchase_order_lines_sell_price_check
  CHECK (sell_price_minor IS NULL OR sell_price_minor >= 0);

-- The serial and pass-through rules the domain validates, spelled once at the
-- schema so a writer that bypasses the domain cannot land a line the receipt
-- engine would have to refuse. A line that inherits its destination from the
-- header is checked by the receipt engine, which is the only place both words
-- are known.
ALTER TABLE purchase_order_lines
  ADD CONSTRAINT purchase_order_lines_serial_shape
  CHECK (NOT serial_expected
         OR line_kind IN ('serialized_unit', 'asset', 'rental_unit')),
  ADD CONSTRAINT purchase_order_lines_serialized_expects_serial
  CHECK (line_kind <> 'serialized_unit' OR serial_expected),
  ADD CONSTRAINT purchase_order_lines_pass_through_shape
  CHECK (destination_class IS NULL
         OR line_kind NOT IN ('pass_through', 'service')
         OR destination_class NOT IN ('sale_stock', 'internal_fleet',
                                      'rental_out', 'tools'));

-- --- the receipt line's one consequence ---------------------------------------

ALTER TABLE purchase_receipt_lines
  ADD COLUMN destination_class text,
  ADD COLUMN device_asset_id uuid REFERENCES device_assets(id) ON DELETE SET NULL;

ALTER TABLE purchase_receipt_lines
  ADD CONSTRAINT purchase_receipt_lines_destination_class_check
  CHECK (destination_class IS NULL
         OR destination_class IN ('sale_stock', 'internal_fleet', 'rental_in',
                                  'rental_out', 'client_custody',
                                  'ewaste_custody', 'tools'));

CREATE INDEX idx_purchase_receipt_lines_asset
  ON purchase_receipt_lines (org_id, device_asset_id)
  WHERE device_asset_id IS NOT NULL;

-- --- recharge rule overrides --------------------------------------------------

-- The ticket/project/client rung of plan §5.3. A null row means "nothing
-- configured here", which is why the table holds overrides only: the
-- organisation default lives on `orgs` (migration 0234) and never needs a copy.
CREATE TABLE purchase_recharge_rule_overrides (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- ticket and project are the first rung; client is the second. An agreement
  -- is reached through its client, so it needs no rung of its own.
  scope_kind text NOT NULL CHECK (scope_kind IN ('ticket', 'project', 'client')),
  scope_id uuid NOT NULL,
  rule_kind text NOT NULL
    CHECK (rule_kind IN ('at_cost', 'markup_bps', 'fixed', 'none')),
  markup_bps integer CHECK (markup_bps IS NULL OR markup_bps BETWEEN 0 AND 100000),
  fixed_minor bigint CHECK (fixed_minor IS NULL OR fixed_minor >= 0),
  currency text CHECK (currency IS NULL OR currency ~ '^[A-Z]{3}$'),
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT purchase_recharge_rule_overrides_shape CHECK (
    CASE rule_kind
      WHEN 'markup_bps' THEN markup_bps IS NOT NULL
                            AND fixed_minor IS NULL AND currency IS NULL
      WHEN 'fixed' THEN fixed_minor IS NOT NULL AND currency IS NOT NULL
                       AND markup_bps IS NULL
      ELSE markup_bps IS NULL AND fixed_minor IS NULL AND currency IS NULL
    END
  )
);

-- Indexes, not table constraints, for both uniqueness guards below: neither
-- key is user input (a scope row is one row by construction, and a receipt line
-- recharges once), so a collision is a writer bug that surfaces as a sanitised
-- 409 rather than a message — the same posture migration 0233 records for the
-- serial/asset pair.
CREATE UNIQUE INDEX uq_purchase_recharge_rule_overrides_scope
  ON purchase_recharge_rule_overrides (org_id, scope_kind, scope_id);

ALTER TABLE purchase_recharge_rule_overrides ENABLE ROW LEVEL SECURITY;
CREATE POLICY purchase_recharge_rule_overrides_org ON purchase_recharge_rule_overrides
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- --- the issued recharge ------------------------------------------------------

-- One row per receipt line that was recharged to a client. Everything needed to
-- explain the amount is stored beside it, so a later policy change cannot
-- rewrite an issued recharge: the rule, the source of the rule, the basis and
-- currency it was applied to, the charge and currency it produced, and the FX
-- snapshots of both sides.
CREATE TABLE purchase_recharges (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  purchase_order_id uuid NOT NULL REFERENCES purchase_orders(id) ON DELETE CASCADE,
  po_line_id uuid NOT NULL REFERENCES purchase_order_lines(id) ON DELETE CASCADE,
  receipt_line_id uuid NOT NULL REFERENCES purchase_receipt_lines(id) ON DELETE CASCADE,
  client_id uuid REFERENCES clients(id) ON DELETE SET NULL,
  ticket_id uuid REFERENCES tickets(id) ON DELETE SET NULL,
  project_id uuid REFERENCES projects(id) ON DELETE SET NULL,
  rule_kind text NOT NULL
    CHECK (rule_kind IN ('at_cost', 'markup_bps', 'fixed', 'none')),
  -- Which rung produced the rule. 'purchase_order' is the explicit statement on
  -- the order; the rest are the ladder in plan §5.3.
  rule_source text NOT NULL
    CHECK (rule_source IN ('purchase_order', 'ticket', 'project', 'client',
                           'organisation')),
  rule_markup_bps integer,
  rule_fixed_minor bigint,
  rule_currency text,
  -- The cost the rule was applied to, in the cost line's own currency.
  basis_amount_minor bigint NOT NULL CHECK (basis_amount_minor >= 0),
  basis_currency text NOT NULL CHECK (basis_currency ~ '^[A-Z]{3}$'),
  cost_fx_rate_used numeric(24,12) CHECK (cost_fx_rate_used IS NULL OR cost_fx_rate_used > 0),
  cost_fx_rate_date date,
  cost_fx_source text,
  cost_base_amount_minor bigint CHECK (cost_base_amount_minor IS NULL
                                       OR cost_base_amount_minor >= 0),
  charge_amount_minor bigint NOT NULL CHECK (charge_amount_minor >= 0),
  charge_currency text NOT NULL CHECK (charge_currency ~ '^[A-Z]{3}$'),
  charge_fx_rate_used numeric(24,12)
    CHECK (charge_fx_rate_used IS NULL OR charge_fx_rate_used > 0),
  charge_fx_rate_date date,
  charge_fx_source text,
  charge_base_amount_minor bigint CHECK (charge_base_amount_minor IS NULL
                                         OR charge_base_amount_minor >= 0),
  -- The billable cost line the charge landed on. Null when the purchase named no
  -- ticket; the invoice join reaches the receipt line directly then.
  --
  -- No foreign key, on purpose: this table is append-only, and an
  -- `ON DELETE SET NULL` would be an update the guard has to reject. The cost
  -- line is a cited document, not a parent — the same posture as
  -- `ticket_cost_lines.source_id` and `inventory_class_movements.source_id`.
  cost_line_id uuid,
  issued_at timestamptz NOT NULL DEFAULT now(),
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT purchase_recharges_rule_shape CHECK (
    CASE rule_kind
      WHEN 'markup_bps' THEN rule_markup_bps IS NOT NULL
                            AND rule_fixed_minor IS NULL AND rule_currency IS NULL
      WHEN 'fixed' THEN rule_fixed_minor IS NOT NULL AND rule_currency IS NOT NULL
                       AND rule_markup_bps IS NULL
      ELSE rule_markup_bps IS NULL AND rule_fixed_minor IS NULL
           AND rule_currency IS NULL
    END
  ),
  CONSTRAINT purchase_recharges_cost_fx_atomic CHECK (
    (cost_fx_rate_used IS NULL AND cost_fx_rate_date IS NULL
      AND cost_fx_source IS NULL AND cost_base_amount_minor IS NULL)
    OR
    (cost_fx_rate_used IS NOT NULL AND cost_fx_rate_date IS NOT NULL
      AND cost_fx_source IS NOT NULL AND cost_base_amount_minor IS NOT NULL)
  ),
  CONSTRAINT purchase_recharges_charge_fx_atomic CHECK (
    (charge_fx_rate_used IS NULL AND charge_fx_rate_date IS NULL
      AND charge_fx_source IS NULL AND charge_base_amount_minor IS NULL)
    OR
    (charge_fx_rate_used IS NOT NULL AND charge_fx_rate_date IS NOT NULL
      AND charge_fx_source IS NOT NULL AND charge_base_amount_minor IS NOT NULL)
  ),
  -- A rule that recharges nothing is not stored as a charge.
  CONSTRAINT purchase_recharges_issues CHECK (rule_kind <> 'none')
);

CREATE INDEX idx_purchase_recharges_order
  ON purchase_recharges (org_id, purchase_order_id, issued_at DESC);
-- One receipt line recharges once: a replayed receipt cannot bill twice.
CREATE UNIQUE INDEX uq_purchase_recharges_receipt_line
  ON purchase_recharges (org_id, receipt_line_id);
CREATE INDEX idx_purchase_recharges_client
  ON purchase_recharges (org_id, client_id) WHERE client_id IS NOT NULL;
CREATE INDEX idx_purchase_recharges_ticket
  ON purchase_recharges (org_id, ticket_id) WHERE ticket_id IS NOT NULL;
CREATE INDEX idx_purchase_recharges_cost_line
  ON purchase_recharges (cost_line_id) WHERE cost_line_id IS NOT NULL;

ALTER TABLE purchase_recharges ENABLE ROW LEVEL SECURITY;
CREATE POLICY purchase_recharges_org ON purchase_recharges
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- An issued recharge is a fact. Correcting it is a credit note, not an edit.
CREATE FUNCTION purchase_recharges_append_only() RETURNS trigger AS $$
BEGIN
  RAISE EXCEPTION 'purchase_recharges is append-only: % rejected', TG_OP;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER purchase_recharges_append_only_guard
  BEFORE UPDATE OR DELETE ON purchase_recharges
  FOR EACH ROW EXECUTE FUNCTION purchase_recharges_append_only();
