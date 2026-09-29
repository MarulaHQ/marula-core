-- Time categories (Wave 39 lane 1, `p39-time-categories`; migration 0110 is
-- this lane's exclusive window; head was 0109).
--
-- Purpose: the categorization backbone for the billing workflow (report
-- V3/V4/V5/V6). `time_entries.work_type` has been free text with no
-- taxonomy — entries land as NULL and T&M bills them silently at the
-- contract default rate. This migration gives every org a semantic,
-- admin-managed category catalog:
--
--   * `time_categories` — org-scoped taxonomy with typed billing
--     semantics: `default_billable`, `requires_unbillable_reason`
--     ( flipping billable off needs a typed reason when the category
--     says so), `rate_card_affinity` (a hint that T&M rate-card rows
--     key onto this slug; the card itself stays contract-owned and is
--     NOT migrated here), and a display order.
--
--   * `time_category_work_type_map` — the KEPT mapping table from the
--     one-time backfill: every distinct legacy `work_type` free-text
--     value an org had, and the category slug it was mapped to. kept
--     verbatim so admins can audit the normalization later.
--
--   * `time_entries.unbillable_reason` — the typed reason (V3: warranty,
--     courtesy, in_retainer_block, sla_covered, internal, mistake)
--     required when a category's semantics say so; null on billable
--     entries.
--
-- Seed set: the ten categories the demo org already gestures at
-- (remote / on_site / depot / travel / incident / project / internal /
-- warranty / training / admin), seeded per-org idempotently so
-- self-hosters upgrading an existing install get the same taxonomy.
-- Seeding rides a trigger-free INSERT .. SELECT over orgs; the unique
-- index keeps re-runs honest.
--
-- Additive only: two new tables, one new column on time_entries, one
-- index. Nothing existing is rewritten or dropped.

CREATE TABLE time_categories (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- Lowercase snake_case semantic key. Entries key their `work_type`
  -- column onto this; the T&M rate card keys `contract_rates.work_type`
  -- onto it the same way. Stable forever: renames change `label` only.
  slug text NOT NULL CHECK (slug ~ '^[a-z][a-z0-9_]{0,47}$'),
  label text NOT NULL CHECK (char_length(label) BETWEEN 1 AND 80),
  description text CHECK (char_length(description) <= 500),
  -- Billing semantics (typed defaults, not bare booleans alone):
  default_billable boolean NOT NULL DEFAULT true,
  -- When false, billable flips off WITHOUT a reason; when true, the
  -- category demands a typed unbillable reason first (validation in
  -- core-domain + the API route, not a DB constraint — legacy rows
  -- predate it).
  requires_unbillable_reason boolean NOT NULL DEFAULT false,
  -- Hint that T&M card rows for this slug are expected; documentation
  -- for admins and the settings surface, not a billing behavior.
  rate_card_affinity boolean NOT NULL DEFAULT false,
  -- Admin-managed display order (capture surfaces sort by it).
  position integer NOT NULL DEFAULT 0,
  -- Soft retire: never delete (entries keep their slug reference);
  -- retired categories drop out of pickers and capture defaults.
  retired boolean NOT NULL DEFAULT false,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, slug)
);

CREATE INDEX idx_time_categories_org_order
  ON time_categories (org_id, position, slug) WHERE NOT retired;

-- The one-time backfill mapping, kept forever (lane scope B): every
-- distinct legacy free-text `work_type` per org and the slug it
-- normalized to. Backfilled rows carry `source = 'backfill'`; admins
--adding manual aliases later write `source = 'manual'`.
CREATE TABLE time_category_work_type_map (
  id uuid PRIMARY KEY,
  -- FK carries BOTH columns so it targets the (org_id, slug) unique
  -- constraint on time_categories: an alias can only name a category
  -- in the same org (and the org cascade cascades with it).
  org_id uuid NOT NULL,
  -- The raw legacy value (trimmed, case-preserved as found).
  legacy_value text NOT NULL,
  category_slug text NOT NULL,
  CONSTRAINT fk_time_category_map_org
    FOREIGN KEY (org_id) REFERENCES orgs(id) ON DELETE CASCADE,
  CONSTRAINT fk_time_category_map_category
    FOREIGN KEY (org_id, category_slug) REFERENCES time_categories(org_id, slug)
    ON UPDATE CASCADE,
  source text NOT NULL DEFAULT 'backfill' CHECK (source IN ('backfill', 'manual')),
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, legacy_value)
);

-- V3: the typed unbillable reason (Vocabulary matches the verification
-- gaps: warranty, courtesy, in_retainer_block, sla_covered, internal,
-- mistake). Null on billable entries; checked by app validation.
ALTER TABLE time_entries
  ADD COLUMN unbillable_reason text;

CREATE INDEX idx_time_entries_org_work_type
  ON time_entries (org_id, work_type) WHERE work_type IS NOT NULL;

ALTER TABLE time_categories ENABLE ROW LEVEL SECURITY;
CREATE POLICY time_categories_org ON time_categories
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE time_category_work_type_map ENABLE ROW LEVEL SECURITY;
CREATE POLICY time_category_work_type_map_org ON time_category_work_type_map
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- Seed the ten semantic categories for EVERY org (idempotent: the
-- unique (org_id, slug) index dedupes on re-run). Positions are spaced
-- by ten so admins can reorder without renumbering everything.
INSERT INTO time_categories
  (id, org_id, slug, label, description, default_billable,
   requires_unbillable_reason, rate_card_affinity, position, retired)
SELECT
  gen_random_uuid(),
  o.id,
  v.slug,
  v.label,
  v.description,
  v.default_billable,
  v.requires_unbillable_reason,
  v.rate_card_affinity,
  v.position,
  false
FROM orgs o
CROSS JOIN (VALUES
  ('remote',    'Remote',         'Work done remotely (phone, email, remote tools)',        true,  false, true,  0),
  ('on_site',   'On-site',        'Work at the client location',                            true,  false, true,  10),
  ('depot',     'Depot',          'Bench / workshop repair work',                           true,  false, true,  20),
  ('travel',    'Travel',         'Travel time to or from client sites',                    true,  false, false, 30),
  ('incident',  'Incident',       'Time against incident response or emergencies',          true,  false, true,  40),
  ('project',   'Project',        'Time against project phases rather than tickets',        true,  false, true,  50),
  ('internal',  'Internal',       'Internal admin, meetings, and own-system work',          false, false, false, 60),
  ('warranty',  'Warranty',       'Work covered by warranty — not billable to the client',  false, true,  false, 70),
  ('training',  'Training',       'Training and enablement time',                           false, false, false, 80),
  ('admin',     'Admin',          'Administrative time not attributable to client work',    false, false, false, 90)
) AS v(slug, label, description, default_billable, requires_unbillable_reason, rate_card_affinity, position)
ON CONFLICT (org_id, slug) DO NOTHING;

-- One-time backfill of existing free-text work_type values onto the
-- seed slugs (lane scope B — deterministic, case-insensitive,
-- whitespace-normalized). Unmapped values get NO row here: they stay
-- visible to admins as legacy free text (the report's "unclassified"
-- posture — surfaced, never silently guessed). The demo seed's values
-- (remote, on_site, onsite, depot, travel) all normalize; the wrap-up
-- sheet's old UI defaults ("Remote Support", "On-site", "Project work",
-- "Consulting", "Internal") map too.
-- The mapping is built from BOTH columns that key onto a category:
-- historical `time_entries.work_type` AND `contract_rates.work_type`.
-- Seeding from entries alone would leave a rate card written as `onsite`
-- unmatched after the backfill normalizes entries onto `on_site` — the
-- contract would silently fall back to the default rate, which is the
-- exact failure this lane exists to fix.
INSERT INTO time_category_work_type_map
  (id, org_id, legacy_value, category_slug, source)
SELECT DISTINCT
  gen_random_uuid(),
  src.org_id,
  src.work_type,
  m.slug,
  'backfill'
FROM (
  SELECT org_id, work_type FROM time_entries WHERE work_type IS NOT NULL
  UNION
  SELECT org_id, work_type FROM contract_rates WHERE work_type IS NOT NULL
) src
JOIN (
  VALUES
    ('remote',          'remote'),
    ('remote support',  'remote'),
    ('onsite',          'on_site'),
    ('on_site',         'on_site'),
    ('on-site',         'on_site'),
    ('depot',           'depot'),
    ('travel',          'travel'),
    ('project work',    'project'),
    ('consulting',      'project'),
    ('internal',        'internal'),
    ('training',        'training'),
    ('warranty',        'warranty'),
    ('admin',           'admin')
) AS m(legacy, slug) ON lower(btrim(m.legacy)) = lower(btrim(src.work_type))
ON CONFLICT (org_id, legacy_value) DO NOTHING;

-- Point existing entries at their normalized category slug. Legacy
-- values that had no mapping row keep their original free text (honest
-- unclassified posture — never guessed).
UPDATE time_entries te
   SET work_type = m.category_slug
  FROM time_category_work_type_map m
 WHERE m.org_id = te.org_id
   AND lower(btrim(m.legacy_value)) = lower(btrim(te.work_type))
   AND m.source = 'backfill'
   AND te.work_type IS DISTINCT FROM m.category_slug;

-- The rate card keys the SAME slugs. Normalizing it here is what keeps an
-- upgraded contract pricing correctly instead of dropping every carded
-- work type onto the contract default rate.
UPDATE contract_rates cr
   SET work_type = m.category_slug
  FROM time_category_work_type_map m
 WHERE m.org_id = cr.org_id
   AND lower(btrim(m.legacy_value)) = lower(btrim(cr.work_type))
   AND m.source = 'backfill'
   AND cr.work_type IS DISTINCT FROM m.category_slug;
