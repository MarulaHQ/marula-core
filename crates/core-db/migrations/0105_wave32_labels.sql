-- Label templates (Wave 32 lane 5, `p32-labels-scanning`; migration 0105 is
-- this lane's exclusive window; head was 0104).
--
-- Purpose: typed, org-scoped label templates per print purpose (stock item,
-- location bin, serialized tool/unit, loan device) that render
-- deterministically to ONE label model two ways: the browser print dialog
-- (everywhere) and raw ZPL for networked Zebra printers (desktop shell).
--
-- Why a table and not a settings key: the settings registry explicitly
-- carries three scalar payload types ("richer structures are entities, not
-- settings" — crates/core-domain/src/settings.rs). A template is an ordered
-- token list + size + symbology config, so it is an entity: own endpoints,
-- rendered under the settings IA.
--
-- Field tokens are stored as an ordered JSON string array (max 6 lines);
-- the domain layer validates membership against the purpose's allowed set
-- (crates/core-domain/src/entities/labels.rs). The QR payload is ALWAYS the
-- entity's canonical deep link (`meridian://<kind>/<uuid>`) so a scan means
-- one thing on every surface — no per-template QR mode that could drift.
--
-- Additive only: one new table + its RLS policy. Nothing existing changes.

CREATE TABLE label_templates (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL CHECK (char_length(btrim(name)) BETWEEN 1 AND 80),
  purpose text NOT NULL CHECK (purpose IN ('stock_item', 'stock_location', 'serialized_unit', 'device_asset')),
  -- Common Zebra (203/300/600 dpi thermal) and DYMO LabelWriter stock sizes,
  -- plus a custom size. Dimensions live in tenths of a millimetre so the
  -- print targets stay integer-exact (no float ever reaches the renderer).
  size_preset text NOT NULL CHECK (size_preset IN (
    'zebra_2x1', 'zebra_3x1', 'zebra_4x6',
    'dymo_address', 'dymo_multipurpose', 'dymo_square', 'custom')),
  width_tenths_mm integer CHECK (width_tenths_mm IS NULL OR width_tenths_mm BETWEEN 100 AND 3000),
  height_tenths_mm integer CHECK (height_tenths_mm IS NULL OR height_tenths_mm BETWEEN 100 AND 3000),
  -- 'custom' requires both dimensions; a preset must not carry them.
  CONSTRAINT label_templates_custom_size CHECK (
    (size_preset = 'custom') = (width_tenths_mm IS NOT NULL AND height_tenths_mm IS NOT NULL)),
  dpi integer NOT NULL DEFAULT 203 CHECK (dpi IN (203, 300, 600)),
  -- The token the Code128 symbology encodes; NULL = no barcode on this label.
  barcode_token text CHECK (barcode_token IS NULL OR barcode_token IN ('sku', 'part_number', 'serial', 'code', 'tag')),
  -- QR is always-on/off; when on it encodes the entity deep link.
  include_qr boolean NOT NULL DEFAULT true,
  -- Ordered text lines (token names), max 6; validated per purpose in the domain.
  fields jsonb NOT NULL DEFAULT '[]'::jsonb CHECK (
    jsonb_typeof(fields) = 'array' AND jsonb_array_length(fields) <= 6),
  -- One active default per (org, purpose): partial unique index below.
  is_default boolean NOT NULL DEFAULT false,
  active boolean NOT NULL DEFAULT true,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, name)
);

CREATE INDEX idx_label_templates_org ON label_templates (org_id, purpose, active);

-- One default template per purpose per org (active rows only; archiving the
-- default frees the slot deterministically).
CREATE UNIQUE INDEX uq_label_templates_default
  ON label_templates (org_id, purpose) WHERE is_default AND active;

ALTER TABLE label_templates ENABLE ROW LEVEL SECURITY;
CREATE POLICY label_templates_org ON label_templates
  USING (org_id = current_setting('app.org_id', true)::uuid);
