-- Documentation templates (surface-pack lane, migration 0219).
--
-- Purpose: Hudu ships KB templates and IT Glue ships a library; both are the
-- same idea — a named starting body plus a field list, so a team writes a
-- runbook once and starts the next one from it. `knowledge_templates` is that
-- store for articles, documents and procedures, scoped to the org (a template
-- is shared across clients on purpose) or to one client when a client asked
-- for its own.
--
-- `versions` is an append-only snapshot list, because a template that changes
-- under a document already written from it is exactly the drift a
-- documentation tool is supposed to prevent.
--
-- Non-purpose: no template marketplace and no AI generation. A template is a
-- starting point a person edits.
--
-- Indexes lead with (org_id, kind); RLS on both tables.

CREATE TABLE knowledge_templates (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  client_id uuid REFERENCES clients(id) ON DELETE CASCADE,
  kind text NOT NULL CHECK (kind IN ('article', 'document', 'procedure')),
  name text NOT NULL,
  description text,
  body text NOT NULL DEFAULT '',
  fields jsonb NOT NULL DEFAULT '[]',
  version integer NOT NULL DEFAULT 1 CHECK (version >= 1),
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  updated_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, kind, name)
);

CREATE INDEX idx_knowledge_templates_scope
  ON knowledge_templates (org_id, kind, name);

CREATE TABLE knowledge_template_versions (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  template_id uuid NOT NULL REFERENCES knowledge_templates(id) ON DELETE CASCADE,
  version integer NOT NULL CHECK (version >= 1),
  name text NOT NULL,
  body text NOT NULL,
  fields jsonb NOT NULL DEFAULT '[]',
  author_id uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (template_id, version)
);

CREATE INDEX idx_knowledge_template_versions_tpl
  ON knowledge_template_versions (org_id, template_id, version DESC);

ALTER TABLE knowledge_templates ENABLE ROW LEVEL SECURITY;
CREATE POLICY knowledge_templates_org ON knowledge_templates
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE knowledge_template_versions ENABLE ROW LEVEL SECURITY;
CREATE POLICY knowledge_template_versions_org ON knowledge_template_versions
  USING (org_id = current_setting('app.org_id', true)::uuid);
