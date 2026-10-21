-- Database observability extensions (Wave 52 lane P8,
-- `p52-db-observability-extensions`).
--
-- Why this exists: the perf program measures one endpoint at a time, so the
-- real query mix is unseen and the tail — which `docs/perf.md` §7 shows is
-- where dataset cost lands — is invisible. Four extensions were enabled
-- across 133 migrations (`pgcrypto`, `pg_trgm`, `btree_gist`, `vector`), all
-- of them for features rather than for seeing the server work. This
-- migration adds the seeing half: statement statistics, plan capture for the
-- slow tail, a maintenance scheduler, and hypothetical indexes.
--
-- What this migration adds, in order:
--   1. `pg_stat_statements`, `hypopg`, `pg_cron` — created only where the
--      server can actually run them, never assumed.
--   2. `ops.db_signals` — one row per signal saying whether it is live, so a
--      consumer asks instead of failing on a missing extension.
--   3. `ops.query_tail` — the query mix for this database, worst total time
--      first.
--   4. `ops.explain_plan()` — the EXPLAIN signal, as `jsonb`.
--   5. pg_cron jobs that vacuum and analyze the hot tables in a quiet hour.
--
-- Non-purpose: no automatic `pg_stat_statements_reset()` (a reset destroys
-- the evidence someone is reading — the operator decides), no retention of
-- plans outside the server log, no change to any application query, and no
-- index. The candidate index in `docs/perf.md` F4 stays unbuilt; `hypopg`
-- exists so it can be tried without building it.
--
-- Every extension below is created behind two guards: the server must offer
-- it (`pg_available_extensions`) and the command itself is wrapped, so a
-- managed Postgres that forbids an extension logs a notice and the chain
-- continues. A migration that cannot instrument a database must not stop it
-- from upgrading. `docs/site/reference/db-observability.md` carries the
-- provider matrix and the access path.

-- ---------------------------------------------------------------------------
-- 1. The extensions, each created only where it can run
-- ---------------------------------------------------------------------------

-- The operator schema is created by 0026 with the report read models; this
-- line only makes the rest of the file runnable on its own, which is how it is
-- tested and how an operator can apply it by hand.
CREATE SCHEMA IF NOT EXISTS ops;

-- `pg_stat_statements` needs `shared_preload_libraries` to *collect*, but
-- `CREATE EXTENSION` succeeds without it; the view then errors on read. That
-- asymmetry is why `ops.db_signals` reports installed and active separately.
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_available_extensions WHERE name = 'pg_stat_statements') THEN
    CREATE EXTENSION IF NOT EXISTS pg_stat_statements;
  ELSE
    RAISE NOTICE 'marula: pg_stat_statements is not available on this server; the query mix stays unseen';
  END IF;
EXCEPTION WHEN OTHERS THEN
  RAISE NOTICE 'marula: pg_stat_statements not created (%); the query mix stays unseen', SQLERRM;
END
$$;

-- `hypopg` needs no preload: the planner hook installs when a session first
-- calls one of its functions, and the hypothetical index dies with the
-- session. That is exactly the shape the F4 candidate wants.
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_available_extensions WHERE name = 'hypopg') THEN
    CREATE EXTENSION IF NOT EXISTS hypopg;
  ELSE
    RAISE NOTICE 'marula: hypopg is not available on this server; candidate indexes cannot be simulated';
  END IF;
EXCEPTION WHEN OTHERS THEN
  RAISE NOTICE 'marula: hypopg not created (%)', SQLERRM;
END
$$;

-- `pg_cron` is the one that fails loudly: it refuses to be created at all
-- unless the library is preloaded (`pg_cron can only be loaded via
-- shared_preload_libraries`) and unless this is the database named by
-- `cron.database_name`. Both conditions are checked before the attempt, and
-- the attempt is wrapped anyway because providers add their own rules.
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_available_extensions WHERE name = 'pg_cron') THEN
    RAISE NOTICE 'marula: pg_cron is not available on this server; maintenance jobs stay unscheduled';
  ELSIF NOT EXISTS (
    SELECT 1 FROM pg_settings
     WHERE name = 'shared_preload_libraries'
       AND setting ~ '(^|,)\s*pg_cron\s*(,|$)'
  ) THEN
    RAISE NOTICE 'marula: pg_cron is available but not preloaded; maintenance jobs stay unscheduled';
  ELSE
    CREATE EXTENSION IF NOT EXISTS pg_cron;
  END IF;
EXCEPTION WHEN OTHERS THEN
  RAISE NOTICE 'marula: pg_cron not created (%); maintenance jobs stay unscheduled', SQLERRM;
END
$$;

-- ---------------------------------------------------------------------------
-- 2. `ops.db_signals` — what is live, in one read
-- ---------------------------------------------------------------------------
--
-- The access path for everything below. A consumer (an API route, a report, a
-- human with psql) asks this first, then reads the signal it names. It does
-- not touch any optional extension object, so it works on a database where
-- none of them were created.
--
--   installed — the extension's SQL objects exist
--   active    — the server is actually doing the work (library preloaded)
--
-- The two differ for `pg_stat_statements` on a server that created the
-- extension but never set `shared_preload_libraries`, which is the common
-- managed-Postgres mistake. `auto_explain` has no SQL objects at all, so it
-- is never "installed": it is active or it is nothing.
CREATE OR REPLACE VIEW ops.db_signals WITH (security_invoker = true) AS
SELECT s.signal,
       s.installed,
       s.active,
       s.note
  FROM (
        VALUES
          ('pg_stat_statements',
           EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'pg_stat_statements'),
           COALESCE((SELECT setting FROM pg_settings WHERE name = 'shared_preload_libraries'), '')
             ~ '(^|,)\s*pg_stat_statements\s*(,|$)',
           'Per-statement call counts and timings. Read them through ops.query_tail.'),
          ('auto_explain',
           false,
           COALESCE((SELECT setting FROM pg_settings WHERE name = 'shared_preload_libraries'), '')
             ~ '(^|,)\s*auto_explain\s*(,|$)'
           OR COALESCE((SELECT setting FROM pg_settings WHERE name = 'session_preload_libraries'), '')
             ~ '(^|,)\s*auto_explain\s*(,|$)',
           'Logs the plan of any statement slower than auto_explain.log_min_duration. No SQL objects.'),
          ('pg_cron',
           EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'pg_cron'),
           EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'pg_cron')
             AND COALESCE((SELECT setting FROM pg_settings WHERE name = 'shared_preload_libraries'), '')
                 ~ '(^|,)\s*pg_cron\s*(,|$)',
           'Runs the scheduled VACUUM and ANALYZE jobs. Managed providers often forbid it.'),
          ('hypopg',
           EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'hypopg'),
           EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'hypopg'),
           'Hypothetical indexes for a session. No preload needed, nothing persisted.')
       ) AS s(signal, installed, active, note);

COMMENT ON VIEW ops.db_signals IS
  'One row per database observability signal: installed (SQL objects exist) and active (the server is doing the work).';

-- ---------------------------------------------------------------------------
-- 3. `ops.query_tail` — the query mix, worst total time first
-- ---------------------------------------------------------------------------
--
-- Built only when `pg_stat_statements` is installed *and* preloaded: the view
-- is a window onto a view that errors without preload, so creating it where
-- it cannot be read would turn a missing feature into a broken view.
--
-- Sorted by total time rather than mean time on purpose. The expensive thing
-- on a small install is usually a cheap query run a great many times; mean
-- time hides it and total time does not. `pct_time` says how much of the
-- measured server time one statement owns.
--
-- `rows_per_call` is the cheap shape check: a page query that returns 20 rows
-- and one that returns 4,000 differ there before any plan is read.
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'pg_stat_statements')
     AND COALESCE((SELECT setting FROM pg_settings WHERE name = 'shared_preload_libraries'), '')
         ~ '(^|,)\s*pg_stat_statements\s*(,|$)'
  THEN
    EXECUTE $view$
      CREATE OR REPLACE VIEW ops.query_tail WITH (security_invoker = true) AS
      SELECT s.queryid,
             left(s.query, 4000)                                   AS query,
             s.calls,
             round(s.total_exec_time::numeric, 1)                  AS total_ms,
             round(s.mean_exec_time::numeric, 2)                   AS mean_ms,
             round(s.max_exec_time::numeric, 1)                    AS max_ms,
             -- The statistics columns are double precision, so the ratio is
             -- cast before round() — round(double precision, int) does not
             -- exist and would fail only at read time.
             round((100 * s.total_exec_time
                    / NULLIF(sum(s.total_exec_time) OVER (), 0))::numeric, 2) AS pct_time,
             s.rows,
             round(s.rows::numeric / NULLIF(s.calls, 0), 1)        AS rows_per_call
        FROM pg_stat_statements s
       WHERE s.dbid = (SELECT oid FROM pg_database WHERE datname = current_database())
         AND s.calls > 0
         -- Reading the statistics is itself a statement; keeping it out stops
         -- the observer from climbing its own list.
         AND s.query NOT LIKE '%pg_stat_statements%'
       ORDER BY s.total_exec_time DESC
    $view$;

    COMMENT ON VIEW ops.query_tail IS
      'pg_stat_statements for this database, ordered by total execution time. Read ops.db_signals first.';
  ELSE
    RAISE NOTICE 'marula: ops.query_tail not created (pg_stat_statements is not installed and preloaded)';
  END IF;
EXCEPTION WHEN OTHERS THEN
  RAISE NOTICE 'marula: ops.query_tail not created (%)', SQLERRM;
END
$$;

-- ---------------------------------------------------------------------------
-- 4. `ops.explain_plan()` — the EXPLAIN signal
-- ---------------------------------------------------------------------------
--
-- One name, one return type: `SELECT ops.explain_plan('SELECT ...')` gives the
-- plan as `jsonb`, ready to read, diff or store. `with_analyze => true` runs the
-- statement and returns the measured plan, which is the only form that
-- carries actual row counts and buffer reads.
--
-- SECURITY INVOKER, deliberately. `EXPLAIN ANALYZE` executes the statement,
-- so a definer-rights version would let any caller write with the owner's
-- privileges. As an invoker function it can do exactly what the caller could
-- have typed, and nothing else.
--
-- `with_analyze => true` carries a 5s statement timeout: an accidental
-- `EXPLAIN ANALYZE` of a full-table aggregate should stop, not hold a
-- connection until someone notices.
--
-- The function sets its own `search_path`, so pass schema-qualified names for
-- anything outside `public` (for example `ops.v1_tickets`).
--
-- The second parameter is `with_analyze` rather than `analyze` because
-- `analyze` is a reserved keyword in SQL and cannot name a parameter.
-- Callers write `ops.explain_plan('...', with_analyze => true)`.
CREATE OR REPLACE FUNCTION ops.explain_plan(query_text text, with_analyze boolean DEFAULT false)
RETURNS jsonb
LANGUAGE plpgsql
SECURITY INVOKER
SET search_path = public
AS $$
DECLARE
  plan json;
BEGIN
  IF with_analyze THEN
    SET LOCAL statement_timeout = '5s';
    EXECUTE format('EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) %s', query_text) INTO plan;
  ELSE
    EXECUTE format('EXPLAIN (FORMAT JSON) %s', query_text) INTO plan;
  END IF;
  RETURN plan -> 0;
END
$$;

COMMENT ON FUNCTION ops.explain_plan(text, boolean) IS
  'The plan of one statement as jsonb. with_analyze => true runs it under a 5s timeout. Invoker rights.';

-- ---------------------------------------------------------------------------
-- 5. pg_cron — maintenance on the hot tables
-- ---------------------------------------------------------------------------
--
-- Small installs run one Postgres and no database operator. Autovacuum keeps
-- up with a quiet database and falls behind on the tables that churn, and the
-- tables that churn here are the ones the perf program measures: tickets, the
-- job queue, the outbox, the audit trail, search, and consumer deliveries.
--
-- One table per job, ten minutes apart, so a maintenance sweep never has two
-- vacuums competing for the same I/O window. Plain `VACUUM (ANALYZE)`: no
-- FULL, no FREEZE override, nothing that takes an exclusive lock beyond the
-- short ones vacuum already takes.
--
-- `cron.schedule(name, ...)` replaces a job of the same name, so re-running
-- this migration (or a later one repeating the call) updates rather than
-- duplicates. Tables that do not exist are skipped, which keeps this file
-- applicable to a database that is only partway through the chain.
--
-- What pg_cron cannot do here: it runs SQL, not the worker's Rust sweeps.
-- The housekeeping loops in `docs/worker-cadence.md` stay in the worker; what
-- moves to the database is the maintenance that was never a sweep at all.
DO $$
DECLARE
  job record;
BEGIN
  IF to_regclass('cron.job') IS NULL THEN
    RAISE NOTICE 'marula: pg_cron not installed; no maintenance jobs scheduled';
    RETURN;
  END IF;

  FOR job IN
    SELECT * FROM (VALUES
      ('marula-vacuum-tickets',             '10 3 * * *', 'public.tickets'),
      ('marula-vacuum-jobs',                '20 3 * * *', 'public.jobs'),
      ('marula-vacuum-outbox-events',       '30 3 * * *', 'public.outbox_events'),
      ('marula-vacuum-audit-events',        '40 3 * * *', 'public.audit_events'),
      ('marula-vacuum-search-index',        '50 3 * * *', 'public.search_index'),
      ('marula-vacuum-consumer-deliveries', '0 4 * * *',  'public.consumer_deliveries')
    ) AS t(jobname, schedule, relation)
  LOOP
    IF to_regclass(job.relation) IS NULL THEN
      CONTINUE;
    END IF;
    PERFORM cron.schedule(
      job.jobname,
      job.schedule,
      format('VACUUM (ANALYZE) %s', job.relation)
    );
  END LOOP;
EXCEPTION WHEN OTHERS THEN
  RAISE NOTICE 'marula: pg_cron jobs not scheduled (%)', SQLERRM;
END
$$;
