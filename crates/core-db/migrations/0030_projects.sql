-- Projects line (TASKLIST Phase 3: "Projects: phases/tasks/views/deps/
-- templates; milestone billing → invoicing"). Lane PROJECTS, Wave 8b.
--
-- Purpose: the project → phase → task substrate for PSA delivery work,
-- plus saved task-list views, task dependencies, and reusable project
-- templates. Six org-scoped tables, all RLS-bound to the caller's org
-- exactly like every other tenant table (0001 convention):
--
--   * projects — the engagement. `billing_method` records how delivered
--     work turns into money; `budget_minor` + `currency` (ISO 4217 text)
--     carry an optional project budget. Money is integer minor units —
--     floats are forbidden near amounts (§5).
--
--   * project_phases — delivery stages. A phase flagged `is_milestone` is
--     a billing milestone: completing it emits `project.milestone_completed`
--     through the transactional outbox (the ONLY seam to core-billing; the
--     invoicing consumer lands Wave 9).
--
--   * project_tasks — units of work. Milestone tasks carry the same
--     amount/currency pair. `completed` is reached only through the
--     completion path, which enforces dependency readiness.
--
--   * project_task_deps — finish-to-start dependencies between tasks in
--     the SAME project. PK(task_id, depends_on_task_id); self-deps are
--     rejected by CHECK, cycles by the application layer (core-projects
--     `deps` module) before insert.
--
--   * project_views — saved task-list views (filter/group/sort spec as
--     versioned additive jsonb, customizations-as-data per policy §5).
--     `project_id NULL` = org-wide view; Postgres UNIQUE treats NULLs as
--     distinct, so name uniqueness for org-wide views is enforced by the
--     application layer.
--
--   * project_templates — reusable blueprints (name + versioned jsonb
--     spec); instantiation is an ordinary transactional insert plan.
--     Templates are data, not live references: deleting one never touches
--     projects already created from it.
--
-- Non-purpose: no invoicing rows, no PaymentProvider, no invoice state
-- machine (core-billing, Wave 9 — the seam is the `project.milestone_
-- completed` event envelope only); no labor actuals rollups yet (those
-- join through the read-model views per the reporting track); no Gantt/
-- scheduling engine.
--
-- Compat: purely additive (policy §4.1) — new tables, nullable columns,
-- no destructive statements, so no `--breaking` flag.

CREATE TABLE projects (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  client_id uuid REFERENCES clients(id) ON DELETE SET NULL,
  name text NOT NULL,
  description text,
  status text NOT NULL DEFAULT 'planning'
    CHECK (status IN ('planning','active','on_hold','completed','cancelled')),
  billing_method text NOT NULL DEFAULT 'non_billable'
    CHECK (billing_method IN ('milestone','time_and_materials','fixed_fee','non_billable')),
  budget_minor bigint,
  -- ISO 4217 code; required whenever budget_minor is set (app-enforced).
  currency text,
  start_date date,
  due_date date,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, name)
);
CREATE INDEX idx_projects_org ON projects (org_id, status);

CREATE TABLE project_phases (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  name text NOT NULL,
  description text,
  sort_order integer NOT NULL DEFAULT 0,
  status text NOT NULL DEFAULT 'pending'
    CHECK (status IN ('pending','active','completed','cancelled')),
  is_milestone boolean NOT NULL DEFAULT false,
  milestone_amount_minor bigint,
  milestone_currency text,
  completed_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (project_id, name)
);
CREATE INDEX idx_project_phases_project ON project_phases (project_id, sort_order);

CREATE TABLE project_tasks (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  phase_id uuid REFERENCES project_phases(id) ON DELETE SET NULL,
  title text NOT NULL,
  description text,
  status text NOT NULL DEFAULT 'pending'
    CHECK (status IN ('pending','in_progress','blocked','completed','cancelled')),
  assignee_id uuid REFERENCES users(id) ON DELETE SET NULL,
  sort_order integer NOT NULL DEFAULT 0,
  is_milestone boolean NOT NULL DEFAULT false,
  milestone_amount_minor bigint,
  milestone_currency text,
  due_at timestamptz,
  completed_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_project_tasks_project ON project_tasks (project_id, sort_order);
CREATE INDEX idx_project_tasks_assignee ON project_tasks (org_id, assignee_id)
  WHERE status NOT IN ('completed','cancelled');

CREATE TABLE project_task_deps (
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  task_id uuid NOT NULL REFERENCES project_tasks(id) ON DELETE CASCADE,
  depends_on_task_id uuid NOT NULL REFERENCES project_tasks(id) ON DELETE CASCADE,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (task_id, depends_on_task_id),
  CHECK (task_id <> depends_on_task_id)
);
-- Reverse lookup: what blocks when a dependency moves.
CREATE INDEX idx_project_task_deps_reverse ON project_task_deps (depends_on_task_id);

CREATE TABLE project_views (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- NULL = org-wide view spanning every project.
  project_id uuid REFERENCES projects(id) ON DELETE CASCADE,
  name text NOT NULL,
  spec jsonb NOT NULL DEFAULT '{}'::jsonb,
  is_default boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_project_views_org ON project_views (org_id, project_id);

CREATE TABLE project_templates (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  description text,
  spec jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, name)
);

ALTER TABLE projects ENABLE ROW LEVEL SECURITY;
CREATE POLICY projects_org ON projects USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE project_phases ENABLE ROW LEVEL SECURITY;
CREATE POLICY project_phases_org ON project_phases USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE project_tasks ENABLE ROW LEVEL SECURITY;
CREATE POLICY project_tasks_org ON project_tasks USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE project_task_deps ENABLE ROW LEVEL SECURITY;
CREATE POLICY project_task_deps_org ON project_task_deps USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE project_views ENABLE ROW LEVEL SECURITY;
CREATE POLICY project_views_org ON project_views USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE project_templates ENABLE ROW LEVEL SECURITY;
CREATE POLICY project_templates_org ON project_templates USING (org_id = current_setting('app.org_id', true)::uuid);
