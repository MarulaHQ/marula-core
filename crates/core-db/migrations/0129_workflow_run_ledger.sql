-- The workflow run ledger (TASKLIST Wave 43 lane 14, `p43-workflow-run-ledger`;
-- head on main was 0128, the node registry).
--
-- TWO tables, and the split between them is the lane's load-bearing
-- decision:
--
-- (A) `workflow_runs` — ONE ROW PER EXECUTION ATTEMPT of a workflow
--     document. A workflow's life is a CHAIN of these rows: the run an
--     envelope started, the row a resume continues it as, and the row an
--     operator retry re-runs it as. Two self-references carry that chain
--     and they mean different things:
--       resumed_from — the PARKED run this row continues. Set by the
--                     resume job, never by a rewrite of the parked row.
--       retry_of     — the FAILED run this row re-executes. Set by the
--                     operator retry action.
--
--     A2's rule is enforced HERE, in the database, not only in Rust: the
--     `workflow_runs_ledger_guard` trigger makes every provenance and
--     identity column immutable on UPDATE. A run may move through its
--     lifecycle (running → waiting → succeeded/failed/abandoned) and it may
--     never be edited into a different story. That is the difference
--     between an audit record and fiction, and the difference is a trigger.
--
--     `waiting` is a first-class status, not a comment on `running`: a run
--     is either EXECUTING or PARKED, so the row carries where it resumes
--     from (`resume_cursor`), when (`wake_at`) and what it is waiting for
--     (`wake_on`). A CHECK spells the whole shape so a half-shaped row can
--     never land regardless of column order or hand-written inserts.
--
--     The row is self-describing forever: `rule_definition` is the exact
--     document that ran, `catalogue_version` is the node release it ran
--     against, and both survive the rule being renamed or removed
--     (`rule_id` is SET NULL on delete — the ledger keeps the run, it never
--     erases history to match a workspace tidy-up).
--
-- (B) `workflow_run_steps` — one row per step ATTEMPT: per-step status,
--     timing, input, output and error. A step retried INSIDE a run is a
--     NEW row carrying a higher `attempt`, so what already ran stays
--     visible instead of being overwritten by the next try. The same
--     immutable-identity trigger guards the step's identity, its input and
--     its start time; status, output, error and finish time are the
--     lifecycle a step is allowed to move through.
--
-- (C) Retention is a SETTING, not an unbounded table. Runs are the
--     highest-volume rows this engine will produce, so the window lives in
--     the typed settings registry (`workflow.run_retention_days`, declared
--     in core-domain, default 30 days) and the purge reads it per org.
--     Nothing here stores a default: defaults-first doctrine says the
--     Rust declaration owns it.
--
-- Non-purpose: no step-scheduling machinery (the queue at 0015 owns
-- retries and dead-letters; the `workflow.resume` job kind is the executor
-- lane's to settle), no workflow DOCUMENT store (that is
-- `automation_rules`), and no events emitted from this migration.

-- ============================================================================
-- (A) one row per execution attempt
-- ============================================================================

CREATE TABLE workflow_runs (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,

  -- The document that ran. NULL after the rule is deleted: the run keeps
  -- its name, its snapshot and its history, which is what makes the ledger
  -- an audit record rather than a join against a table that moved on.
  rule_id uuid REFERENCES automation_rules(id) ON DELETE SET NULL,
  rule_name text NOT NULL,
  -- The document revision the WRITER read. `automation_rules` carries no
  -- revision counter yet, so a source without one reports 1 — and
  -- `rule_definition` below is the byte-exact identity that makes the
  -- number mean something either way.
  rule_version integer NOT NULL DEFAULT 1 CHECK (rule_version >= 1),
  -- The exact trigger + conditions + actions document this run executed,
  -- snapshotted at start. Self-describing forever: the run never needs a
  -- second table to explain what it ran.
  rule_definition jsonb NOT NULL CHECK (jsonb_typeof(rule_definition) = 'object'),
  -- The node-catalogue release this run executed against (0128's pin
  -- records releases globally; the executor validates the document against
  -- the release it names).
  catalogue_version text NOT NULL,

  -- executing | parked | terminal. See the shape CHECK below.
  status text NOT NULL DEFAULT 'running'
    CHECK (status IN ('running','waiting','succeeded','failed','abandoned')),

  -- Why this row exists:
  --   event          — an envelope matched the rule and started it
  --   resume         — a parked run woke and CONTINUED as this row
  --   operator_retry — an operator re-ran a failed run as this row
  --   manual         — an operator ran the document directly
  trigger_kind text NOT NULL
    CHECK (trigger_kind IN ('event','resume','operator_retry','manual')),
  -- The envelope that triggered the run (NULL for a manual run).
  trigger_event_id uuid REFERENCES outbox_events(id) ON DELETE SET NULL,
  -- The `jobs` row that produced this run (0015's queue; recorded against,
  -- never forked). NULL only for rows written before the queue leg.
  trigger_job_id uuid REFERENCES jobs(id) ON DELETE SET NULL,
  -- The member who started a manual or retry run; NULL for event-driven.
  trigger_user_id uuid REFERENCES users(id) ON DELETE SET NULL,

  -- The chain. Exactly one of these is ever set on a row, and each is
  -- pinned to the trigger kind that legitimately sets it (below).
  resumed_from uuid REFERENCES workflow_runs(id) ON DELETE SET NULL,
  retry_of uuid REFERENCES workflow_runs(id) ON DELETE SET NULL,

  -- Where a parked run continues from, when it wakes, and what it is
  -- waiting for. The cursor is the EXECUTOR's opaque continuation state;
  -- a retry writes its own documented shape (`start_at_step` / mode) into
  -- the same column. `wake_on` names the node or the event type the run is
  -- parked on ('queue' while a retry waits for its job to be claimed).
  resume_cursor jsonb,
  wake_at timestamptz,
  wake_on text,

  -- The run-level failure, when it failed.
  error_code text,
  error text CHECK (error IS NULL OR length(error) <= 4000),

  started_at timestamptz NOT NULL DEFAULT now(),
  finished_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

-- The run shape as data, one constraint spelling the whole truth:
--   * a PARKED run (waiting) always says where it resumes from and what it
--     is waiting for — `flow.delay` carries a wake-at, `flow.waitFor`
--     carries the event type, a queued retry carries both;
--   * a RUNNING row never carries a wake-at or a finish time (the cursor
--     may ride along: a running executor keeps its live state there);
--   * a TERMINAL row always carries its finish time and never a wake-at.
ALTER TABLE workflow_runs
  ADD CONSTRAINT workflow_runs_status_shape CHECK (
    (status = 'waiting'
       AND resume_cursor IS NOT NULL
       AND (wake_at IS NOT NULL OR wake_on IS NOT NULL)
       AND finished_at IS NULL)
    OR (status = 'running'
       AND wake_at IS NULL
       AND finished_at IS NULL)
    OR (status IN ('succeeded','failed','abandoned')
       AND resume_cursor IS NULL
       AND wake_at IS NULL
       AND finished_at IS NOT NULL)
  );

-- The chain shape: a resume row came from a resume trigger, a retry row
-- from a retry trigger, a row never continues itself, and a row is never
-- both a resume and a retry of another run.
ALTER TABLE workflow_runs
  ADD CONSTRAINT workflow_runs_trigger_shape CHECK (
    (resumed_from IS NULL OR trigger_kind = 'resume')
    AND (retry_of IS NULL OR trigger_kind = 'operator_retry')
    AND NOT (resumed_from IS NOT NULL AND retry_of IS NOT NULL)
    AND (resumed_from IS NULL OR resumed_from <> id)
    AND (retry_of IS NULL OR retry_of <> id)
  );

-- The ledger read: newest first, keyset-cursored on (created_at, id) — the
-- same shape as the webhook delivery ledger (webhook_admin.rs:333), so the
-- two logs page and behave alike.
CREATE INDEX idx_workflow_runs_ledger
  ON workflow_runs (org_id, created_at DESC, id DESC);
-- Filtered listings (by status, by rule) reuse the same key order.
CREATE INDEX idx_workflow_runs_status
  ON workflow_runs (org_id, status, created_at DESC, id DESC);
CREATE INDEX idx_workflow_runs_rule
  ON workflow_runs (org_id, rule_id, created_at DESC, id DESC);
-- The resume sweep: what is due, across every org, without scanning
-- history. Partial because only parked rows are ever due.
CREATE INDEX idx_workflow_runs_wake
  ON workflow_runs (wake_at) WHERE status = 'waiting';
-- Chain walks (a run's resume/retry lineage) in both directions.
CREATE INDEX idx_workflow_runs_chain_from ON workflow_runs (resumed_from);
CREATE INDEX idx_workflow_runs_chain_retry ON workflow_runs (retry_of);

ALTER TABLE workflow_runs ENABLE ROW LEVEL SECURITY;
CREATE POLICY workflow_runs_org ON workflow_runs
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- --- the audit-record guard (A2, mechanically enforced) ---------------------
--
-- A run moves through a lifecycle; it is never edited into a different
-- story. The identity, the document snapshot, the provenance and the chain
-- are immutable on UPDATE — only status, the park coordinates, the error
-- and the finish time may move. A lane that tries to rewrite a parked row
-- gets a database error, not a silent success.
--
-- The same function caps what a row may carry: the document snapshot, the
-- resume cursor and the error text are bounded so one runaway workflow
-- cannot bloat the highest-volume table this engine produces.

CREATE FUNCTION workflow_runs_ledger_guard() RETURNS trigger AS $$
BEGIN
  IF pg_column_size(NEW.rule_definition) > 262144
     OR (NEW.resume_cursor IS NOT NULL
         AND pg_column_size(NEW.resume_cursor) > 262144)
     OR (NEW.error IS NOT NULL AND length(NEW.error) > 4000)
  THEN
    RAISE EXCEPTION
      'workflow_runs: document, cursor or error exceeds the ledger cap (row %)', NEW.id;
  END IF;
  IF TG_OP = 'UPDATE' THEN
    IF NEW.org_id <> OLD.org_id
       OR NEW.rule_id IS DISTINCT FROM OLD.rule_id
       OR NEW.rule_name <> OLD.rule_name
       OR NEW.rule_version <> OLD.rule_version
       OR NEW.rule_definition <> OLD.rule_definition
       OR NEW.catalogue_version <> OLD.catalogue_version
       OR NEW.trigger_kind <> OLD.trigger_kind
       OR NEW.trigger_event_id IS DISTINCT FROM OLD.trigger_event_id
       OR NEW.trigger_job_id IS DISTINCT FROM OLD.trigger_job_id
       OR NEW.trigger_user_id IS DISTINCT FROM OLD.trigger_user_id
       OR NEW.resumed_from IS DISTINCT FROM OLD.resumed_from
       OR NEW.retry_of IS DISTINCT FROM OLD.retry_of
       OR NEW.started_at <> OLD.started_at
       OR NEW.created_at <> OLD.created_at
    THEN
      RAISE EXCEPTION
        'workflow_runs is an audit ledger: identity and provenance columns are immutable (row %)',
        OLD.id;
    END IF;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER workflow_runs_ledger_immutable
  BEFORE INSERT OR UPDATE ON workflow_runs
  FOR EACH ROW EXECUTE FUNCTION workflow_runs_ledger_guard();

-- ============================================================================
-- (B) one row per step attempt
-- ============================================================================

CREATE TABLE workflow_run_steps (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  run_id uuid NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,

  -- Position in the run's execution order, assigned by the executor. A
  -- bounded `foreach` iteration gets its OWN index rather than reusing one,
  -- which is what makes (run_id, step_index, attempt) a real identity.
  step_index integer NOT NULL CHECK (step_index >= 0),
  -- The registry node that ran. Denormalized beside the run's
  -- `catalogue_version` so a step still reads after a release renames a
  -- label; an unknown node id is kept rather than dropped (the ledger
  -- records what ran, including things a later build no longer knows).
  node_id text NOT NULL,
  node_label text NOT NULL,
  -- 1 for the first try, higher for an in-run retry of the SAME step: a
  -- retried step appends a row instead of overwriting the last one.
  attempt integer NOT NULL DEFAULT 1 CHECK (attempt >= 1),
  -- The bounded fan-out iteration this attempt belongs to (NULL when the
  -- node ran once).
  iteration integer CHECK (iteration IS NULL OR iteration >= 0),

  status text NOT NULL DEFAULT 'running'
    CHECK (status IN ('running','waiting','succeeded','failed','abandoned','skipped')),
  input jsonb NOT NULL DEFAULT '{}' CHECK (jsonb_typeof(input) = 'object'),
  output jsonb,
  -- Machine-readable reason the step failed, beside the prose. A refusal
  -- names itself; a broken dispatch does not have to.
  error_code text,
  error text CHECK (error IS NULL OR length(error) <= 4000),

  started_at timestamptz NOT NULL DEFAULT now(),
  finished_at timestamptz,

  -- The append-only contract: one row per attempt. A double-write of the
  -- same attempt is a bug, and it fails loudly instead of silently
  -- overwriting the step the operator is looking at.
  UNIQUE (run_id, step_index, attempt)
);

-- A step either finished (and says when) or did not (and says nothing).
-- `skipped` is a TERMINAL fact — a branch not taken is recorded, not left
-- looking half-run.
ALTER TABLE workflow_run_steps
  ADD CONSTRAINT workflow_run_steps_status_shape CHECK (
    (status IN ('succeeded','failed','abandoned','skipped') AND finished_at IS NOT NULL)
    OR (status IN ('running','waiting') AND finished_at IS NULL)
  );

-- The trace read: the run's steps in execution order.
CREATE INDEX idx_workflow_run_steps_run
  ON workflow_run_steps (run_id, step_index, attempt);
-- Cross-run step analytics (which node fails, how often).
CREATE INDEX idx_workflow_run_steps_node ON workflow_run_steps (org_id, node_id, started_at DESC);

ALTER TABLE workflow_run_steps ENABLE ROW LEVEL SECURITY;
CREATE POLICY workflow_run_steps_org ON workflow_run_steps
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- The same audit posture as the run: a step's identity, its input and its
-- start time never change; only its lifecycle does. The input a step
-- received is part of the record of what happened, so re-writing it to
-- match a later retry would be the same fiction the run guard refuses.
CREATE FUNCTION workflow_run_steps_ledger_guard() RETURNS trigger AS $$
BEGIN
  IF (NEW.input IS NOT NULL AND pg_column_size(NEW.input) > 65536)
     OR (NEW.output IS NOT NULL AND pg_column_size(NEW.output) > 262144)
     OR (NEW.error IS NOT NULL AND length(NEW.error) > 4000)
  THEN
    RAISE EXCEPTION
      'workflow_run_steps: input, output or error exceeds the ledger cap (step %)', NEW.id;
  END IF;
  IF TG_OP = 'UPDATE' THEN
    IF NEW.org_id <> OLD.org_id
       OR NEW.run_id <> OLD.run_id
       OR NEW.step_index <> OLD.step_index
       OR NEW.node_id <> OLD.node_id
       OR NEW.node_label <> OLD.node_label
       OR NEW.attempt <> OLD.attempt
       OR NEW.iteration IS DISTINCT FROM OLD.iteration
       OR NEW.input <> OLD.input
       OR NEW.started_at <> OLD.started_at
    THEN
      RAISE EXCEPTION
        'workflow_run_steps is an audit ledger: identity and input columns are immutable (step %)',
        OLD.id;
    END IF;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER workflow_run_steps_ledger_immutable
  BEFORE INSERT OR UPDATE ON workflow_run_steps
  FOR EACH ROW EXECUTE FUNCTION workflow_run_steps_ledger_guard();
