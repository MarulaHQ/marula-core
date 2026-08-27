-- Import framework core (TASKLIST D2).
--
-- Purpose: durable bookkeeping for the import engine — one row per import
-- run (`import_batches`) and the stable external-id → internal-uuid mapping
-- (`external_ids`) that makes repeated pushes idempotent. The mapping table
-- is the idempotency substrate: a source-system key resolves to exactly one
-- internal uuid per org per entity kind, so re-pushing a batch can never
-- duplicate rows; conflict outcomes (skip|update|fail) are resolved in code
-- against it.
--
-- Provenance doctrine: `import_batches.created_by` records WHO drove the
-- run. It carries NO foreign key by design — machine drivers authenticate
-- with x-api-key whose id is not a users-row, so an FK would silently exclude
-- the documented production flow; `created_by_kind` ('user' | 'api_key')
-- disambiguates and audit rows attribute the run to actor_kind 'import'.
--
-- Non-purpose: not a CSV-content store (row payloads live only for the life
-- of a request), not saved field-mapping templates per source system
-- (deferred D2 follow-up), no workflow-state column — batches are
-- start/finish span markers, and dry-runs are distinguished purely by flag.
--
-- Widen the append-only audit provenance vocabulary additively: everything
-- 0001 + 0021 allowed stays valid; 'import' joins it.

ALTER TABLE audit_events DROP CONSTRAINT audit_events_actor_kind_check;
ALTER TABLE audit_events ADD CONSTRAINT audit_events_actor_kind_check
  CHECK (actor_kind IN ('human','agent','plugin','connector','system',
                        'portal_contact','portal_approver','portal_it_manager',
                        'import'));

CREATE TABLE import_batches (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  label text NOT NULL DEFAULT 'unnamed',
  -- Free-form operator note about the originating source system.
  source_note text,
  -- Which entity family this run targets; the import engine's per-batch
  -- single-kind contract keeps parent-before-child ordering explicit.
  entity_kind text NOT NULL CHECK (entity_kind IN ('client','site','contact')),
  conflict_policy text NOT NULL CHECK (conflict_policy IN ('skip','update','fail')),
  dry_run boolean NOT NULL DEFAULT false,
  rows_total integer NOT NULL DEFAULT 0,
  rows_created integer NOT NULL DEFAULT 0,
  rows_updated integer NOT NULL DEFAULT 0,
  rows_skipped integer NOT NULL DEFAULT 0,
  rows_failed integer NOT NULL DEFAULT 0,
  created_by uuid NOT NULL,
  created_by_kind text NOT NULL CHECK (created_by_kind IN ('user','api_key')),
  started_at timestamptz NOT NULL DEFAULT now(),
  finished_at timestamptz
);
CREATE INDEX idx_import_batches_org ON import_batches (org_id, started_at DESC);

-- external_key → internal uuid per (org, entity kind). UNIQUE is the whole
-- idempotency contract: same key re-pushed resolves to the SAME internal id.
CREATE TABLE external_ids (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  entity_kind text NOT NULL CHECK (entity_kind IN ('client','site','contact')),
  external_key text NOT NULL,
  internal_id uuid NOT NULL,
  last_seen_batch uuid REFERENCES import_batches(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, entity_kind, external_key)
);
-- Reverse lookups: does this internal row already carry a stable key?
CREATE INDEX idx_external_ids_internal ON external_ids (org_id, entity_kind, internal_id);

ALTER TABLE import_batches ENABLE ROW LEVEL SECURITY;
CREATE POLICY import_batches_org ON import_batches USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE external_ids ENABLE ROW LEVEL SECURITY;
CREATE POLICY external_ids_org ON external_ids USING (org_id = current_setting('app.org_id', true)::uuid);
