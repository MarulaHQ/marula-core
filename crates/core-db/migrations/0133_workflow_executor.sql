-- 0133_workflow_executor.sql
--
-- Wave 43 lane 15 (`p43-workflow-executor`). Two additive changes and one
-- index, and nothing that rewrites history.
--
-- (A) `automation_rules.definition` — the multi-step workflow DOCUMENT.
--     ADR 0016 decision 3 says one document has two editors: the rule
--     builder writes the simple tier into `conditions`/`actions`, and the
--     canvas (lane 16) edits the same row's `definition`. A row with a
--     `definition` is a workflow the executor runs; a row without one is
--     the simple tier the existing automations consumer already runs. The
--     two never both apply, and the executor's match is `definition IS NOT
--     NULL`, so a simple rule can never be run as a graph by accident.
--
-- (B) `automation_rules.revision` — the document revision lane 14's
--     `workflow_runs.rule_version` observed. Lane 14 recorded a constant 1
--     because no counter existed (its comment says exactly that). Now one
--     does, so a run's `rule_version` means "the revision this run read"
--     rather than "1 forever".
--
-- (C) `idx_workflow_runs_waiters` — the event router's WAITERS INDEX. Once
--     `flow.waitFor` exists, a parked run is woken by an EVENT (lane 14's
--     `idx_workflow_runs_wake` covers the time-based wakes; this covers the
--     event-based ones). The event consumer looks up `waiting` runs whose
--     `wake_on` equals the arriving event type; the partial index keeps that
--     lookup off the run history.
--
-- Non-purpose: no document validation (core-domain owns it: `workflow_document`
-- refuses a malformed or loop-unsafe document before it ever runs), no run
-- rows (lane 14), and no events emitted from this migration.

-- (A) + (B)
ALTER TABLE automation_rules
  ADD COLUMN definition jsonb,
  ADD COLUMN revision integer NOT NULL DEFAULT 1,
  -- Who authored the document. A workflow runs AS ITS AUTHOR, never as a
  -- system user with more power (spec item B): the executor sets
  -- `app.user_id` to this member for every node write, so RLS and the
  -- permission model see the same principal a manual run would. NULL for
  -- the simple tier and for rules predating this column; the executor
  -- REFUSES to run a document whose author is NULL rather than guessing.
  ADD COLUMN created_by uuid REFERENCES users(id) ON DELETE SET NULL;

ALTER TABLE automation_rules
  ADD CONSTRAINT automation_rules_revision_positive CHECK (revision >= 1),
  -- A definition, when present, is an OBJECT (the workflow document). The
  -- executor parses it with `WorkflowDocument::parse`, which refuses a
  -- non-object anyway; the CHECK makes the shape true in storage too.
  ADD CONSTRAINT automation_rules_definition_shape
    CHECK (definition IS NULL OR jsonb_typeof(definition) = 'object');

-- A workflow document is matched by trigger event; `idx_automation_rules_trigger`
-- already covers the simple tier, and this partial index keeps the executor's
-- per-event lookup over only the graph rules.
CREATE INDEX idx_automation_rules_workflow
  ON automation_rules (org_id, trigger_event)
  WHERE enabled AND definition IS NOT NULL;

-- (C) the waiters index: parked runs waiting on an event type, per org.
CREATE INDEX idx_workflow_runs_waiters
  ON workflow_runs (org_id, wake_on)
  WHERE status = 'waiting' AND wake_on IS NOT NULL;
