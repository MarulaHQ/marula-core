-- Report definitions as data (the private build tracker R3; ADR 0001 §Decision).
--
-- Purpose: durable storage for STORED report specs — one row per saved
-- report definition. Each row carries the registry-backed identification
-- columns (`kind` = view slug, `viz` = visualization hint, `scope` =
-- visibility) plus the spec BODY as jsonb (`{filters, group_by, measures}`).
-- Registry backing at the storage layer: the CHECK constraints pin the
-- column values to the typed registry's slugs (core-reports `catalog` /
-- `spec`), so a hand-inserted row outside the enum spaces cannot even
-- land; full registry validation (column names, operand typing, aggregate
-- compatibility, shape caps) is re-applied in Rust on EVERY write and load
-- — the DB CHECKs are the floor, the Rust registry is the gate.
--
-- Customizations-as-data doctrine (upgrade-compatibility-policy §5): a
-- definition never embeds SQL and never references a physical table —
-- `kind` names an `ops.v1_*` view slug only, so saved reports are
-- migration-safe by construction and visible to the U5 manifest / U6
-- preflight like every other customization.
--
-- Scope semantics: `org` rows are published org-wide (admins only);
-- `self` rows are private to their creator. Visibility filtering happens
-- in the query layer; RLS keeps tenancy absolute.
--
-- Provenance: `created_by` is a hard FK to the authoring staff user —
-- this surface is JWT-only (no api-key path), so every definition has a
-- real author, and the U5 manifest section carries it verbatim.
--
-- Non-purpose: no report EXECUTION state (execution compiles on demand
-- from the spec; results are never persisted), no scheduling (R4 owns a
-- delivery table), no widget/layout placement (R2 dashboards persist via
-- the settings registry), no per-row tenant escaping (org_id + RLS only).

CREATE TABLE report_definitions (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  description text,
  -- Registry-backed view slug (core-reports catalog::ViewId).
  kind text NOT NULL CHECK (kind IN
    ('tickets','time_entries','sla_outcomes','gap_days','first_response')),
  -- Spec body document: {"filters":[…], "group_by":[…], "measures":[…]}.
  -- Column typing/compatibility is enforced by the Rust registry; here we
  -- only pin the document shape floor.
  spec jsonb NOT NULL CHECK (jsonb_typeof(spec) = 'object'),
  -- Registry-backed visualization hint (core-reports spec::VizHint).
  viz text NOT NULL CHECK (viz IN ('bar','line','pie','table')),
  -- Visibility: published org-wide vs private to the creator.
  scope text NOT NULL DEFAULT 'self' CHECK (scope IN ('org','self')),
  created_by uuid NOT NULL REFERENCES users(id),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, name)
);

CREATE INDEX idx_report_definitions_org ON report_definitions (org_id, scope, name);

ALTER TABLE report_definitions ENABLE ROW LEVEL SECURITY;
CREATE POLICY report_definitions_org ON report_definitions
  USING (org_id = current_setting('app.org_id', true)::uuid);
