-- Audit trail: append-only at the database, not by convention.
--
-- Audit record (Wave 56):
--   crates/core-db/migrations/0001_orgs_auth.sql:
--     audit_events-no-db-append-only-guard-app-role-update-delete
--
-- What was true before this migration: `audit_events` was called append-only
-- in route comments and trusted to stay that way in review, and the
-- application role held SELECT, INSERT, UPDATE and DELETE on it — 0001's
-- blanket grant, re-issued to the rebranded role by 0114. RLS scoped the rows
-- per org; it did not make them immutable. One `UPDATE audit_events` from a
-- code path, or one `DELETE`, would edit evidence and leave no mark.
--
-- Three layers now, weakest first:
--
--   1. Privilege. The application role keeps SELECT and INSERT and loses
--      UPDATE and DELETE. The owner keeps them (it runs migrations and the
--      erasure verb), so this layer alone is not the guarantee.
--   2. Trigger. `audit_events_append_only_guard` refuses UPDATE, DELETE and
--      TRUNCATE for everyone, the owner included, except through one of the
--      three sanctioned paths named in the function below.
--   3. The sanctioned paths, each explicit:
--        * `purge_audit_events()` — retention. A SECURITY DEFINER function
--          that enforces the 365-day floor, deletes in bounded batches, and
--          is the only place a past-window row may go.
--        * the org cascade — an erasure or `seed-demo --force` deletes the
--          `orgs` row and the foreign key cascades here. Allowed when the
--          parent org row is already gone in this statement's snapshot.
--        * `app.dev_reset_bypass` — the 0119 escape the demo toolchain sets,
--          so a demo org can be wiped and reseeded. Tenant traffic never sets
--          it.
--
-- A future `GRANT UPDATE` on this table cannot restore mutability: the
-- trigger fires after the privilege check and answers to the statement, not
-- to the grant. A later reader should not "simplify" the guard by letting the
-- owner through — the owner is exactly who would run a careless delete, and
-- retention now goes through the function instead.
--
-- Non-purpose: no column is added. This migration changes who may mutate a
-- row, not its shape; the `org_usage` read model migration 0131 builds over
-- this table, and the export tail in `core_db::audit`, are untouched.

-- ---------------------------------------------------------------------------
-- 1. Privilege
-- ---------------------------------------------------------------------------
--
-- Both role names. A database migrated from before the rebrand still carries
-- `meridian_app` with 0001's blanket grant — nothing ever revoked it — so the
-- legacy login must lose the write verbs too, not just the current one. The
-- DO block keeps that step safe if a deployment has since dropped the old
-- role; `marula_app` is created by 0114 and is unconditional.
DO $$
BEGIN
  IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'meridian_app') THEN
    EXECUTE 'REVOKE UPDATE, DELETE ON audit_events FROM meridian_app';
  END IF;
END
$$;

REVOKE UPDATE, DELETE ON audit_events FROM marula_app;

-- ---------------------------------------------------------------------------
-- 2. The guard
-- ---------------------------------------------------------------------------
--
-- SECURITY INVOKER on purpose, unlike the receipt guard in 0134: the retention
-- path is recognised by `current_user` becoming the table owner inside
-- `purge_audit_events()`, and a SECURITY DEFINER guard would make every caller
-- look like the owner and erase that signal. The consequence is that the
-- org-existence probe below runs with the invoker's privileges, under RLS. In
-- that light it is the right question: a caller who cannot see the parent org
-- has no business deleting its rows, and every sanctioned cascade (the CLI
-- erasure, the test harness) runs on the owner pool, which sees it.
CREATE FUNCTION audit_events_append_only() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  -- TRUNCATE has no row and no NEW/OLD, so it is handled as its own branch in
  -- the statement-level trigger. The app role was never granted TRUNCATE, and
  -- this keeps a future `GRANT TRUNCATE` from being a bulk-erase hole.
  IF TG_OP = 'TRUNCATE' THEN
    IF current_setting('app.dev_reset_bypass', true) = 'on' THEN
      RETURN NULL;
    END IF;
    RAISE EXCEPTION 'audit_events is append-only: TRUNCATE rejected'
      USING ERRCODE = '42501',
            HINT = 'Retention deletes past-window rows through purge_audit_events(); erasure and dev reset remove the org row.';
  END IF;

  -- The demo toolchain's escape (0119): wipe the demo org by cascade, then
  -- reseed. Application traffic never sets this.
  IF current_setting('app.dev_reset_bypass', true) = 'on' THEN
    RETURN COALESCE(NEW, OLD);
  END IF;

  IF TG_OP = 'DELETE' THEN
    -- A tenant erasure deletes the org row; this foreign key cascades here and
    -- the parent is already invisible in this statement's snapshot (the same
    -- test the receipt guard in 0134 relies on). This is the erasure contract,
    -- not a hole: whoever can delete the org already holds the authority to
    -- delete everything it owns.
    IF NOT EXISTS (SELECT 1 FROM public.orgs WHERE id = OLD.org_id) THEN
      RETURN OLD;
    END IF;

    -- Retention, and only from inside purge_audit_events(). The marker is
    -- transaction-local and set there; the statement must also run as a
    -- member of the table's owning role, which is what a SECURITY DEFINER
    -- function body does and what a raw DELETE by the app role cannot fake.
    IF current_setting('marula.audit_retention', true) = 'on'
       AND pg_has_role(
             current_user,
             (SELECT relowner FROM pg_catalog.pg_class WHERE oid = TG_RELID),
             'MEMBER'
           )
    THEN
      RETURN OLD;
    END IF;
  END IF;

  RAISE EXCEPTION 'audit_events is append-only: % rejected', TG_OP
    USING ERRCODE = '42501',
          HINT = 'The trail is evidence. Retention calls purge_audit_events(); erasure deletes the org row; dev reset sets app.dev_reset_bypass.';
END;
$$;

CREATE TRIGGER audit_events_append_only_guard
  BEFORE UPDATE OR DELETE ON audit_events
  FOR EACH ROW EXECUTE FUNCTION audit_events_append_only();

CREATE TRIGGER audit_events_no_truncate
  BEFORE TRUNCATE ON audit_events
  FOR EACH STATEMENT EXECUTE FUNCTION audit_events_append_only();

-- ---------------------------------------------------------------------------
-- 3. Retention: one audited door
-- ---------------------------------------------------------------------------
--
-- The sweep used to run a bare `DELETE ... WHERE occurred_at < now() - $1` on
-- the owner pool. The guard above refuses that statement, which is the point,
-- so the window moves here: the predicate, the floor and the batch cap now sit
-- in one function a reviewer can read.
--
-- SECURITY DEFINER, so the body runs as the migration owner — the role the
-- guard accepts for the retention path. EXECUTE is granted to the app role so
-- the sweep is correct whichever pool a deployment hands the worker (the stock
-- compose gives it the owner pool; `MARULA_WORKER_APP_DATABASE_URL` exists for
-- deployments that do not).
--
-- The floor repeats the retention matrix's floor (`core_db::retention`)
-- because this is a public SQL surface and must not trust its caller. A window
-- below 365 days raises instead of clamping: a caller asking for less has a
-- bug, and silently keeping more would hide it. The batch cap is the sweep's
-- own ceiling (ten passes of 1,000 rows), so a single call can never be an
-- unbounded table-wide delete.
CREATE FUNCTION purge_audit_events(p_days integer, p_batch integer)
RETURNS bigint
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
  removed bigint;
BEGIN
  IF p_days IS NULL OR p_days < 365 THEN
    RAISE EXCEPTION 'purge_audit_events: % days is below the 365-day audit floor', p_days
      USING ERRCODE = '22023';
  END IF;
  IF p_batch IS NULL OR p_batch < 1 OR p_batch > 10000 THEN
    RAISE EXCEPTION 'purge_audit_events: batch must be between 1 and 10000 (got %)', p_batch
      USING ERRCODE = '22023';
  END IF;

  -- Transaction-local, read by the guard in the same statement tree. This
  -- marker alone unlocks nothing: the guard also requires the statement to run
  -- as the table owner, which only this body does.
  PERFORM set_config('marula.audit_retention', 'on', true);

  DELETE FROM audit_events
   WHERE ctid IN (
     SELECT ctid FROM audit_events
      WHERE occurred_at < now() - make_interval(days => p_days)
      ORDER BY occurred_at
      LIMIT p_batch
   );
  GET DIAGNOSTICS removed = ROW_COUNT;
  RETURN removed;
END;
$$;

REVOKE ALL ON FUNCTION purge_audit_events(integer, integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION purge_audit_events(integer, integer) TO marula_app;

COMMENT ON FUNCTION purge_audit_events(integer, integer) IS
  'The one sanctioned delete path for audit_events: past-window rows, bounded '
  'batch, 365-day floor. Called by the retention sweep. The append-only guard '
  'accepts nothing else except an org cascade and the dev-reset bypass.';

COMMENT ON TRIGGER audit_events_append_only_guard ON audit_events IS
  'Refuses UPDATE and DELETE. Sanctioned deletes: purge_audit_events(), an org '
  'cascade, and the dev-reset bypass (0119).';
