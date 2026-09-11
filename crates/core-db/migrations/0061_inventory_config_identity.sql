-- Migration 0061: inventory lifecycle + configuration identity (Wave 12 lane 3).
--
-- Purpose (ADR 0003 configuration identity, CI1–CI6; plus asset lifecycle):
--   * Instances stay `device_assets` (0002) — NO parallel CI type/instance
--     registry. The canonical `ConfigItem` IS the asset row; it gains
--     deterministic identity keys, curated-vs-observed field tiers, and a
--     merge tombstone.
--   * Types stay `document_layouts` (0053); CI6 binds each layout to a
--     semantic class (workstation|server|vm|network_appliance|printer|iot).
--     Behavior reads the class, so renaming a layout never changes behavior.
--   * `config_source_links` is the ONLY feed write path (ADR 0003): an
--     adapter writes through (source system, external id) bound to one
--     canonical item; UNIQUE(org, source_system, external_id) makes a
--     source's second sync update its own link and never create a twin.
--   * `ci_interfaces` holds NICs/MACs/IPs/VLANs as attributes of a CI (the
--     ADR lifecycle rule: independently-lifecycle'd things are their own CI
--     linked by composition, not a NIC row).
--   * `ci_merge_candidates` + `ci_merges` implement the visible-evidence
--     merge queue: ambiguous matches are reviewable, auto-merge only above
--     the org threshold in `ci_settings`, every merge is audited and
--     undoable (split), and splits are possible.
--   * `ci_field_precedence` stores per-type/per-source precedence OVERRIDES;
--     the ADR-shipped defaults resolve in Rust when no row exists.
--   * `device_assets` also gains rental tracking + depreciation inputs for
--     the inventory/asset lifecycle (money is integer minor units + a
--     currency code, per ADR 0002; no implicit conversion here).
--
-- BINDING POLICY: additive-only. Pre-existing columns are kept verbatim for
-- connector compatibility. Every table is org-bound and RLS'd.
-- Non-purpose: no vendor transports (lane 8 owns NinjaOne/IT Glue/Auvik);
-- no booking engine here — depreciation is stored inputs, book value is a
-- pure domain calculation.

-- 1. asset lifecycle: rental tracking + depreciation inputs (additive) ------

ALTER TABLE device_assets
  ADD COLUMN ownership text NOT NULL DEFAULT 'owned'
    CHECK (ownership IN ('owned', 'leased', 'rented')),
  ADD COLUMN rental_vendor text,
  ADD COLUMN rental_start date,
  ADD COLUMN rental_end date,
  ADD COLUMN rental_cost_minor bigint
    CHECK (rental_cost_minor IS NULL OR rental_cost_minor >= 0),
  ADD COLUMN rental_currency text,
  ADD COLUMN rental_cycle text
    CHECK (rental_cycle IS NULL OR rental_cycle IN ('monthly', 'quarterly', 'annual')),
  ADD COLUMN purchase_date date,
  ADD COLUMN purchase_cost_minor bigint
    CHECK (purchase_cost_minor IS NULL OR purchase_cost_minor >= 0),
  ADD COLUMN purchase_currency text,
  ADD COLUMN depreciation_method text NOT NULL DEFAULT 'none'
    CHECK (depreciation_method IN ('none', 'straight_line', 'declining_balance')),
  ADD COLUMN useful_life_months integer
    CHECK (useful_life_months IS NULL OR useful_life_months > 0),
  ADD COLUMN salvage_value_minor bigint
    CHECK (salvage_value_minor IS NULL OR salvage_value_minor >= 0),
  ADD COLUMN depreciation_rate_bps integer
    CHECK (depreciation_rate_bps IS NULL
           OR (depreciation_rate_bps > 0 AND depreciation_rate_bps < 10000));

-- 2. configuration identity on the asset row (CI1/CI3/CI5) ------------------

ALTER TABLE device_assets
  -- CI3: deterministic identity keys ({source_guid, serial, service_tag,
  -- macs:[], hostname, domain, ...}); app-validated, feed-writable.
  ADD COLUMN identity_keys jsonb NOT NULL DEFAULT '{}',
  -- CI5: observed (feed-written, churns with its source) and curated
  -- (human-set, sticky forever) tiers; `fields` stays the effective view.
  ADD COLUMN observed_fields jsonb NOT NULL DEFAULT '{}',
  ADD COLUMN curated_fields jsonb NOT NULL DEFAULT '{}',
  -- per observed field: {source_system, source_link_id, observed_at}.
  ADD COLUMN field_sources jsonb NOT NULL DEFAULT '{}',
  -- CI4: merge tombstone — a non-NULL value means this row was absorbed by
  -- the target (the row survives for undo/audit; the target is canonical).
  ADD COLUMN merged_into_id uuid REFERENCES device_assets(id) ON DELETE SET NULL;

CREATE INDEX idx_device_assets_merged_into ON device_assets (org_id, merged_into_id)
  WHERE merged_into_id IS NOT NULL;
CREATE INDEX idx_device_assets_identity_keys ON device_assets USING gin (identity_keys);

-- 3. semantic class binding on the type registry (CI6) ----------------------

ALTER TABLE document_layouts
  ADD COLUMN semantic_class text
    CHECK (semantic_class IS NULL OR semantic_class IN
      ('workstation', 'server', 'vm', 'network_appliance', 'printer', 'iot'));

UPDATE document_layouts SET semantic_class = CASE slug
    WHEN 'server' THEN 'server'
    WHEN 'workstation' THEN 'workstation'
    WHEN 'firewall' THEN 'network_appliance'
    WHEN 'network-device' THEN 'network_appliance'
    ELSE semantic_class
  END
  WHERE is_system
    AND slug IN ('server', 'workstation', 'firewall', 'network-device');

-- 4. CI2: source links — the only feed write path ---------------------------

CREATE TABLE config_source_links (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  config_item_id uuid NOT NULL REFERENCES device_assets(id) ON DELETE CASCADE,
  -- e.g. 'ninjaone' | 'it_glue' | 'auvix' (lane 8 supplies the transports).
  source_system text NOT NULL,
  external_id text NOT NULL,
  source_guid text,
  -- opaque reference to the raw payload in the feed's own store.
  payload_ref text,
  last_seen_at timestamptz NOT NULL DEFAULT now(),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  -- CI2: one link per source record; a second sync updates its own link.
  UNIQUE (org_id, source_system, external_id)
);
CREATE INDEX idx_config_source_links_item ON config_source_links (org_id, config_item_id);
CREATE INDEX idx_config_source_links_guid ON config_source_links (org_id, source_guid)
  WHERE source_guid IS NOT NULL;

-- 5. CI interfaces (NICs/IPs/VLANs as CI attributes) ------------------------

CREATE TABLE ci_interfaces (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  config_item_id uuid NOT NULL REFERENCES device_assets(id) ON DELETE CASCADE,
  name text,
  -- normalized lower colon form (aa:bb:cc:dd:ee:ff); identity signal for CI3.
  mac text,
  ips jsonb NOT NULL DEFAULT '[]',
  vlans jsonb NOT NULL DEFAULT '[]',
  is_primary boolean NOT NULL DEFAULT false,
  source_link_id uuid REFERENCES config_source_links(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_ci_interfaces_item ON ci_interfaces (org_id, config_item_id);
CREATE INDEX idx_ci_interfaces_mac ON ci_interfaces (org_id, mac) WHERE mac IS NOT NULL;

-- 6. CI5: per-type/per-source field precedence overrides --------------------

CREATE TABLE ci_field_precedence (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- NULL layout_id = applies to every CI type (the shipped default shape).
  layout_id uuid REFERENCES document_layouts(id) ON DELETE CASCADE,
  semantic_class text
    CHECK (semantic_class IS NULL OR semantic_class IN
      ('workstation', 'server', 'vm', 'network_appliance', 'printer', 'iot')),
  field_key text NOT NULL,
  source_system text NOT NULL,
  -- higher priority wins among feed sources; curated always outranks feeds.
  priority smallint NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX uq_ci_field_precedence
  ON ci_field_precedence (
    org_id,
    COALESCE(layout_id, '00000000-0000-0000-0000-000000000000'::uuid),
    field_key,
    source_system
  );

-- 7. CI4: merge queue (visible evidence) + audited, undoable merges ---------

CREATE TABLE ci_merge_candidates (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- the canonical (surviving) item an incoming/other item may fold into.
  primary_config_item_id uuid NOT NULL REFERENCES device_assets(id) ON DELETE CASCADE,
  candidate_config_item_id uuid NOT NULL REFERENCES device_assets(id) ON DELETE CASCADE,
  -- 0.000 .. 1.000
  confidence numeric(4,3) NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
  -- human-readable reason ("serial match SN-1234", "MAC set overlap 2/3").
  reason text NOT NULL,
  evidence jsonb NOT NULL DEFAULT '{}',
  status text NOT NULL DEFAULT 'pending'
    CHECK (status IN ('pending', 'auto_merged', 'accepted', 'rejected')),
  decided_by uuid REFERENCES users(id) ON DELETE SET NULL,
  decided_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CHECK (primary_config_item_id <> candidate_config_item_id),
  UNIQUE (org_id, primary_config_item_id, candidate_config_item_id)
);
CREATE INDEX idx_ci_merge_candidates_status ON ci_merge_candidates (org_id, status);

CREATE TABLE ci_merges (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  primary_config_item_id uuid NOT NULL REFERENCES device_assets(id) ON DELETE CASCADE,
  merged_config_item_id uuid NOT NULL REFERENCES device_assets(id) ON DELETE CASCADE,
  candidate_id uuid REFERENCES ci_merge_candidates(id) ON DELETE SET NULL,
  confidence numeric(4,3),
  reason text NOT NULL,
  evidence jsonb NOT NULL DEFAULT '{}',
  -- what moved, for a faithful split: {source_link_ids, interface_ids,
  -- fields_before, observed_before, curated_before}.
  moved jsonb NOT NULL DEFAULT '{}',
  status text NOT NULL DEFAULT 'applied' CHECK (status IN ('applied', 'undone')),
  merged_by uuid REFERENCES users(id) ON DELETE SET NULL,
  merged_at timestamptz NOT NULL DEFAULT now(),
  undone_by uuid REFERENCES users(id) ON DELETE SET NULL,
  undone_at timestamptz,
  CHECK (primary_config_item_id <> merged_config_item_id)
);
CREATE INDEX idx_ci_merges_primary ON ci_merges (org_id, primary_config_item_id);
CREATE INDEX idx_ci_merges_merged ON ci_merges (org_id, merged_config_item_id);

-- 8. CI4: org-configurable auto-merge confidence threshold ------------------

CREATE TABLE ci_settings (
  org_id uuid PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
  -- auto-merge only at/above this confidence; below it, a human reviews.
  auto_merge_threshold numeric(4,3) NOT NULL DEFAULT 0.850
    CHECK (auto_merge_threshold >= 0 AND auto_merge_threshold <= 1),
  updated_at timestamptz NOT NULL DEFAULT now()
);

-- 9. RLS: org-bound on every table (USING + WITH CHECK) ---------------------

ALTER TABLE config_source_links ENABLE ROW LEVEL SECURITY;
CREATE POLICY config_source_links_org ON config_source_links
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE ci_interfaces ENABLE ROW LEVEL SECURITY;
CREATE POLICY ci_interfaces_org ON ci_interfaces
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE ci_field_precedence ENABLE ROW LEVEL SECURITY;
CREATE POLICY ci_field_precedence_org ON ci_field_precedence
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE ci_merge_candidates ENABLE ROW LEVEL SECURITY;
CREATE POLICY ci_merge_candidates_org ON ci_merge_candidates
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE ci_merges ENABLE ROW LEVEL SECURITY;
CREATE POLICY ci_merges_org ON ci_merges
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE ci_settings ENABLE ROW LEVEL SECURITY;
CREATE POLICY ci_settings_org ON ci_settings
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);
