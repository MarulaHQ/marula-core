-- Wave 22 lane 2 `p22-ecosystem-dedupe-workbench` — ADR 0003 extension:
-- the source-link + identity-key + merge-candidate pattern extended to
-- clients/contacts (config-items stay as-is), plus the deferred candidate
-- status the guided merge workbench needs.
--
-- 1. `ci_merge_candidates.status` gains 'deferred' — the workbench's
--    batch "defer" action parks a candidate out of the pending queue
--    without deciding it (reversible; never a silent drop).
--
-- 2. `contacts` grows the same CI5 field tiers as device_assets:
--    observed_fields (feed-written, churns with its source),
--    curated_fields (human-set, sticky forever), field_sources
--    (per-field provenance {source_system, source_link_id, observed_at}).
--    `fields` stays the effective view (curated over observed over base).
--
-- 3. `entity_source_links` — the CI2 link key generalized: (org,
--    entity_kind, source_system, external_id) binds one vendor record to
--    one canonical client/contact. A source's second sync updates its own
--    link; it never creates a twin.
--
-- 4. `entity_merge_candidates` — the review queue for clients/contacts:
--    same visible-evidence shape as `ci_merge_candidates` (confidence,
--    reason, evidence signals) plus a deferred status. Two flavors ride
--    the same table:
--      * contact dedupe: both entity ids set (email/phone identity keys).
--      * client org_map reconciliation: `candidate_entity_id` is NULL and
--        the candidate side is the VENDOR ORG (source_system,
--        vendor_org_id, vendor_label) — the operator maps it to an
--        existing Meridian client or keeps it unmapped.
--
-- 5. `entity_merges` — audited, undoable contact merges (the ci_merges
--    pattern: the `moved` packet carries what folded for a faithful
--    split). Client merges are NOT modeled: a client is an org-level
--    container, its records are reconciled/mapped, never folded.
--
-- 6. `client_org_map` — the DURABLE operator mapping the connectors'
--    org_map (adapters-core) reconcile candidates resolve against: accept
--    on a reconciliation candidate writes one row here (audited),
--    replacing the "unmapped org is only a sync error" status quo.
--
-- 7. RLS: org-bound on every new table (USING + WITH CHECK).

-- ---------------------------------------------------------------------------
-- 1. CI merge candidates learn to defer.
-- ---------------------------------------------------------------------------

ALTER TABLE ci_merge_candidates DROP CONSTRAINT ci_merge_candidates_status_check;
ALTER TABLE ci_merge_candidates ADD CONSTRAINT ci_merge_candidates_status_check
  CHECK (status IN ('pending', 'auto_merged', 'accepted', 'rejected', 'deferred'));

-- ---------------------------------------------------------------------------
-- 2. Contacts get the observed/curated tiers (CI5 parity).
-- ---------------------------------------------------------------------------

ALTER TABLE contacts
  ADD COLUMN observed_fields jsonb NOT NULL DEFAULT '{}',
  ADD COLUMN curated_fields jsonb NOT NULL DEFAULT '{}',
  ADD COLUMN field_sources jsonb NOT NULL DEFAULT '{}',
  -- merge tombstone (the entity_merges split substrate), mirroring
  -- device_assets.merged_into_id.
  ADD COLUMN merged_into_id uuid,
  -- The contacts table never carried a mutation stamp (0002 kept only
  -- created_at); the feed write path + workbench need one.
  ADD COLUMN updated_at timestamptz NOT NULL DEFAULT now();

-- ---------------------------------------------------------------------------
-- 3. Entity source links (clients/contacts).
-- ---------------------------------------------------------------------------

CREATE TABLE entity_source_links (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- 'contact' | 'client' (client links bind the org_map reconciliation).
  entity_kind text NOT NULL CHECK (entity_kind IN ('contact', 'client')),
  entity_id uuid NOT NULL,
  source_system text NOT NULL,
  external_id text NOT NULL,
  source_guid text,
  payload_ref text,
  last_seen_at timestamptz NOT NULL DEFAULT now(),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, entity_kind, source_system, external_id)
);
CREATE INDEX idx_entity_source_links_entity
  ON entity_source_links (org_id, entity_kind, entity_id);
CREATE INDEX idx_entity_source_links_last_seen
  ON entity_source_links (org_id, entity_kind, last_seen_at);

-- ---------------------------------------------------------------------------
-- 4. Entity merge candidates (the same visible-evidence queue).
-- ---------------------------------------------------------------------------

CREATE TABLE entity_merge_candidates (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  entity_kind text NOT NULL CHECK (entity_kind IN ('contact', 'client')),
  -- contact dedupe: both ids are contacts. client reconciliation: only
  -- the proposed target client id; the candidate side is the vendor org.
  primary_entity_id uuid NOT NULL,
  candidate_entity_id uuid,
  -- Set for BOTH flavors: contact dedupe is fed by connectors too.
  source_system text,
  vendor_org_id text,
  vendor_label text,
  -- 0.000 .. 1.000
  confidence numeric(4,3) NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
  -- human-readable reason ("email match a@b.co", "phone match +1…",
  -- "vendor org label matches client name").
  reason text NOT NULL,
  evidence jsonb NOT NULL DEFAULT '{}',
  status text NOT NULL DEFAULT 'pending'
    CHECK (status IN ('pending', 'auto_merged', 'accepted', 'rejected', 'deferred')),
  decided_by uuid REFERENCES users(id) ON DELETE SET NULL,
  decided_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  -- A candidate never proposes folding a row into itself.
  CHECK (candidate_entity_id IS NULL OR candidate_entity_id <> primary_entity_id)
);
CREATE INDEX idx_entity_merge_candidates_queue
  ON entity_merge_candidates (org_id, entity_kind, status);
-- One candidate per (primary, candidate) pair — re-upserts refresh
-- evidence in place instead of stacking duplicates. NULLS NOT DISTINCT
-- so two reconciliation rows for the same vendor org collapse onto one
-- candidate id; decided rows are covered too (a decided pair never
-- re-queues silently — create_entity_candidate's upsert keeps the
-- decided status).
CREATE UNIQUE INDEX uq_entity_merge_candidates_pair
  ON entity_merge_candidates (
    org_id, entity_kind, primary_entity_id, candidate_entity_id,
    source_system, vendor_org_id
  ) NULLS NOT DISTINCT;

-- ---------------------------------------------------------------------------
-- 5. Entity merges (contact fold; audited + undoable).
-- ---------------------------------------------------------------------------

CREATE TABLE entity_merges (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  entity_kind text NOT NULL CHECK (entity_kind = 'contact'),
  primary_entity_id uuid NOT NULL,
  merged_entity_id uuid NOT NULL,
  candidate_id uuid REFERENCES entity_merge_candidates(id) ON DELETE SET NULL,
  confidence numeric(4,3),
  reason text NOT NULL,
  evidence jsonb NOT NULL DEFAULT '{}',
  -- what moved, for a faithful split: {ticket_refs_relinked,
  -- fields_before, observed_before, curated_before, source_link_ids}.
  moved jsonb NOT NULL DEFAULT '{}',
  status text NOT NULL DEFAULT 'applied' CHECK (status IN ('applied', 'undone')),
  merged_by uuid REFERENCES users(id) ON DELETE SET NULL,
  merged_at timestamptz NOT NULL DEFAULT now(),
  undone_by uuid REFERENCES users(id) ON DELETE SET NULL,
  undone_at timestamptz,
  CHECK (primary_entity_id <> merged_entity_id)
);
CREATE INDEX idx_entity_merges_primary ON entity_merges (org_id, primary_entity_id);

-- ---------------------------------------------------------------------------
-- 6. The durable operator org→client map.
-- ---------------------------------------------------------------------------

CREATE TABLE client_org_map (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  source_system text NOT NULL,
  -- The vendor organization id as the vendor API carries it (stringified).
  vendor_org_id text NOT NULL,
  client_id uuid NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
  vendor_label text,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, source_system, vendor_org_id)
);
CREATE INDEX idx_client_org_map_client ON client_org_map (org_id, client_id);

-- ---------------------------------------------------------------------------
-- 7. RLS: org-bound on every new table.
-- ---------------------------------------------------------------------------

ALTER TABLE entity_source_links ENABLE ROW LEVEL SECURITY;
CREATE POLICY entity_source_links_org ON entity_source_links
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE entity_merge_candidates ENABLE ROW LEVEL SECURITY;
CREATE POLICY entity_merge_candidates_org ON entity_merge_candidates
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE entity_merges ENABLE ROW LEVEL SECURITY;
CREATE POLICY entity_merges_org ON entity_merges
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE client_org_map ENABLE ROW LEVEL SECURITY;
CREATE POLICY client_org_map_org ON client_org_map
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

-- ---------------------------------------------------------------------------
-- 8. The Jev intelligent-dedupe workflow (one row per org): manual or
--    scheduled passes over the pending queues that ask the
--    `decision_record_dedupe` surface per candidate; report_only keeps
--    every decision suggestion-grade, auto_merge additionally folds the
--    candidates whose Noul/answer sits at/above the workflow threshold
--    (audited + undoable exactly like a human accept). Enablement is
--    GATED at the API layer: the org's decision model must be configured
--    (ADR 0010/0011 posture) before a workflow can turn on.
-- ---------------------------------------------------------------------------

CREATE TABLE dedupe_workflows (
  org_id uuid PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
  enabled boolean NOT NULL DEFAULT false,
  -- the workflow's own confidence dial (0.000..1.000); auto_merge folds
  -- only suggestions at/above it, everything below stays queued for
  -- human review in the workbench.
  confidence_threshold numeric(4,3) NOT NULL DEFAULT 0.800
    CHECK (confidence_threshold >= 0 AND confidence_threshold <= 1),
  mode text NOT NULL DEFAULT 'report_only'
    CHECK (mode IN ('report_only', 'auto_merge')),
  -- scheduled cadence; NULL = manual runs only (the workbench trigger).
  interval_hours integer
    CHECK (interval_hours IS NULL OR (interval_hours BETWEEN 1 AND 720)),
  notify_user_ids uuid[] NOT NULL DEFAULT '{}',
  -- which queues the pass covers. Clients ride reconciliation only and
  -- are therefore always report-only by definition.
  entity_kinds text[] NOT NULL DEFAULT '{config_items,contacts}',
  next_run_at timestamptz,
  last_run_at timestamptz,
  last_result jsonb NOT NULL DEFAULT '{}',
  updated_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE dedupe_workflows ENABLE ROW LEVEL SECURITY;
CREATE POLICY dedupe_workflows_org ON dedupe_workflows
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

-- The dedupe run report lands as a notification to designated users.
ALTER TABLE notifications DROP CONSTRAINT notifications_kind_check;
ALTER TABLE notifications ADD CONSTRAINT notifications_kind_check
  CHECK (kind IN ('mention','assignment','sla_risk','sla_breach','escalation',
                  'missing_time','ticket_update','report_delivery','ai_cap_warn',
                  'secret_rotation','dedupe_report'));

ALTER TABLE notification_preferences DROP CONSTRAINT notification_preferences_kind_check;
ALTER TABLE notification_preferences ADD CONSTRAINT notification_preferences_kind_check
  CHECK (kind IN ('mention','assignment','sla_risk','sla_breach','escalation',
                  'missing_time','ticket_update','report_delivery','ai_cap_warn',
                  'secret_rotation','dedupe_report'));
