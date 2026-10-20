-- Wave 8b import/export lane (the private build tracker D2 remainder + D1).
--
-- Purpose:
-- 1. Saved field-mapping templates per source system (D2 remainder):
--    `import_mapping_templates` stores one reusable source→canonical column
--    mapping per (org, source system, entity kind, name) so operators stop
--    re-mapping the same exporter's CSV by hand on every run. The mapping
--    document is validated against the import engine's canonical field
--    catalogs in code before it is ever saved.
-- 2. Additional import entity families (D2 remainder): the batch and
--    external-id CHECKs widen ADDITIVELY to 'board','ticket','time_entry'
--    (widening only — 0027's audit_events actor_kind precedent; every
--    previously valid row stays valid). Parent-before-child ordering
--    extends to: boards → clients → sites → contacts → tickets →
--    time entries.
-- 3. Async giant batches (D2 remainder): `import_batches` gains
--    `staged_chunks` (rows staged to the ObjectStore under deterministic
--    chunk keys chunk-00000…chunk-{n-1}) and `queued_at` (worker-run
--    marker; enqueue is once-per-batch, finished batches never re-queue).
--
-- Non-purpose: no org-export bookkeeping table. The D1 export bundle is
-- deliberately stateless: content lands in the ObjectStore under
-- org/{org_id}/exports/{export_id}/ and downloads ride short-lived signed
-- URLs, so there is no schema to keep in sync.

ALTER TABLE import_batches DROP CONSTRAINT import_batches_entity_kind_check;
ALTER TABLE import_batches ADD CONSTRAINT import_batches_entity_kind_check
  CHECK (entity_kind IN ('client','site','contact','board','ticket','time_entry'));

ALTER TABLE external_ids DROP CONSTRAINT external_ids_entity_kind_check;
ALTER TABLE external_ids ADD CONSTRAINT external_ids_entity_kind_check
  CHECK (entity_kind IN ('client','site','contact','board','ticket','time_entry'));

ALTER TABLE import_batches
  ADD COLUMN staged_chunks integer NOT NULL DEFAULT 0,
  ADD COLUMN queued_at timestamptz;

CREATE TABLE import_mapping_templates (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  -- Free-form source-system label the operator keys on ("hansworld",
  -- "connectwise", …); grouping + listing dimension, not an enum.
  source_system text NOT NULL,
  entity_kind text NOT NULL CHECK (entity_kind IN ('client','site','contact','board','ticket','time_entry')),
  -- JSON object: source column → canonical Meridian column. Canonical
  -- targets are validated against the engine's field catalog at save time.
  mapping jsonb NOT NULL,
  created_by uuid NOT NULL,
  created_by_kind text NOT NULL CHECK (created_by_kind IN ('user','api_key')),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, source_system, entity_kind, name)
);
CREATE INDEX idx_import_mapping_templates_org
  ON import_mapping_templates (org_id, source_system);

ALTER TABLE import_mapping_templates ENABLE ROW LEVEL SECURITY;
CREATE POLICY import_mapping_templates_org ON import_mapping_templates
  USING (org_id = current_setting('app.org_id', true)::uuid);
