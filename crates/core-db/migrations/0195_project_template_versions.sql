-- Migration 0195: versioned project templates and their lessons.
--
-- A template used to be one mutable document. That is enough while a team
-- writes its own templates and wrong the moment a shipped template is
-- improved: an organisation that tuned a template should keep the plan it
-- tuned, and an organisation that never touched one should get the better
-- version. Both are only possible if a version is a row.
--
--   * project_template_versions — one row per published or draft version of a
--     template, carrying the spec as it stood. A shipped version is immutable:
--     its `source` is `shipped` and nothing in the product updates it. A fork
--     is a new template row whose first version records where it came from.
--
--   * project_templates gains three facts: which version is current, whether
--     the template shipped with the product, and the version it was forked
--     from. Existing templates keep working: their current version is created
--     from the document they already hold, and they are org-owned.
--
--   * project_template_lessons — what a closed project taught about the
--     template it came from. A lesson names the project that produced it, so
--     a suggestion is evidence rather than opinion, and it is stored whether
--     or not anyone acts on it.
--
-- Compat: additive (policy §4.1). The backfill creates one version row per
-- existing template from its current spec, which is the only honest reading of
-- a document that was never versioned.

CREATE TABLE project_template_versions (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  template_id uuid NOT NULL REFERENCES project_templates(id) ON DELETE CASCADE,
  version integer NOT NULL CHECK (version > 0),
  spec jsonb NOT NULL,
  status text NOT NULL DEFAULT 'draft'
    CHECK (status IN ('draft','published','retired')),
  -- Where this version came from: shipped with the product, written by the
  -- organisation, or forked from another template's version.
  source text NOT NULL DEFAULT 'org'
    CHECK (source IN ('shipped','org','fork')),
  note text,
  published_by uuid REFERENCES users(id) ON DELETE SET NULL,
  published_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (template_id, version)
);
CREATE INDEX idx_project_template_versions_template
  ON project_template_versions (org_id, template_id, version DESC);

ALTER TABLE project_templates
  ADD COLUMN current_version_id uuid REFERENCES project_template_versions(id) ON DELETE SET NULL,
  ADD COLUMN is_shipped boolean NOT NULL DEFAULT false,
  ADD COLUMN shipped_key text,
  ADD COLUMN forked_from_version_id uuid REFERENCES project_template_versions(id) ON DELETE SET NULL;

-- One shipped template per key, so an upgrade can find the row it owns.
CREATE UNIQUE INDEX idx_project_templates_shipped_key
  ON project_templates (org_id, shipped_key) WHERE shipped_key IS NOT NULL;

CREATE TABLE project_template_lessons (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  template_id uuid NOT NULL REFERENCES project_templates(id) ON DELETE CASCADE,
  -- The project that taught the lesson. A suggestion without a project is an
  -- opinion, and this column is what keeps it evidence.
  project_id uuid REFERENCES projects(id) ON DELETE SET NULL,
  kind text NOT NULL CHECK (kind IN
    ('duration','dependency','missing_step','extra_step','role','material','other')),
  message text NOT NULL,
  -- The numbers behind the lesson: planned against actual days, the task that
  -- was missing, the role that was wrong. Consumers render it, never guess it.
  evidence jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_project_template_lessons_template
  ON project_template_lessons (org_id, template_id, created_at DESC);

ALTER TABLE project_template_versions ENABLE ROW LEVEL SECURITY;
CREATE POLICY project_template_versions_org ON project_template_versions
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE project_template_lessons ENABLE ROW LEVEL SECURITY;
CREATE POLICY project_template_lessons_org ON project_template_lessons
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- Backfill: every existing template becomes version 1, published, org-owned,
-- and points at that version.
INSERT INTO project_template_versions
  (id, org_id, template_id, version, spec, status, source, note, published_at, created_at)
SELECT gen_random_uuid(), t.org_id, t.id, 1, t.spec, 'published', 'org',
       'Created from the template document at first versioning',
       t.created_at, t.created_at
FROM project_templates t;

UPDATE project_templates t
   SET current_version_id = v.id
  FROM project_template_versions v
 WHERE v.template_id = t.id AND v.version = 1;
