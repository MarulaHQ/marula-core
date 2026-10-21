-- Benchmark harness functions.
--
-- The measured statement is the production vector leg, character for
-- character in its predicates and ordering
-- (crates/core-db/src/search.rs:650-673), with the org id and the query
-- embedding supplied as arguments. Nothing else is added or removed: if this
-- query is fast here, the product query is fast for the same reason.
--
-- Everything runs as marula_app, which owns no table in the fixture and is
-- not a superuser, so the RLS policy is enforced rather than bypassed.

\set ON_ERROR_STOP on

SET ROLE marula_app;

-- The twenty query embeddings per tenant: the first twenty rows by id, which
-- spreads them across topics because ids are md5.
--
-- This has to run tenant by tenant with app.org_id set. The first version of
-- this file inserted them in one statement, RLS filtered every row out, and
-- the run reported empty result sets as if they were measurements. The guard
-- below turns that failure into an error.
CREATE TABLE bench.queries (
  tenant text NOT NULL,
  q int NOT NULL,
  qtext text NOT NULL,
  PRIMARY KEY (tenant, q)
);

DO $$
DECLARE t record;
BEGIN
  FOR t IN SELECT label, id FROM bench.tenant_ids WHERE label IN ('S', 'L') ORDER BY label LOOP
    PERFORM set_config('app.org_id', t.id::text, false);
    INSERT INTO bench.queries
    SELECT t.label,
           row_number() OVER (ORDER BY x.id),
           x.txt
      FROM (SELECT e.id, e.embedding::text AS txt
              FROM search_embeddings e
             WHERE e.org_id = t.id AND e.entity_kind = 'ticket'
             ORDER BY e.id
             LIMIT 20) x;
  END LOOP;
END $$;

-- In the real-embedding run 03-queries-real.sql replaces this set, so the
-- guard is skipped when the driver says so.
\if :{?real_corpus}
\else
DO $$
DECLARE n int;
BEGIN
  SELECT count(*) INTO n FROM bench.queries;
  IF n <> 40 THEN
    RAISE EXCEPTION 'query set is % rows, expected 40 (two tenants x 20); RLS probably hid the rows', n;
  END IF;
END $$;
\endif

-- Manual check: the plan this benchmark must exercise.
CREATE OR REPLACE FUNCTION bench.candidate_ids(p_org uuid, p_q text, p_limit int DEFAULT 10)
RETURNS uuid[] LANGUAGE plpgsql AS $$
DECLARE ids uuid[];
BEGIN
  EXECUTE $sql$
    SELECT array_agg(entity_id ORDER BY dist, entity_kind, entity_id)
      FROM (
        SELECT e.entity_id, e.entity_kind, e.embedding <=> $1::text::vector AS dist
          FROM search_embeddings e
         WHERE e.org_id = $2
           AND e.entity_kind = ANY(ARRAY['ticket','comment','client','site','contact',
                                         'kb_article','document'])
           AND (
             e.entity_kind <> 'document'
             OR $3::uuid IS NULL
             OR EXISTS (
               SELECT 1 FROM search_index s
                WHERE s.org_id = e.org_id
                  AND s.entity_kind = e.entity_kind
                  AND s.entity_id = e.entity_id
                  AND (s.client_scope IS NULL OR s.client_scope = $3)
             )
           )
         ORDER BY e.embedding <=> $1::text::vector, e.entity_kind, e.entity_id
         LIMIT $4
      ) hit
  $sql$ INTO ids
  USING p_q, p_org, NULL::uuid, p_limit;
  RETURN coalesce(ids, ARRAY[]::uuid[]);
END $$;

-- Exact pass. Runs before any ANN index exists, so the only plan available is
-- the one production uses today; that result set is the recall ground truth.
CREATE OR REPLACE FUNCTION bench.exact_pass(p_tenant text, p_iters int)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
  org uuid;
  qu record;
  t0 timestamptz;
  i int;
  ids uuid[];
  warm int;
BEGIN
  SELECT id INTO org FROM bench.tenant_ids WHERE label = p_tenant;
  PERFORM set_config('app.org_id', org::text, false);

  FOR warm IN 1..5 LOOP
    PERFORM bench.candidate_ids(org, q.qtext, 10) FROM bench.queries q
     WHERE q.tenant = p_tenant AND q.q = warm % 20 + 1;
  END LOOP;

  FOR i IN 1..p_iters LOOP
    SELECT q.qtext INTO qu FROM bench.queries q
     WHERE q.tenant = p_tenant AND q.q = i % 20 + 1;
    t0 := clock_timestamp();
    PERFORM bench.candidate_ids(org, qu.qtext, 10);
    INSERT INTO bench.timings
    VALUES (current_setting('bench.engine'), 'exact', p_tenant, i,
            extract(epoch FROM clock_timestamp() - t0) * 1000);
  END LOOP;

  -- Ground truth for recall: every query vector, equally unmeasured.
  FOR qu IN SELECT * FROM bench.queries WHERE tenant = p_tenant LOOP
    ids := bench.candidate_ids(org, qu.qtext, 10);
    INSERT INTO bench.exact_ids VALUES (p_tenant, qu.q, ids);
    INSERT INTO bench.arm_ids VALUES ('exact', p_tenant, qu.q, ids)
      ON CONFLICT DO NOTHING;
  END LOOP;

  IF (SELECT count(*) FROM bench.exact_ids WHERE tenant = p_tenant) <> 20 THEN
    RAISE EXCEPTION 'exact pass stored % result sets for tenant %, expected 20',
      (SELECT count(*) FROM bench.exact_ids WHERE tenant = p_tenant), p_tenant;
  END IF;
END $$;

-- One ANN arm. Session-level GUCs (iterative scan, prefilter, probes) are set
-- by the caller before the call, and the dynamic SQL re-plans each time, so an
-- arm cannot inherit the previous arm's plan.
CREATE OR REPLACE FUNCTION bench.run_arm(p_arm text, p_tenant text, p_iters int)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
  org uuid;
  qu record;
  t0 timestamptz;
  i int;
  ids uuid[];
  warm int;
BEGIN
  SELECT id INTO org FROM bench.tenant_ids WHERE label = p_tenant;
  PERFORM set_config('app.org_id', org::text, false);

  FOR warm IN 1..5 LOOP
    PERFORM bench.candidate_ids(org, q.qtext, 10) FROM bench.queries q
     WHERE q.tenant = p_tenant AND q.q = warm % 20 + 1;
  END LOOP;

  FOR i IN 1..p_iters LOOP
    SELECT q.qtext INTO qu FROM bench.queries q
     WHERE q.tenant = p_tenant AND q.q = i % 20 + 1;
    t0 := clock_timestamp();
    PERFORM bench.candidate_ids(org, qu.qtext, 10);
    INSERT INTO bench.timings
    VALUES (current_setting('bench.engine'), p_arm, p_tenant, i,
            extract(epoch FROM clock_timestamp() - t0) * 1000);
  END LOOP;

  FOR qu IN SELECT * FROM bench.queries WHERE tenant = p_tenant LOOP
    ids := bench.candidate_ids(org, qu.qtext, 10);
    INSERT INTO bench.arm_ids VALUES (p_arm, p_tenant, qu.q, ids)
      ON CONFLICT (arm, tenant, q) DO UPDATE SET ids = EXCLUDED.ids;
  END LOOP;

  IF (SELECT count(*) FROM bench.arm_ids WHERE arm = p_arm AND tenant = p_tenant) <> 20 THEN
    RAISE EXCEPTION 'arm % stored % result sets for tenant %, expected 20',
      p_arm, (SELECT count(*) FROM bench.arm_ids WHERE arm = p_arm AND tenant = p_tenant), p_tenant;
  END IF;
END $$;

CREATE OR REPLACE FUNCTION bench.candidate_ids_global(p_q text, p_limit int)
RETURNS uuid[] LANGUAGE plpgsql AS $$
DECLARE ids uuid[];
BEGIN
  EXECUTE $sql$
    SELECT array_agg(entity_id ORDER BY dist, entity_kind, entity_id)
      FROM (
        SELECT e.entity_id, e.entity_kind, e.embedding <=> $1::text::vector AS dist
          FROM search_embeddings e
         ORDER BY e.embedding <=> $1::text::vector, e.entity_kind, e.entity_id
         LIMIT $2
      ) hit
  $sql$ INTO ids
  USING p_q, p_limit;
  RETURN coalesce(ids, ARRAY[]::uuid[]);
END $$;

-- Ground truth for the index quality question, taken as the owner with no
-- tenant filter in the statement. Runs before any ANN index exists.
CREATE OR REPLACE FUNCTION bench.exact_global_pass(p_tenant text, p_iters int)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
  qu record;
  t0 timestamptz;
  i int;
  ids uuid[];
  warm int;
BEGIN
  FOR warm IN 1..5 LOOP
    PERFORM bench.candidate_ids_global(q.qtext, 10) FROM bench.queries q
     WHERE q.tenant = p_tenant AND q.q = warm % 20 + 1;
  END LOOP;

  FOR i IN 1..p_iters LOOP
    SELECT q.qtext INTO qu FROM bench.queries q
     WHERE q.tenant = p_tenant AND q.q = i % 20 + 1;
    t0 := clock_timestamp();
    PERFORM bench.candidate_ids_global(qu.qtext, 10);
    INSERT INTO bench.timings
    VALUES (current_setting('bench.engine'), 'exact-global', p_tenant, i,
            extract(epoch FROM clock_timestamp() - t0) * 1000);
  END LOOP;

  FOR qu IN SELECT * FROM bench.queries WHERE tenant = p_tenant LOOP
    ids := bench.candidate_ids_global(qu.qtext, 10);
    INSERT INTO bench.exact_global_ids VALUES (p_tenant, qu.q, ids);
    INSERT INTO bench.arm_ids VALUES ('exact-global', p_tenant, qu.q, ids)
      ON CONFLICT DO NOTHING;
  END LOOP;
END $$;

-- The index-quality control. Called by the table owner, so no policy applies
-- and no tenant predicate is in the statement: this is what the index alone
-- can do. The gap between this and the same index under the tenant filter is
-- the filter's cost, which is the thing migration 0062 was worried about.
CREATE OR REPLACE FUNCTION bench.run_arm_global(p_arm text, p_tenant text, p_iters int)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
  qu record;
  t0 timestamptz;
  i int;
  ids uuid[];
  warm int;
BEGIN
  FOR warm IN 1..5 LOOP
    PERFORM bench.candidate_ids_global(q.qtext, 10) FROM bench.queries q
     WHERE q.tenant = p_tenant AND q.q = warm % 20 + 1;
  END LOOP;

  FOR i IN 1..p_iters LOOP
    SELECT q.qtext INTO qu FROM bench.queries q
     WHERE q.tenant = p_tenant AND q.q = i % 20 + 1;
    t0 := clock_timestamp();
    PERFORM bench.candidate_ids_global(qu.qtext, 10);
    INSERT INTO bench.timings
    VALUES (current_setting('bench.engine'), p_arm, p_tenant, i,
            extract(epoch FROM clock_timestamp() - t0) * 1000);
  END LOOP;

  FOR qu IN SELECT * FROM bench.queries WHERE tenant = p_tenant LOOP
    ids := bench.candidate_ids_global(qu.qtext, 10);
    INSERT INTO bench.arm_ids VALUES (p_arm, p_tenant, qu.q, ids)
      ON CONFLICT (arm, tenant, q) DO UPDATE SET ids = EXCLUDED.ids;
  END LOOP;
END $$;

CREATE OR REPLACE FUNCTION bench.capture_plan(p_arm text, p_tenant text)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
  org uuid;
  qtext text;
  plan text := '';
  line record;
BEGIN
  SELECT id INTO org FROM bench.tenant_ids WHERE label = p_tenant;
  PERFORM set_config('app.org_id', org::text, false);
  SELECT q.qtext INTO qtext FROM bench.queries q
   WHERE q.tenant = p_tenant AND q.q = 1;
  FOR line IN EXECUTE
    'EXPLAIN (ANALYZE, BUFFERS, COSTS OFF) SELECT e.entity_id
       FROM search_embeddings e
      WHERE e.org_id = $1
        AND e.entity_kind = ANY(ARRAY[''ticket'',''comment'',''client'',''site'',''contact'',
                                      ''kb_article'',''document''])
       ORDER BY e.embedding <=> $2::vector
      LIMIT 10' USING org, qtext::vector
  LOOP
    plan := plan || line."QUERY PLAN" || E'\n';
  END LOOP;
  INSERT INTO bench.plans VALUES (current_setting('bench.engine'), p_arm, p_tenant, plan);
END $$;

-- Index build time goes into the record because it is a real cost and no
-- latency number shows it. Defined after RESET ROLE: CREATE INDEX needs the
-- table owner, and the benchmark role must not be the owner or RLS stops
-- applying to every other measurement in this file.
RESET ROLE;

CREATE OR REPLACE FUNCTION bench.build_index(p_arm text, p_ddl text)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, bench AS $$
DECLARE t0 timestamptz;
BEGIN
  t0 := clock_timestamp();
  EXECUTE p_ddl;
  EXECUTE 'ANALYZE search_embeddings';
  INSERT INTO bench.builds
  VALUES (current_setting('bench.engine'), p_arm,
          extract(epoch FROM clock_timestamp() - t0) * 1000, 0);
END $$;

CREATE OR REPLACE FUNCTION bench.record_size(p_arm text, p_index text)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, bench AS $$
BEGIN
  UPDATE bench.builds
     SET size_bytes = pg_relation_size(p_index)
   WHERE engine = current_setting('bench.engine') AND arm = p_arm;
END $$;

-- Isolation for the ANN arms. With the tenant index present the planner
-- refuses the ANN index at both measured tenant sizes and runs a filtered
-- exact scan instead, which measures the reroute rather than the index. The
-- unique constraint carries a second index on the same leading column, so
-- dropping the plain index alone was not enough: the first attempt at this
-- still measured the exact path at the small tenant and reported recall 1.000
-- against a 10 ms number that was the btree, not a graph walk.
CREATE OR REPLACE FUNCTION bench.isolate()
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, bench AS $$
DECLARE c record;
BEGIN
  DROP INDEX IF EXISTS public.idx_search_embeddings_org_kind;
  FOR c IN SELECT conname FROM pg_constraint
            WHERE conrelid = 'public.search_embeddings'::regclass AND contype = 'u' LOOP
    EXECUTE format('ALTER TABLE public.search_embeddings DROP CONSTRAINT %I', c.conname);
  END LOOP;
END $$;

-- Put the fixture back the way production has it, so the next engine and the
-- final plan captures see the real index set.
CREATE OR REPLACE FUNCTION bench.restore_indexes()
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, bench AS $$
BEGIN
  CREATE INDEX IF NOT EXISTS idx_search_embeddings_org_kind
    ON public.search_embeddings (org_id, entity_kind);
  IF NOT EXISTS (SELECT 1 FROM pg_constraint
                  WHERE conrelid = 'public.search_embeddings'::regclass AND contype = 'u') THEN
    ALTER TABLE public.search_embeddings
      ADD CONSTRAINT search_embeddings_org_kind_entity_key UNIQUE (org_id, entity_kind, entity_id);
  END IF;
END $$;
