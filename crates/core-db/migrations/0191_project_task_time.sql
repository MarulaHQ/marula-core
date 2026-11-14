-- Migration 0191: task time, work type and the per-project working calendar.
--
-- Purpose: a project task carried a status and a title, so a plan could say a
-- project was late but never when the work sat. This migration adds the facts a
-- schedule needs and the calendar those facts are measured against.
--
--   * project_tasks gains a start and a duration, an effort estimate and the
--     effort still remaining, a work type, a priority, a role key and the
--     skills the work needs. The pair (start_at, duration_minutes) is
--     both-or-neither: a start with no length is not a bar on a timeline, and a
--     length with no start has nowhere to sit. Neither column is required,
--     because a task an organisation has not dated stays undated — the honest
--     state, and the one the plan draws as an undated row rather than guessing.
--
--   * baseline_start_at and baseline_duration_minutes hold what the plan said
--     when it was agreed. They are written by the baseline capture, never by an
--     ordinary edit, so a slipped date can be shown against the promise.
--
--   * project_task_deps gains a kind. Only finish-to-start computes dates
--     today; naming the kind keeps the graph honest when a second kind is
--     added and lets the UI label a blocker rather than imply one rule for all.
--
--   * project_working_calendars lets a project state its own working week
--     (a six-day client site, a project that avoids Mondays). Where no row
--     exists the organisation's calendar applies. This is ADR 0034 decision 3:
--     the project's pattern wins for its own dates, the organisation's calendar
--     is the base, and a person's weekly hours scale capacity without moving a
--     date.
--
-- Compat: purely additive (policy §4.1) — nullable columns, one new table, no
-- destructive statements, so no `--breaking` flag.

ALTER TABLE project_tasks
  ADD COLUMN start_at timestamptz,
  ADD COLUMN duration_minutes integer CHECK (duration_minutes >= 0),
  ADD COLUMN effort_estimate_minutes integer CHECK (effort_estimate_minutes >= 0),
  ADD COLUMN effort_remaining_minutes integer CHECK (effort_remaining_minutes >= 0),
  ADD COLUMN work_type text CHECK (work_type IN
    ('onsite','bench','remote','vendor','client','coordination')),
  ADD COLUMN priority text CHECK (priority IN ('p1','p2','p3','p4')),
  ADD COLUMN role_key text,
  ADD COLUMN required_skills text[] NOT NULL DEFAULT '{}',
  ADD COLUMN is_critical boolean NOT NULL DEFAULT false,
  ADD COLUMN baseline_start_at timestamptz,
  ADD COLUMN baseline_duration_minutes integer CHECK (baseline_duration_minutes >= 0),
  ADD COLUMN external_owner text,
  ADD COLUMN calendar_pin uuid;

-- A task with a start must say how long it runs, and a task with a length must
-- say when it starts.
ALTER TABLE project_tasks
  ADD CONSTRAINT project_tasks_time_pair
  CHECK ((start_at IS NULL) = (duration_minutes IS NULL));

ALTER TABLE project_task_deps
  ADD COLUMN kind text NOT NULL DEFAULT 'finish_to_start'
  CHECK (kind IN ('finish_to_start'));

CREATE INDEX idx_project_tasks_plan ON project_tasks (org_id, project_id, start_at)
  WHERE start_at IS NOT NULL;
CREATE INDEX idx_project_tasks_assignee_plan ON project_tasks (org_id, assignee_id, start_at)
  WHERE start_at IS NOT NULL;
CREATE INDEX idx_project_tasks_skills ON project_tasks USING gin (required_skills);

CREATE TABLE project_working_calendars (
  project_id uuid PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- Monday at bit 0, Sunday at bit 6. Monday to Friday is 31.
  weekdays smallint NOT NULL DEFAULT 31 CHECK (weekdays BETWEEN 1 AND 127),
  -- Minutes of work in one working day, lunch already excluded.
  minutes_per_day integer NOT NULL DEFAULT 480
    CHECK (minutes_per_day > 0 AND minutes_per_day <= 1440),
  note text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE project_working_calendars ENABLE ROW LEVEL SECURITY;
CREATE POLICY project_working_calendars_org ON project_working_calendars
  USING (org_id = current_setting('app.org_id', true)::uuid);
