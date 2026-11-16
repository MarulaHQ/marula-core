-- Procedures: ordered steps, a runbook body and a checklist posture
-- (surface-pack lane, migration 0216).
--
-- Purpose: Hudu's processes/SOPs. A procedure is a title, a short summary, a
-- long body (the runbook), an owner, an optional checklist, and ordered
-- steps. `client_visible` lets the client portal show the runbooks a client
-- asked for, and `version` increments on every edit so the portal surface can
-- say which revision the client is reading.
--
-- Steps are a separate table rather than JSON because a step is a record the
-- audit trail should be able to name, and because the checklist a technician
-- ticks is the same row the author wrote.
--
-- Non-purpose: no execution engine and no generated assignment. A procedure
-- documents what a person does; the workflow engine runs automations.

CREATE TABLE knowledge_procedures (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  client_id uuid NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
  title text NOT NULL,
  summary text,
  body text NOT NULL DEFAULT '',
  owner_user_id uuid REFERENCES users(id) ON DELETE SET NULL,
  is_checklist boolean NOT NULL DEFAULT true,
  client_visible boolean NOT NULL DEFAULT false,
  version integer NOT NULL DEFAULT 1 CHECK (version >= 1),
  fields jsonb NOT NULL DEFAULT '{}',
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  updated_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, client_id, title)
);

CREATE INDEX idx_knowledge_procedures_scope
  ON knowledge_procedures (org_id, client_id, updated_at DESC);
CREATE INDEX idx_knowledge_procedures_portal
  ON knowledge_procedures (org_id, client_id)
  WHERE client_visible;

CREATE TABLE knowledge_procedure_steps (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  procedure_id uuid NOT NULL REFERENCES knowledge_procedures(id) ON DELETE CASCADE,
  position integer NOT NULL CHECK (position >= 0),
  title text NOT NULL,
  body text,
  required boolean NOT NULL DEFAULT true,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (procedure_id, position)
);

CREATE INDEX idx_knowledge_procedure_steps_proc
  ON knowledge_procedure_steps (org_id, procedure_id, position);

ALTER TABLE knowledge_procedures ENABLE ROW LEVEL SECURITY;
CREATE POLICY knowledge_procedures_org ON knowledge_procedures
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE knowledge_procedure_steps ENABLE ROW LEVEL SECURITY;
CREATE POLICY knowledge_procedure_steps_org ON knowledge_procedure_steps
  USING (org_id = current_setting('app.org_id', true)::uuid);
