-- The workflow run ledger's referential releases (Wave 56 lane `p56-retention-purge`).
--
-- Record `worker/run-retention-sweep/purge-aborts-on-fk-setnull-vs-ledger-guard`.
--
-- WHAT WAS BROKEN. 0129's `workflow_runs_ledger_guard` makes every identity and
-- provenance column immutable on UPDATE, and lists the five foreign keys among
-- them. Every one of those five is declared `ON DELETE SET NULL` (rule_id,
-- trigger_event_id, trigger_job_id, trigger_user_id, resumed_from, retry_of).
-- A referential action is an UPDATE: when the referenced row is deleted,
-- PostgreSQL runs
--
--   UPDATE ONLY workflow_runs SET retry_of = NULL WHERE retry_of = <deleted id>
--
-- which fires the same BEFORE UPDATE guard, which saw a provenance column
-- change and raised. So the declared reference was permanent and the declared
-- action was illegal at the same time. The visible symptom was the run
-- retention sweep: purging a retried run that finished 40 days ago aborts with
-- `workflow_runs is an audit ledger: identity and provenance columns are
-- immutable`, because its 10-day-old retry still points at it and is inside
-- the window. The same collision blocked three paths that have nothing to do
-- with retention: deleting an automation rule that has any run row, the
-- outbox purge (`prune_published`) releasing `trigger_event_id`, and the jobs
-- retention pass releasing `trigger_job_id`.
--
-- THE FIX, AND WHY IT IS NOT A WEAKENING. A run's story is never rewritten:
-- identity, name, version, the document snapshot, catalogue version, trigger
-- kind, start and create time stay immutable for every caller at every depth.
-- What the guard now permits is RELEASE, and only release: a pointer going
-- from a value to NULL, with every other column unchanged, and only while the
-- statement is a referential action (`pg_trigger_depth() >= 2` — the RI
-- trigger is the enclosing one; a plain application UPDATE sits at depth 1).
-- A pointer can therefore never be rewritten to a different fact, and an
-- application statement still cannot erase provenance by hand.
--
-- `resumed_from` and `retry_of` are the two columns this record is about; the
-- other three are the same bug in the same guard and are fixed here rather
-- than left for the next purge to abort on.
--
-- Non-purpose: no change to the ledger's shape, indexes, policies or the step
-- guard (`workflow_run_steps_ledger_guard` — its `run_id` is NOT NULL and
-- CASCADE, so no referential action ever updates it). No data is rewritten by
-- this migration; it replaces one function definition and nothing else.

CREATE OR REPLACE FUNCTION workflow_runs_ledger_guard() RETURNS trigger AS $$
DECLARE
  -- TRUE when the enclosing statement is a referential action rather than
  -- application code: the RI trigger sits at depth 1 and this guard at depth
  -- 2 (a plain UPDATE reaches the guard directly, at depth 1).
  from_referential_action boolean := pg_trigger_depth() >= 2;
  pointers_changed boolean;
  release_only boolean;
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
    -- Never writable by anyone, referential action or not.
    IF NEW.org_id <> OLD.org_id
       OR NEW.rule_name <> OLD.rule_name
       OR NEW.rule_version <> OLD.rule_version
       OR NEW.rule_definition <> OLD.rule_definition
       OR NEW.catalogue_version <> OLD.catalogue_version
       OR NEW.trigger_kind <> OLD.trigger_kind
       OR NEW.started_at <> OLD.started_at
       OR NEW.created_at <> OLD.created_at
    THEN
      RAISE EXCEPTION
        'workflow_runs is an audit ledger: identity and provenance columns are immutable (row %)',
        OLD.id;
    END IF;

    -- The five declared references. They may be RELEASED when the row they
    -- name is deleted (the FK's own SET NULL), and nothing else: a rewrite to
    -- a different value, or a release written by application code, is still
    -- the edit this ledger refuses.
    pointers_changed :=
         NEW.rule_id IS DISTINCT FROM OLD.rule_id
      OR NEW.trigger_event_id IS DISTINCT FROM OLD.trigger_event_id
      OR NEW.trigger_job_id IS DISTINCT FROM OLD.trigger_job_id
      OR NEW.trigger_user_id IS DISTINCT FROM OLD.trigger_user_id
      OR NEW.resumed_from IS DISTINCT FROM OLD.resumed_from
      OR NEW.retry_of IS DISTINCT FROM OLD.retry_of;
    release_only :=
         (NEW.rule_id IS NOT DISTINCT FROM OLD.rule_id
            OR (OLD.rule_id IS NOT NULL AND NEW.rule_id IS NULL))
     AND (NEW.trigger_event_id IS NOT DISTINCT FROM OLD.trigger_event_id
            OR (OLD.trigger_event_id IS NOT NULL AND NEW.trigger_event_id IS NULL))
     AND (NEW.trigger_job_id IS NOT DISTINCT FROM OLD.trigger_job_id
            OR (OLD.trigger_job_id IS NOT NULL AND NEW.trigger_job_id IS NULL))
     AND (NEW.trigger_user_id IS NOT DISTINCT FROM OLD.trigger_user_id
            OR (OLD.trigger_user_id IS NOT NULL AND NEW.trigger_user_id IS NULL))
     AND (NEW.resumed_from IS NOT DISTINCT FROM OLD.resumed_from
            OR (OLD.resumed_from IS NOT NULL AND NEW.resumed_from IS NULL))
     AND (NEW.retry_of IS NOT DISTINCT FROM OLD.retry_of
            OR (OLD.retry_of IS NOT NULL AND NEW.retry_of IS NULL));

    IF pointers_changed AND NOT (from_referential_action AND release_only) THEN
      RAISE EXCEPTION
        'workflow_runs is an audit ledger: identity and provenance columns are immutable (row %)',
        OLD.id;
    END IF;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;
