-- Stage 2 / item 2.5 — typed settings registry storage (docs/10-agent-platform.md §2).
--
-- Purpose: one table for every registry-declared setting, keyed by
-- (scope, key) with the owning org (tenancy + RLS) and — for user-scope
-- rows — the owning user (`owner_id`: org_id for org-scope rows so the
-- unique key stays total). Values are JSONB validated against the
-- registry BEFORE write; the CHECK here is a coarse backstop on JSON type.
--
-- Non-purpose: no per-key DDL, no defaults stored (defaults live in the
-- declarations and are resolved at read time — defaults-first doctrine),
-- no history table (audit_events records every change).

CREATE TABLE settings (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  scope text NOT NULL CHECK (scope IN ('org', 'user')),
  key text NOT NULL,
  -- org-scope rows: equals org_id; user-scope rows: the owning user id.
  owner_id uuid NOT NULL,
  value jsonb NOT NULL CHECK (jsonb_typeof(value) IN ('string', 'number', 'boolean')),
  updated_by uuid REFERENCES users(id) ON DELETE SET NULL,
  updated_at timestamptz NOT NULL DEFAULT now(),
  -- Shape invariant: org rows are owned by the org itself; user rows must
  -- carry a DIFFERENT owner than the org id (a real user uuid, checked in
  -- the query layer where FK targets are scope-dependent).
  CONSTRAINT settings_owner_shape CHECK (
    (scope = 'org' AND owner_id = org_id)
    OR (scope = 'user' AND owner_id <> org_id)
  )
);

-- Total unique key: COALESCE folds the user NULL-space onto a sentinel so
-- org rows can never duplicate. Upserts target this expression index.
CREATE UNIQUE INDEX settings_key_unique
  ON settings (org_id, scope, key, COALESCE(owner_id, '00000000-0000-0000-0000-000000000000'));

ALTER TABLE settings ENABLE ROW LEVEL SECURITY;
-- Both GUCs go through NULLIF: `missing_ok` yields NULL on never-set
-- sessions but '' on defined-then-reset POOLED sessions, and a bare
-- ''::uuid cast would raise 22P02 instead of default-deny.
CREATE POLICY settings_org ON settings
  USING (
    org_id = NULLIF(current_setting('app.org_id', true), '')::uuid
    AND (
      scope = 'org'
      OR owner_id = NULLIF(current_setting('app.user_id', true), '')::uuid
    )
  );
