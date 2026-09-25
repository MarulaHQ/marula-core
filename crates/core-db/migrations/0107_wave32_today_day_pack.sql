-- The tech's day pack (Wave 32 lane 8, `p32-today-day-pack`; migration 0107
-- is this lane's exclusive window; head was 0106).
--
-- Purpose: the trip-preparation layer on `/today`. Three small org-scoped
-- tables carry day-pack INTENT, and one CHECK widens the label purposes so a
-- toolbox tag prints and scans back to its kit.
--
--   * `ticket_stock_picks` — manual per-ticket picks: a tech or stockroom
--     operator attaches a stock item to a ticket ("this job needs the
--     punch-down tool and 2 RJ45 ends"). A pick RESERVES nothing and moves
--     nothing — the movement ledger stays honest, and collection happens
--     later through the lane-3 checkout transactions or a lane-1
--     consume/transfer movement.
--
--   * `dispatch_pack_rules` — the typed org mapping behind recommended tools
--     and consumables. A scope (ticket type / ticket category / board) maps
--     either to a stock item (a recommendation) or to a kit (the default-kit
--     mapping: "dispatch the field kit with every network ticket"). The
--     settings registry carries scalar values only BY DESIGN, so a list of
--     mappings is an entity, not a settings key.
--
--   * `day_pack_ticks` — tick-off-to-collect plan notes. One row per
--     (user, day, stable line key); writing one NEVER touches the movement
--     ledger, and unticking deletes it.
--
-- Additive only: three new tables and one CHECK widening. Nothing existing
-- is rewritten or dropped.

-- 1. manual per-ticket picks ("reserved/attached" intent) ---------------------

CREATE TABLE ticket_stock_picks (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  ticket_id uuid NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
  stock_item_id uuid NOT NULL REFERENCES stock_items(id) ON DELETE CASCADE,
  qty_milli bigint NOT NULL DEFAULT 1000 CHECK (qty_milli > 0),
  note text CHECK (note IS NULL OR char_length(note) <= 500),
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, ticket_id, stock_item_id)
);
CREATE INDEX idx_ticket_stock_picks_ticket
  ON ticket_stock_picks (org_id, ticket_id);
CREATE INDEX idx_ticket_stock_picks_item
  ON ticket_stock_picks (org_id, stock_item_id);

-- 2. org dispatch rules: scope -> recommended item, or scope -> default kit --

CREATE TABLE dispatch_pack_rules (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  scope_kind text NOT NULL
    CHECK (scope_kind IN ('ticket_type', 'ticket_category', 'board')),
  scope_id uuid NOT NULL,
  target_kind text NOT NULL CHECK (target_kind IN ('item', 'kit')),
  stock_item_id uuid REFERENCES stock_items(id) ON DELETE CASCADE,
  kit_id uuid REFERENCES tool_kits(id) ON DELETE CASCADE,
  qty_milli bigint NOT NULL DEFAULT 1000 CHECK (qty_milli > 0),
  note text CHECK (note IS NULL OR char_length(note) <= 500),
  sort_order integer NOT NULL DEFAULT 0,
  active boolean NOT NULL DEFAULT true,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  -- Exactly one target per rule; a shape violation is a DB error, not a
  -- silently ignored row.
  CHECK (
    (target_kind = 'item' AND stock_item_id IS NOT NULL AND kit_id IS NULL)
    OR (target_kind = 'kit' AND kit_id IS NOT NULL AND stock_item_id IS NULL)
  )
);
-- One rule per (scope, item) and per (scope, kit); two partial indexes
-- because a plain UNIQUE treats NULLs as distinct.
CREATE UNIQUE INDEX uq_dispatch_pack_rules_item
  ON dispatch_pack_rules (org_id, scope_kind, scope_id, stock_item_id)
  WHERE target_kind = 'item';
CREATE UNIQUE INDEX uq_dispatch_pack_rules_kit
  ON dispatch_pack_rules (org_id, scope_kind, scope_id, kit_id)
  WHERE target_kind = 'kit';
CREATE INDEX idx_dispatch_pack_rules_scope
  ON dispatch_pack_rules (org_id, scope_kind, scope_id)
  WHERE active;

-- 3. tick-off-to-collect plan notes -------------------------------------------

CREATE TABLE day_pack_ticks (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  day date NOT NULL,
  -- Stable line identity from the read model ("pick:<uuid>", "rule:<uuid>",
  -- "item:<uuid>", "kit:<uuid>", "checklist:<uuid>").
  line_key text NOT NULL CHECK (char_length(line_key) BETWEEN 1 AND 200),
  ticket_id uuid REFERENCES tickets(id) ON DELETE SET NULL,
  note text CHECK (note IS NULL OR char_length(note) <= 500),
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, user_id, day, line_key)
);
CREATE INDEX idx_day_pack_ticks_day ON day_pack_ticks (org_id, user_id, day);

-- 4. label purposes gain `tool_kit` (toolbox tag -> kit on scan) --------------

ALTER TABLE label_templates DROP CONSTRAINT label_templates_purpose_check;
ALTER TABLE label_templates ADD CONSTRAINT label_templates_purpose_check
  CHECK (purpose IN ('stock_item', 'stock_location', 'serialized_unit',
                     'device_asset', 'tool_kit'));

-- 5. RLS on every new readable table ------------------------------------------

ALTER TABLE ticket_stock_picks ENABLE ROW LEVEL SECURITY;
CREATE POLICY ticket_stock_picks_org ON ticket_stock_picks
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE dispatch_pack_rules ENABLE ROW LEVEL SECURITY;
CREATE POLICY dispatch_pack_rules_org ON dispatch_pack_rules
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE day_pack_ticks ENABLE ROW LEVEL SECURITY;
CREATE POLICY day_pack_ticks_org ON day_pack_ticks
  USING (org_id = current_setting('app.org_id', true)::uuid);
