-- Migration 0209: typed blockers and the project workbench's pods.
--
-- Blocked used to be a status word, so a plan could say a task was blocked but
-- not what it waited on or who could unblock it. In an IT project the blocker
-- is usually the fact that moves the date (S3.9, S10.3), so it becomes data:
--
--   * blocker_type says what kind of wait it is. The type decides who is told
--     and what the plan draws; `client`, `task`, `vendor`, `access`, `parts`
--     and `other` are the closed vocabulary the pack uses.
--   * blocker_owner_id is the person expected to clear it, when one is known.
--   * blocked_at is when the wait started, which is what makes "waiting for 6
--     days" a fact rather than an estimate.
--
-- The project workbench's pods are organisation documents, not per-user state.
-- Two tables keep that distinction honest:
--
--   * project_pod_settings — which pods each role sees on the project task
--     workbench, per surface. It is an org document, so an upgrade never
--     overwrites it and a person cannot widen their own access by hiding a pod.
--   * project_pod_prefs — what one person hid and in which order, layered on
--     top of the role's document.
--
-- Compat: additive (policy §4.1) — nullable columns, two new tables.

ALTER TABLE project_tasks
  ADD COLUMN blocker_type text CHECK (blocker_type IS NULL OR blocker_type IN
    ('client','task','vendor','access','parts','other')),
  ADD COLUMN blocker_note text,
  ADD COLUMN blocker_owner_id uuid REFERENCES users(id) ON DELETE SET NULL,
  ADD COLUMN blocked_at timestamptz;

-- A typed blocker and the moment it started belong together: a type with no
-- timestamp cannot say how long the wait has run, and a timestamp with no type
-- repeats the status word this column set exists to replace.
ALTER TABLE project_tasks
  ADD CONSTRAINT project_tasks_blocker_pair
  CHECK ((blocker_type IS NULL) = (blocked_at IS NULL));

CREATE TABLE project_pod_settings (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- The workbench family: `project_task` today; the ticket workbench keeps its
  -- own settings so one surface's upgrade cannot move the other's.
  surface text NOT NULL DEFAULT 'project_task'
    CHECK (surface IN ('project_task')),
  role_key text NOT NULL,
  -- Ordered pod keys. An empty array is a real choice: this role sees none.
  pods text[] NOT NULL DEFAULT '{}',
  updated_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, surface, role_key)
);

CREATE TABLE project_pod_prefs (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  surface text NOT NULL DEFAULT 'project_task'
    CHECK (surface IN ('project_task')),
  hidden text[] NOT NULL DEFAULT '{}',
  pod_order text[] NOT NULL DEFAULT '{}',
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, user_id, surface)
);

ALTER TABLE project_pod_settings ENABLE ROW LEVEL SECURITY;
CREATE POLICY project_pod_settings_org ON project_pod_settings
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE project_pod_prefs ENABLE ROW LEVEL SECURITY;
CREATE POLICY project_pod_prefs_org ON project_pod_prefs
  USING (org_id = current_setting('app.org_id', true)::uuid);
