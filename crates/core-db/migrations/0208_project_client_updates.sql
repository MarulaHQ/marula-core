-- Migration 0208: client status updates.
--
-- A status update is a product surface with a cadence, an owner and a client
-- view (S2.11, S2.12, S16.4). The draft is assembled from records that already
-- exist — completed tasks, milestones, change orders — and then a person reads
-- it, edits it, approves it and sends it. Nothing here sends itself.
--
--   * status is draft → approved → sent, or withheld. A withheld update stores
--     the reason, because "nothing moved" is a fact worth keeping: it is what
--     stops the next draft from inventing movement.
--
--   * body is the client-facing prose. It is stored as the person approved it,
--     so the portal shows exactly what was emailed.
--
--   * facts is the evidence: the task ids, milestone ids and change-order ids
--     the draft was built from. A sentence without a fact behind it is what
--     this column exists to prevent.
--
--   * period_start/period_end name the window the update covers, so two
--     updates cannot silently describe the same week.
--
-- Compat: additive (policy §4.1) — one new table, no destructive statements.

CREATE TABLE project_client_updates (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  status text NOT NULL DEFAULT 'draft'
    CHECK (status IN ('draft','approved','sent','withheld')),
  -- The client-facing text, as approved.
  body text NOT NULL DEFAULT '',
  -- The records the draft was built from.
  facts jsonb NOT NULL DEFAULT '[]'::jsonb,
  period_start date,
  period_end date,
  withheld_reason text,
  approved_by uuid REFERENCES users(id) ON DELETE SET NULL,
  approved_at timestamptz,
  sent_at timestamptz,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CHECK (period_start IS NULL OR period_end IS NULL OR period_end >= period_start)
);
CREATE INDEX idx_project_client_updates_project
  ON project_client_updates (org_id, project_id, created_at DESC);
CREATE INDEX idx_project_client_updates_sent
  ON project_client_updates (org_id, project_id, sent_at DESC)
  WHERE status = 'sent';

ALTER TABLE project_client_updates ENABLE ROW LEVEL SECURITY;
CREATE POLICY project_client_updates_org ON project_client_updates
  USING (org_id = current_setting('app.org_id', true)::uuid);
