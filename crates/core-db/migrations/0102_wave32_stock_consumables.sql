-- Stock + consumables core (Wave 32 lane 1, `p32-stock-consumables`;
-- migration 0102 is this lane's exclusive window; head was 0101).
--
-- Purpose: the MSP's OWN purchasing-side stock — what is currently MISSING
-- from the entity set (client configurations, the client's DeviceAsset
-- fleet, and catalog items all already exist and stay DECOUPLED). Laid out
-- per docs/instant-first.md operational-read-model doctrine:
--
--   * stock_items — org-owned SKUs. Linked to a catalog item where a
--     catalog line exists (soft ref: SET NULL) else free-form part
--     (name/part_number). `kind` carries the serialized-vs-consumable
--     distinction: a serialized item is ONE physical unit (tracked
--     per-serial in stock_serials — the seam Wave 32 lane 3 tools/loans
--     consumes); a consumable has only counts.
--
--   * stock_locations — typed physical places: warehouse/shelf/van/truck.
--     A van/truck MAY carry an assigned_user_id (whose vehicle it is),
--     optional; parents let a shelf nest under a warehouse.
--
--   * stock_levels — per (item, location) quantity + min/max + reorder
--     thresholds, all `*_milli` thousandths-integers consistent with quote
--     lines (floats never touch quantities). Level rows are the FRESH
--     snapshot the instant-first read path serves; the movement ledger is
--     the ground truth, the level is always its deterministic consequence
--     (the write path updates both in one commit).
--
--   * stock_movements — THE append-only movement ledger (the traceability
--     record every downstream checkout/loan/consumption/shipment row
--     references — later rows carry `movement_id`, never copy facts):
--       kind: 'receive' | 'consume' | 'adjust' | 'transfer' | 'ship_out'
--       receive:     to set,                      qty > 0, unit cost locked
--       consume:     from set,                     qty < 0 (on a job/ticket)
--       adjust:      from XOR to set (count fix),  qty can be ±
--       transfer:    from + to set,                qty > 0, cost carries
--       ship_out:    from set,                     qty < 0, optional carrier
--                    + tracking_ref, client (or a ticket) destination
--     ADR 0002 inventory-receipt FX lock moment: `receive` rows freeze
--     fx_rate_used / fx_rate_date / fx_source / base_unit_cost_minor at
--     receipt (computed by the API layer like every other money lock);
--     transfers stamp cost at send time; display conversions happen only
--     at read time. Ledger rows are immutable in the database (no UPDATE,
--     no DELETE — trigger-guarded, precedent ai_usage_ledger).
--
--   * stock_serials — serialized units: one row per physical serial.
--     status 'in_stock' | 'out'; movements stamp movement_in_id /
--     movement_out_id (append-only ledger stays the audit record; the
--     serial row is the mutable WHERE-IT-IS-NOW consequence). Wave 32
--     lane 3 (tools + client loans) consumes this table additively.
--
-- Non-purpose: NO purchase orders (that is a later lane's entities; this
-- migration only reserves the seam by keeping the ledger's carrier /
-- tracking_ref free of PO coupling), no UI (lane 4), no config-items
-- coupling (a stock item may later instantiate a DeviceAsset when
-- installed at a client: link, don't merge).
--
-- Additive-only: new tables; nothing existing is rewritten or dropped.

-- --- items -------------------------------------------------------------------

CREATE TABLE stock_items (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  -- Optional stock keeping unit; unique per org when present (same
  -- convention as catalog_items).
  sku text,
  -- Soft catalog link: absent catalog item stays as a free-form part.
  catalog_item_id uuid REFERENCES catalog_items(id) ON DELETE SET NULL,
  -- Free-form manufacturer/distributor part reference.
  part_number text,
  -- Serialized = one physical unit per serial; consumable = counts only.
  kind text NOT NULL DEFAULT 'consumable'
    CHECK (kind IN ('consumable','serialized')),
  -- Unit of measure at quantity_milli precision (parity with quote lines).
  unit text NOT NULL DEFAULT 'each' CHECK (unit IN ('each','box','meter')),
  currency text NOT NULL,
  -- Display convenience: the most recent receipt's locked unit cost.
  -- The ledger carries the authoritative per-receipt history.
  last_unit_cost_minor bigint CHECK (last_unit_cost_minor IS NULL OR last_unit_cost_minor >= 0),
  active boolean NOT NULL DEFAULT true,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX uq_stock_items_sku
  ON stock_items (org_id, lower(sku))
  WHERE sku IS NOT NULL AND sku <> '';
CREATE INDEX idx_stock_items_org ON stock_items (org_id, active);

-- --- locations ---------------------------------------------------------------

CREATE TABLE stock_locations (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  kind text NOT NULL CHECK (kind IN ('warehouse','shelf','van','truck')),
  -- Human-facing short code (BIN labels ride lane 5/6).
  code text,
  parent_id uuid REFERENCES stock_locations(id) ON DELETE SET NULL,
  -- Whose van/truck it is (optional; a tech's vehicle).
  assigned_user_id uuid REFERENCES users(id) ON DELETE SET NULL,
  active boolean NOT NULL DEFAULT true,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, name)
);
CREATE INDEX idx_stock_locations_org ON stock_locations (org_id, kind);

-- --- levels ------------------------------------------------------------------

CREATE TABLE stock_levels (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  stock_item_id uuid NOT NULL REFERENCES stock_items(id) ON DELETE CASCADE,
  stock_location_id uuid NOT NULL REFERENCES stock_locations(id) ON DELETE CASCADE,
  -- Thousandths of the item's unit (1.5 boxes = 1500). Integer-only.
  qty_milli bigint NOT NULL DEFAULT 0 CHECK (qty_milli >= 0),
  min_qty_milli bigint CHECK (min_qty_milli IS NULL OR min_qty_milli >= 0),
  max_qty_milli bigint CHECK (max_qty_milli IS NULL OR max_qty_milli >= 0),
  reorder_point_milli bigint CHECK (reorder_point_milli IS NULL OR reorder_point_milli >= 0),
  low_stock_notified_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, stock_item_id, stock_location_id)
);
CREATE INDEX idx_stock_levels_item ON stock_levels (stock_item_id);
CREATE INDEX idx_stock_levels_org ON stock_levels (org_id, stock_location_id);

-- --- serialized units --------------------------------------------------------

CREATE TABLE stock_serials (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  stock_item_id uuid NOT NULL REFERENCES stock_items(id) ON DELETE CASCADE,
  serial_number text NOT NULL,
  -- WHERE-IT-IS-NOW consequence of the ledger (in: receive/transfer-in,
  -- out: consume/transfer-out/ship_out). The movements table stays the
  -- immutable audit record; mutations stamp movement ids both ways.
  location_id uuid REFERENCES stock_locations(id) ON DELETE SET NULL,
  status text NOT NULL DEFAULT 'in_stock' CHECK (status IN ('in_stock','out')),
  movement_in_id uuid,
  movement_out_id uuid,
  ticket_id uuid REFERENCES tickets(id) ON DELETE SET NULL,
  client_id uuid REFERENCES clients(id) ON DELETE SET NULL,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX uq_stock_serials_number
  ON stock_serials (org_id, stock_item_id, lower(serial_number));
CREATE INDEX idx_stock_serials_location ON stock_serials (stock_item_id, location_id);
CREATE INDEX idx_stock_serials_status ON stock_serials (org_id, status);


ALTER TABLE stock_serials ENABLE ROW LEVEL SECURITY;
CREATE POLICY stock_serials_org ON stock_serials
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- --- movements (append-only ledger) -----------------------------------------

CREATE TABLE stock_movements (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  stock_item_id uuid NOT NULL REFERENCES stock_items(id) ON DELETE CASCADE,
  kind text NOT NULL
    CHECK (kind IN ('receive','consume','adjust','transfer','ship_out')),
  from_location_id uuid REFERENCES stock_locations(id) ON DELETE SET NULL,
  to_location_id uuid REFERENCES stock_locations(id) ON DELETE SET NULL,
  -- Signed delta in thousandths: + into stock, - out of stock.
  qty_milli bigint NOT NULL CHECK (qty_milli <> 0),
  -- Cost at the movement (receive: locked at receipt — ADR 0002
  -- inventory-receipt FX lock; transfer/ship_out stamp the carrying cost;
  -- consume stamps the carrying cost at time of consumption).
  cost_minor bigint,
  currency text,
  fx_rate_used text,
  fx_rate_date date,
  fx_source text,
  base_cost_minor bigint,
  -- Serialized seam (lane 3): a serialized unit's movement rows point at
  -- the serial; consumables leave these NULL.
  serial_id uuid REFERENCES stock_serials(id) ON DELETE SET NULL,
  -- Traceability anchors: ship_out → a client (fleet/loan delivery) and
  -- consume/ship_out → an on-site ticket job. Optional, never copied.
  client_id uuid REFERENCES clients(id) ON DELETE SET NULL,
  ticket_id uuid REFERENCES tickets(id) ON DELETE SET NULL,
  carrier text,
  tracking_ref text,
  note text CHECK (note IS NULL OR char_length(note) <= 500),
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_stock_movements_item_time
  ON stock_movements (org_id, stock_item_id, created_at DESC);
CREATE INDEX idx_stock_movements_time ON stock_movements (org_id, created_at DESC);
CREATE INDEX idx_stock_movements_ticket ON stock_movements (org_id, ticket_id)
  WHERE ticket_id IS NOT NULL;
CREATE INDEX idx_stock_movements_client ON stock_movements (org_id, client_id)
  WHERE client_id IS NOT NULL;

ALTER TABLE stock_movements ENABLE ROW LEVEL SECURITY;
CREATE POLICY stock_movements_org ON stock_movements
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- Low-stock alerts ride the notification center (kind `stock_low`;
-- 0081/0101 precedent: extend the kind checks as alert kinds land).
ALTER TABLE notifications DROP CONSTRAINT notifications_kind_check;
ALTER TABLE notifications ADD CONSTRAINT notifications_kind_check
  CHECK (kind IN ('mention','assignment','sla_risk','sla_breach','escalation',
                  'missing_time','ticket_update','report_delivery','ai_cap_warn',
                  'secret_rotation','dedupe_report','stock_low'));

ALTER TABLE notification_preferences DROP CONSTRAINT notification_preferences_kind_check;
ALTER TABLE notification_preferences ADD CONSTRAINT notification_preferences_kind_check
  CHECK (kind IN ('mention','assignment','sla_risk','sla_breach','escalation',
                  'missing_time','ticket_update','report_delivery','ai_cap_warn',
                  'secret_rotation','dedupe_report','stock_low'));

-- --- RLS on every readable table ---------------------------------------------

ALTER TABLE stock_items ENABLE ROW LEVEL SECURITY;
CREATE POLICY stock_items_org ON stock_items
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE stock_locations ENABLE ROW LEVEL SECURITY;
CREATE POLICY stock_locations_org ON stock_locations
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE stock_levels ENABLE ROW LEVEL SECURITY;
CREATE POLICY stock_levels_org ON stock_levels
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- --- append-only discipline (database-enforced) ------------------------------

CREATE FUNCTION stock_movements_append_only() RETURNS trigger AS $$
BEGIN
  RAISE EXCEPTION 'stock_movements is append-only: % rejected', TG_OP;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER stock_movements_append_only_guard
  BEFORE UPDATE OR DELETE ON stock_movements
  FOR EACH ROW EXECUTE FUNCTION stock_movements_append_only();
