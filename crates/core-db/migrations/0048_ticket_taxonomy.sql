-- Ticket type/category/subcategory taxonomy (Wave 11 lane 3; migration 0048).
--
-- Purpose: org-owned classification trees that the ticket board moves and
-- the workspace pickers consume:
--   * `ticket_types` — flat list (Incident / Service Request / Problem /
--     Change seeded), plus `ticket_categories` — a self-referencing tree
--     exactly TWO levels deep (root = category, child = subcategory).
--   * FULL org CRUD lives in core-api `routes/taxonomy.rs`. Rows are never
--     deleted through the API: retirement is a soft flag, so existing
--     tickets keep their reference forever (the FK is ON DELETE RESTRICT
--     to make even a future code path fail loudly rather than orphan).
--   * CLASSIFICATION-ONLY doctrine (§5): nothing in behavior keys off these
--     trees — SLA timers, pause logic, automations, billing, and reports
--     bind to the SEMANTIC status categories (`board_statuses.category`),
--     never to a type/category/subcategory. Renames, reparents, and
--     retirements are therefore always behavior-neutral.
--
-- The standard accepted set is seeded for every existing org below; org
-- creation seeds the same set via `core_db::orgs::seed_default_taxonomy`.
--
-- RLS: org-scoped like every tenant table (0003/0004/0047 pattern).

CREATE TABLE ticket_types (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  description text,
  -- Soft retirement: pickers hide retired rows, existing tickets keep
  -- their FK reference, and the manifest still exports retired nodes so
  -- the upgrade preflight (U6) sees the full classification surface.
  is_retired boolean NOT NULL DEFAULT false,
  sort_order integer NOT NULL DEFAULT 0,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, name)
);
CREATE INDEX idx_ticket_types_org ON ticket_types (org_id, sort_order)
  WHERE is_retired = false;

CREATE TABLE ticket_categories (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- NULL = root category; non-NULL = subcategory of that category.
  parent_id uuid REFERENCES ticket_categories(id) ON DELETE RESTRICT,
  name text NOT NULL,
  description text,
  is_retired boolean NOT NULL DEFAULT false,
  sort_order integer NOT NULL DEFAULT 0,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
-- Name uniqueness scoped to the sibling set (roots vs per-parent children).
CREATE UNIQUE INDEX uq_ticket_categories_root_name ON ticket_categories (org_id, name)
  WHERE parent_id IS NULL;
CREATE UNIQUE INDEX uq_ticket_categories_child_name ON ticket_categories (org_id, parent_id, name)
  WHERE parent_id IS NOT NULL;
CREATE INDEX idx_ticket_categories_org ON ticket_categories (org_id, sort_order)
  WHERE is_retired = false;
CREATE INDEX idx_ticket_categories_parent ON ticket_categories (parent_id);

-- Depth guard: the tree is exactly TWO levels (category → subcategory).
-- The API validates too; this trigger is the DB-level guarantor so no
-- other writer can grow a third level.
CREATE FUNCTION ticket_categories_depth_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  parent_parent uuid;
BEGIN
  IF NEW.parent_id IS NOT NULL THEN
    IF NEW.parent_id = NEW.id THEN
      RAISE EXCEPTION 'ticket_categories: a row cannot be its own parent';
    END IF;
    SELECT parent_id INTO parent_parent FROM ticket_categories WHERE id = NEW.parent_id;
    IF parent_parent IS NOT NULL THEN
      RAISE EXCEPTION 'ticket_categories: subcategories cannot have children (tree is two levels deep)';
    END IF;
  END IF;
  RETURN NEW;
END;
$$;
CREATE TRIGGER trg_ticket_categories_depth
  BEFORE INSERT OR UPDATE ON ticket_categories
  FOR EACH ROW EXECUTE FUNCTION ticket_categories_depth_guard();

ALTER TABLE ticket_types ENABLE ROW LEVEL SECURITY;
CREATE POLICY ticket_types_org ON ticket_types
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE ticket_categories ENABLE ROW LEVEL SECURITY;
CREATE POLICY ticket_categories_org ON ticket_categories
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

-- Ticket classification references. Both nullable: tickets created before
-- this migration (and tickets whose taxonomy entry is not required) stay
-- valid; classification is never a lifecycle gate.
ALTER TABLE tickets
  ADD COLUMN type_id uuid REFERENCES ticket_types(id) ON DELETE RESTRICT,
  ADD COLUMN category_id uuid REFERENCES ticket_categories(id) ON DELETE RESTRICT;
CREATE INDEX idx_tickets_type ON tickets (org_id, type_id) WHERE type_id IS NOT NULL;
CREATE INDEX idx_tickets_category ON tickets (org_id, category_id) WHERE category_id IS NOT NULL;

-- Backfill: seed the standard accepted set for EVERY existing org. New
-- orgs get the identical set from core_db::orgs::seed_default_taxonomy.
WITH t(name, ord) AS (
  VALUES ('Incident', 0), ('Service Request', 1), ('Problem', 2), ('Change', 3)
)
INSERT INTO ticket_types (id, org_id, name, sort_order)
SELECT gen_random_uuid(), o.id, t.name, t.ord FROM orgs o CROSS JOIN t;

WITH c(name, ord) AS (
  VALUES
    ('End-User Computing', 0),
    ('Network', 10),
    ('Servers & Infrastructure', 20),
    ('Cloud Services', 30),
    ('Security', 40),
    ('Telephony & Communications', 50),
    ('Line-of-Business Applications', 60),
    ('Facilities & Assets', 70)
)
INSERT INTO ticket_categories (id, org_id, parent_id, name, sort_order)
SELECT gen_random_uuid(), o.id, NULL, c.name, c.ord FROM orgs o CROSS JOIN c;

-- Subcategories join their per-org root by name (roots exist for every
-- org after the insert above, so the join multiplies per org correctly).
INSERT INTO ticket_categories (id, org_id, parent_id, name, sort_order)
SELECT gen_random_uuid(), root.org_id, root.id, s.name, s.ord
FROM (VALUES
  ('End-User Computing', 'Hardware', 0),
  ('End-User Computing', 'Software & Applications', 1),
  ('End-User Computing', 'Email & Collaboration', 2),
  ('End-User Computing', 'Printing & Scanning', 3),
  ('Network', 'Connectivity & Outage', 0),
  ('Network', 'VPN & Remote Access', 1),
  ('Network', 'DNS & DHCP', 2),
  ('Network', 'Firewall & Perimeter', 3),
  ('Servers & Infrastructure', 'Windows Server', 0),
  ('Servers & Infrastructure', 'Linux & Unix', 1),
  ('Servers & Infrastructure', 'Virtualization', 2),
  ('Servers & Infrastructure', 'Storage & Backup', 3),
  ('Cloud Services', 'Microsoft 365', 0),
  ('Cloud Services', 'Identity & Access', 1),
  ('Cloud Services', 'Azure & AWS', 2),
  ('Cloud Services', 'Backup & Recovery', 3),
  ('Security', 'Malware & Phishing', 0),
  ('Security', 'Vulnerability & Patching', 1),
  ('Security', 'Data Loss & Compliance', 2),
  ('Telephony & Communications', 'VoIP & PBX', 0),
  ('Telephony & Communications', 'Mobile Devices', 1),
  ('Telephony & Communications', 'Video Conferencing', 2),
  ('Line-of-Business Applications', 'Application Error', 0),
  ('Line-of-Business Applications', 'Access Request', 1),
  ('Line-of-Business Applications', 'Data & Reporting', 2),
  ('Line-of-Business Applications', 'Integration & API', 3),
  ('Facilities & Assets', 'Physical Access', 0),
  ('Facilities & Assets', 'Peripherals', 1),
  ('Facilities & Assets', 'New Equipment', 2)
) AS s(cat_name, name, ord)
JOIN ticket_categories root
  ON root.parent_id IS NULL AND root.name = s.cat_name;
