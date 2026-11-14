-- Migration 0192: baseline captures.
--
-- A baseline is what the plan said when it was agreed. The task columns that
-- hold it arrived with 0191 (`baseline_start_at`, `baseline_duration_minutes`);
-- this migration records each capture as an event of its own, so a project can
-- say when the promise was made and by whom, not only what it said.
--
-- A capture is a snapshot, not a running total: the task columns hold the most
-- recent baseline, and this table holds every baseline ever taken. A project
-- that has been re-baselined twice keeps both earlier promises, which is what
-- makes "we agreed this three times" a fact rather than a memory.
--
-- Compat: purely additive (policy §4.1) — one new table, no destructive
-- statements, so no `--breaking` flag.

CREATE TABLE project_baselines (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  captured_by uuid REFERENCES users(id) ON DELETE SET NULL,
  captured_at timestamptz NOT NULL DEFAULT now(),
  note text,
  -- The per-task starts and lengths that were in force at capture time. The
  -- task columns stay the fast read; this document is the record.
  snapshot jsonb NOT NULL DEFAULT '[]'::jsonb,
  -- Tasks that carried a date at capture time and tasks that did not. A
  -- baseline with almost nothing dated is a weak promise and the count says so.
  dated_tasks integer NOT NULL DEFAULT 0,
  total_tasks integer NOT NULL DEFAULT 0
);
CREATE INDEX idx_project_baselines_project
  ON project_baselines (org_id, project_id, captured_at DESC);

ALTER TABLE project_baselines ENABLE ROW LEVEL SECURITY;
CREATE POLICY project_baselines_org ON project_baselines
  USING (org_id = current_setting('app.org_id', true)::uuid);
