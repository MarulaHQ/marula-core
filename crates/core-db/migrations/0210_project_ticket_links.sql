-- Migration 0210: the project-to-ticket link.
--
-- Promotion (S10.7) gives a project task a ticket on the service desk's board
-- and SLA. It is one act with two records, and each side must be able to name
-- the other, so the link is a row rather than a column on either side: a
-- ticket can exist without a task, a task can exist without a ticket, and the
-- link is what says they are the same work.
--
--   * UNIQUE (task_id) — a task has at most one promoted ticket. Promoting
--     twice must return the first ticket, never create a second one.
--   * UNIQUE (ticket_id) — a ticket is the promotion of at most one task. A
--     second task cannot adopt a ticket that already belongs to one.
--
-- Compat: additive (policy §4.1) — one new table, no destructive statements.

CREATE TABLE project_ticket_links (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  task_id uuid NOT NULL REFERENCES project_tasks(id) ON DELETE CASCADE,
  ticket_id uuid NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (task_id),
  UNIQUE (ticket_id)
);
CREATE INDEX idx_project_ticket_links_project
  ON project_ticket_links (org_id, project_id);

ALTER TABLE project_ticket_links ENABLE ROW LEVEL SECURITY;
CREATE POLICY project_ticket_links_org ON project_ticket_links
  USING (org_id = current_setting('app.org_id', true)::uuid);
