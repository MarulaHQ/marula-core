-- Migration 0193: project calendar zones.
--
-- A plan is not free to land wherever it likes. A client closes for two weeks
-- over Christmas; a cutover is only allowed inside an agreed maintenance
-- window. Those are constraints on the calendar, not on any one task, and the
-- timeline has to draw them so a coordinator can see why a bar cannot move into
-- a stretch of days.
--
--   * kind = 'window'   — the agreed change window. Work may sit here, and a
--                         cutover usually must.
--   * kind = 'closed'   — the client is shut. Work may not sit here.
--
-- A zone is a date range with a label, not a rule engine: the reschedule path
-- keeps its own calendar arithmetic, and a zone is drawn plus checked by the
-- surfaces that place work. Keeping it data means an org can state a constraint
-- without a developer encoding it.
--
-- Compat: additive (policy §4.1) — one new table, no destructive statements.

CREATE TABLE project_calendar_zones (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  kind text NOT NULL CHECK (kind IN ('window', 'closed')),
  label text NOT NULL,
  starts_on date NOT NULL,
  ends_on date NOT NULL,
  note text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CHECK (ends_on >= starts_on)
);
CREATE INDEX idx_project_calendar_zones_project
  ON project_calendar_zones (org_id, project_id, starts_on);

ALTER TABLE project_calendar_zones ENABLE ROW LEVEL SECURITY;
CREATE POLICY project_calendar_zones_org ON project_calendar_zones
  USING (org_id = current_setting('app.org_id', true)::uuid);
