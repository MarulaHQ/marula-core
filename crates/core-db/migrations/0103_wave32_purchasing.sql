-- Purchasing + vendors core (Wave 32 lane 2, `p32-purchasing`;
-- migration 0103 is this lane's exclusive window; head was 0102).
--
-- Purpose: the BUY side of lane 1's stock story. Lane 1 owns what the MSP
-- HAS (stock items/locations/levels + the append-only movement ledger);
-- this migration owns what the MSP ORDERS, from whom, and how received
-- goods become ledger rows and project cost:
--
--   * vendors — typed supplier/reseller entities: name, code, the
--     operator's contact channels (email/phone/website), reseller account
--     number, and the default currency for POs to this vendor.
--     `vendor_contacts` carries typed contact rows (name/email/phone/
--     role, one primary per vendor).
--
--   * external_links — typed external URL records (order_portal |
--     tracking | invoice | other) bound to a typed subject (vendor,
--     purchase_order, purchase_order_line, device_asset). This is the
--     asset integration-links pattern (document_links + LinkKind: typed
--     kind enum, org-scoped, subject-bound) applied to URLs OUTSIDE the
--     org — the reseller's order page or a parcel-tracking page opens in
--     one click from the PO. `device_asset` is a subject kind so the same
--     table serves the asset seam additively.
--
--   * purchase_orders — the PO lifecycle: draft → ordered →
--     partially_received → received → closed | cancelled. `project_id`
--     is the cost-object hook for project cost rollups; `vendor_reference`
--     carries the vendor's own order number for the reseller portal.
--     `close_kind` ('full' | 'short') is the typed UNDER-receipt record:
--     closing with unreceived quantity is explicit, never silent.
--
--   * purchase_order_lines — each line references a `stock_items` row
--     and/or a `catalog_items` row (free-form lines stay possible), with
--     `qty_ordered_milli` / `qty_received_milli` / `unit_cost_minor` in
--     integer milli/minor units. `receipt_state` (pending | partial |
--     complete | over) is the typed OVER-receipt record, maintained by
--     the receipt engine in the SAME commit as the ledger row it posts.
--
--   * purchase_receipts / purchase_receipt_lines — a receipt posts ONE
--     lane-1 `stock_movements` row (kind 'receive') per line at the
--     chosen `stock_locations` row and stamps `movement_id` here: the
--     ledger stays the immutable traceability record, the receipt is the
--     document that points at it. Partial receipts are ordinary rows.
--
--   * purchase_bills — the expense/bill hook: one bill per PO. Recording
--     is a draft ('recorded'); POSTING freezes the ADR 0002 FX snapshot
--     (same atomic column group as every other money fact) and is the
--     moment the purchase enters project cost rollups.
--
--   * device_assets — the MSP's OWN equipment rides the same DeviceAsset
--     lifecycle under a typed `scope` ('client' | 'internal_org').
--     Internal-org rows carry client_id NULL: org-scoped assets, never a
--     fake client record. Every existing row backfills scope='client'
--     (the CHECK below makes scope/client_id coherence structural).
--
-- Non-purpose: no UI (lane 4), no settings surface, no connector sync, no
-- shipping-carrier API integration (tracking links are operator-recorded
-- URLs, not live carrier queries), no config-items coupling (a stock item
-- may later instantiate a DeviceAsset when installed: link, don't merge).
--
-- Additive-only: new tables + additive column/constraint work on
-- device_assets (client_id NOT NULL dropped so internal-org rows exist;
-- existing data is valid under the new shape unchanged).

-- --- vendors -----------------------------------------------------------------

CREATE TABLE vendors (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL CHECK (name <> ''),
  -- Operator-facing short code; unique per org (case-insensitive) when
  -- present (same convention as stock item SKUs).
  code text,
  email text,
  phone text,
  website text,
  -- The vendor's account number for THIS org (deeplink/portal fuel).
  account_number text,
  -- ISO 4217 default for POs to this vendor.
  default_currency text NOT NULL DEFAULT 'USD',
  notes text CHECK (notes IS NULL OR char_length(notes) <= 2000),
  active boolean NOT NULL DEFAULT true,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX uq_vendors_code
  ON vendors (org_id, lower(code))
  WHERE code IS NOT NULL AND code <> '';
CREATE INDEX idx_vendors_org ON vendors (org_id, active);

CREATE TABLE vendor_contacts (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  vendor_id uuid NOT NULL REFERENCES vendors(id) ON DELETE CASCADE,
  name text NOT NULL CHECK (name <> ''),
  email text,
  phone text,
  -- Free text ("account manager", "billing", ...).
  role text,
  is_primary boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_vendor_contacts_vendor ON vendor_contacts (org_id, vendor_id);
-- One primary contact per vendor.
CREATE UNIQUE INDEX uq_vendor_contacts_primary
  ON vendor_contacts (vendor_id) WHERE is_primary;

-- --- typed external links ----------------------------------------------------

CREATE TABLE external_links (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- Typed subject (the document_links pattern: kind + id, no polymorphic
  -- FK — existence is checked by the write path).
  subject_kind text NOT NULL
    CHECK (subject_kind IN ('vendor','purchase_order','purchase_order_line','device_asset')),
  subject_id uuid NOT NULL,
  -- order_portal: the reseller's order page. tracking: a parcel-tracking
  -- page (carrier optional). invoice: the vendor's billing portal/PDF.
  kind text NOT NULL
    CHECK (kind IN ('order_portal','tracking','invoice','other')),
  url text NOT NULL CHECK (url <> ''),
  label text,
  -- Carrier that owns the tracking page; free text.
  carrier text,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_external_links_subject
  ON external_links (org_id, subject_kind, subject_id);
CREATE INDEX idx_external_links_kind ON external_links (org_id, kind);

-- --- purchase orders ---------------------------------------------------------

ALTER TABLE orgs ADD COLUMN purchase_order_sequence bigint NOT NULL DEFAULT 0;

CREATE TABLE purchase_orders (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  vendor_id uuid NOT NULL REFERENCES vendors(id) ON DELETE RESTRICT,
  -- Operator-facing number; unique per org (case-insensitive). Generated
  -- as PO-NNNNNN when the caller does not supply one.
  number text NOT NULL CHECK (number <> ''),
  status text NOT NULL DEFAULT 'draft'
    CHECK (status IN ('draft','ordered','partially_received','received','closed','cancelled')),
  -- ISO 4217; every line cost is in this currency (the receipt leg locks
  -- the FX snapshot per ADR 0002 when it differs from the org base).
  currency text NOT NULL,
  -- The vendor's own order reference (their portal shows this).
  vendor_reference text,
  -- Cost-object hook: a posted bill for this PO lands in this project's
  -- cost rollup. NULL = overhead / unassigned.
  project_id uuid REFERENCES projects(id) ON DELETE SET NULL,
  notes text CHECK (notes IS NULL OR char_length(notes) <= 4000),
  ordered_at timestamptz,
  expected_at date,
  -- Set on close. close_kind = 'short' records an under-receipt close
  -- explicitly (never a silent drop of unreceived quantity).
  closed_at timestamptz,
  close_kind text CHECK (close_kind IN ('full','short')),
  cancelled_at timestamptz,
  cancel_reason text,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT purchase_orders_close_shape CHECK (
    (status = 'closed') = (closed_at IS NOT NULL)
    AND (status = 'cancelled') = (cancelled_at IS NOT NULL)
    AND (close_kind IS NULL OR status = 'closed')
  )
);
CREATE UNIQUE INDEX uq_purchase_orders_number
  ON purchase_orders (org_id, lower(number));
CREATE INDEX idx_purchase_orders_org
  ON purchase_orders (org_id, status, created_at DESC);
CREATE INDEX idx_purchase_orders_vendor ON purchase_orders (org_id, vendor_id);
CREATE INDEX idx_purchase_orders_project
  ON purchase_orders (org_id, project_id) WHERE project_id IS NOT NULL;

CREATE TABLE purchase_order_lines (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  purchase_order_id uuid NOT NULL REFERENCES purchase_orders(id) ON DELETE CASCADE,
  -- Receiving posts into stock THROUGH a stock item; a catalog-only line
  -- is fine for purchasing but cannot receive into the ledger (the API
  -- answers a typed 409 instead of inventing an item).
  stock_item_id uuid REFERENCES stock_items(id) ON DELETE SET NULL,
  catalog_item_id uuid REFERENCES catalog_items(id) ON DELETE SET NULL,
  description text NOT NULL CHECK (description <> ''),
  qty_ordered_milli bigint NOT NULL CHECK (qty_ordered_milli > 0),
  qty_received_milli bigint NOT NULL DEFAULT 0 CHECK (qty_received_milli >= 0),
  unit_cost_minor bigint NOT NULL CHECK (unit_cost_minor >= 0),
  -- Typed receipt consequence, maintained by the receipt engine in the
  -- same commit as the ledger row: pending | partial | complete | over.
  receipt_state text NOT NULL DEFAULT 'pending'
    CHECK (receipt_state IN ('pending','partial','complete','over')),
  position integer NOT NULL DEFAULT 0,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_purchase_order_lines_order
  ON purchase_order_lines (org_id, purchase_order_id, position);
CREATE INDEX idx_purchase_order_lines_stock_item
  ON purchase_order_lines (stock_item_id) WHERE stock_item_id IS NOT NULL;

CREATE TABLE purchase_receipts (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  purchase_order_id uuid NOT NULL REFERENCES purchase_orders(id) ON DELETE CASCADE,
  -- Where the goods landed (the lane-1 location the levels moved at).
  location_id uuid NOT NULL REFERENCES stock_locations(id),
  carrier text,
  tracking_ref text,
  note text CHECK (note IS NULL OR char_length(note) <= 500),
  received_at timestamptz NOT NULL DEFAULT now(),
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_purchase_receipts_order
  ON purchase_receipts (org_id, purchase_order_id, received_at DESC);

CREATE TABLE purchase_receipt_lines (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  receipt_id uuid NOT NULL REFERENCES purchase_receipts(id) ON DELETE CASCADE,
  po_line_id uuid NOT NULL REFERENCES purchase_order_lines(id) ON DELETE CASCADE,
  qty_milli bigint NOT NULL CHECK (qty_milli > 0),
  -- THE lane-1 ledger row this receipt line posted (append-only
  -- stock_movements). Read-only reference; the ledger is ground truth.
  movement_id uuid REFERENCES stock_movements(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_purchase_receipt_lines_receipt
  ON purchase_receipt_lines (org_id, receipt_id);
CREATE INDEX idx_purchase_receipt_lines_po_line ON purchase_receipt_lines (po_line_id);
CREATE INDEX idx_purchase_receipt_lines_movement
  ON purchase_receipt_lines (movement_id) WHERE movement_id IS NOT NULL;

CREATE TABLE purchase_bills (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- One bill per PO in v1 (partial billing is additive later).
  purchase_order_id uuid NOT NULL UNIQUE REFERENCES purchase_orders(id) ON DELETE CASCADE,
  -- The vendor's invoice number.
  bill_number text,
  amount_minor bigint NOT NULL CHECK (amount_minor >= 0),
  currency text NOT NULL,
  -- ADR 0002 lock at POSTING (fx_rate_used is BASE major units per one
  -- FACT major unit; the group is atomic — all set or all NULL).
  fx_rate_used numeric(24,12) CHECK (fx_rate_used IS NULL OR fx_rate_used > 0),
  fx_rate_date date,
  fx_source text,
  base_amount_minor bigint CHECK (base_amount_minor IS NULL OR base_amount_minor >= 0),
  -- Posting is the rollup moment; NULL = recorded, not yet in rollups.
  posted_at timestamptz,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT purchase_bills_fx_snapshot_atomic CHECK (
    (fx_rate_used IS NULL AND fx_rate_date IS NULL AND fx_source IS NULL
      AND base_amount_minor IS NULL)
    OR
    (fx_rate_used IS NOT NULL AND fx_rate_date IS NOT NULL AND fx_source IS NOT NULL
      AND base_amount_minor IS NOT NULL)
  )
);
CREATE INDEX idx_purchase_bills_posted
  ON purchase_bills (org_id, posted_at) WHERE posted_at IS NOT NULL;

-- --- internal-org asset scope ------------------------------------------------

-- The MSP's own equipment rides the SAME DeviceAsset lifecycle. `scope`
-- is the typed discriminator; the CHECK below ties it to client_id: a
-- client-scoped asset always carries a client, an internal-org asset
-- never does (no fake client record).
ALTER TABLE device_assets
  ADD COLUMN scope text NOT NULL DEFAULT 'client'
    CHECK (scope IN ('client','internal_org'));

ALTER TABLE device_assets ALTER COLUMN client_id DROP NOT NULL;

ALTER TABLE device_assets
  ADD CONSTRAINT device_assets_scope_client_shape
  CHECK ((scope = 'internal_org') = (client_id IS NULL));

CREATE INDEX idx_assets_org_scope ON device_assets (org_id, scope);

-- --- RLS on every new table --------------------------------------------------

ALTER TABLE vendors ENABLE ROW LEVEL SECURITY;
CREATE POLICY vendors_org ON vendors
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE vendor_contacts ENABLE ROW LEVEL SECURITY;
CREATE POLICY vendor_contacts_org ON vendor_contacts
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE external_links ENABLE ROW LEVEL SECURITY;
CREATE POLICY external_links_org ON external_links
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE purchase_orders ENABLE ROW LEVEL SECURITY;
CREATE POLICY purchase_orders_org ON purchase_orders
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE purchase_order_lines ENABLE ROW LEVEL SECURITY;
CREATE POLICY purchase_order_lines_org ON purchase_order_lines
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE purchase_receipts ENABLE ROW LEVEL SECURITY;
CREATE POLICY purchase_receipts_org ON purchase_receipts
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE purchase_receipt_lines ENABLE ROW LEVEL SECURITY;
CREATE POLICY purchase_receipt_lines_org ON purchase_receipt_lines
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE purchase_bills ENABLE ROW LEVEL SECURITY;
CREATE POLICY purchase_bills_org ON purchase_bills
  USING (org_id = current_setting('app.org_id', true)::uuid);
