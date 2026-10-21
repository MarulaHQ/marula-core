-- pgvectorscale StreamingDiskANN, in its own file because this build is the
-- one that can take the server down. On an 8 GB Docker VM the parallel build
-- of 302,000 vectors at 768 dimensions killed the postmaster, which cost the
-- whole engine-B run its results. Kept separate so a crash here loses this
-- arm and nothing else, and with parallelism off and a smaller work memory so
-- it has the best chance of finishing.
--
-- Two arms:
--   diskann        the tenant filter stays a query predicate applied after the
--                  graph walk, same as every other arm
--   diskann-label  the tenant becomes an index label, so the filter is part of
--                  the index rather than a predicate over its output

\set ON_ERROR_STOP on

SET bench.engine = 'pg18+vchord+vectorscale';
SET maintenance_work_mem = '512MB';
SET max_parallel_maintenance_workers = 0;

SELECT bench.isolate();

SELECT bench.build_index('diskann',
  'CREATE INDEX idx_bench_diskann ON search_embeddings USING diskann (embedding vector_cosine_ops)');
SELECT bench.record_size('diskann', 'idx_bench_diskann');

SET ROLE marula_app;
SET enable_seqscan = off;
SELECT bench.run_arm('diskann', 'S', :iters);
SELECT bench.run_arm('diskann', 'L', :iters);
SELECT bench.capture_plan('diskann', 'S');
SELECT bench.capture_plan('diskann', 'L');
RESET ROLE;

DROP INDEX idx_bench_diskann;

-- The labelled path. pgvectorscale 0.9.1 takes labels as smallint[], not the
-- scalar smallint earlier releases used: the operator class is registered for
-- intype smallint[] and rejects a scalar column ("operator class
-- vector_smallint_label_ops does not accept data type smallint"). The index
-- also wants the vector column first; label-first fails with "indexed column
-- has no valid vector dimension". Both are recorded here because the published
-- examples show the older shape.
--
-- The tenant becomes one element of the label array, so the tenant filter is
-- part of the index rather than a predicate over its output.
ALTER TABLE search_embeddings ADD COLUMN labels smallint[];
UPDATE search_embeddings e
   SET labels = ARRAY[(t.rn)::smallint]
  FROM (SELECT id, row_number() OVER (ORDER BY label) AS rn FROM bench.tenant_ids) t
 WHERE e.org_id = t.id;
GRANT SELECT (labels) ON search_embeddings TO marula_app;

SELECT bench.build_index('diskann-label',
  'CREATE INDEX idx_bench_diskann_label ON search_embeddings USING diskann (embedding vector_cosine_ops, labels vector_smallint_label_ops)');
SELECT bench.record_size('diskann-label', 'idx_bench_diskann_label');

SET ROLE marula_app;

CREATE OR REPLACE FUNCTION bench.candidate_ids_label(p_org uuid, p_label smallint, p_q text, p_limit int)
RETURNS uuid[] LANGUAGE plpgsql AS $$
DECLARE ids uuid[];
BEGIN
  EXECUTE $sql$
    SELECT array_agg(entity_id ORDER BY dist, entity_kind, entity_id)
      FROM (
        SELECT e.entity_id, e.entity_kind, e.embedding <=> $1::text::vector AS dist
          FROM search_embeddings e
         WHERE e.org_id = $2
           AND e.labels @> ARRAY[$3]::smallint[]
           AND e.entity_kind = ANY(ARRAY['ticket','comment','client','site','contact',
                                         'kb_article','document'])
         ORDER BY e.embedding <=> $1::text::vector, e.entity_kind, e.entity_id
         LIMIT $4
      ) hit
  $sql$ INTO ids
  USING p_q, p_org, p_label, p_limit;
  RETURN coalesce(ids, ARRAY[]::uuid[]);
END $$;

CREATE OR REPLACE FUNCTION bench.run_arm_label(p_arm text, p_tenant text, p_iters int)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
  org uuid;
  lbl smallint;
  qu record;
  t0 timestamptz;
  i int;
  ids uuid[];
  warm int;
BEGIN
  SELECT id INTO org FROM bench.tenant_ids WHERE label = p_tenant;
  -- Order matters: the label lookup reads the table, so the policy has to be
  -- scoped first. Doing it the other way returned NULL, and a NULL label
  -- filtered every query to zero rows while the arm still looked like it ran.
  PERFORM set_config('app.org_id', org::text, false);
  SELECT e.labels[1] INTO lbl FROM search_embeddings e WHERE e.org_id = org LIMIT 1;
  IF lbl IS NULL THEN
    RAISE EXCEPTION 'tenant % has no label value', p_tenant;
  END IF;

  FOR warm IN 1..5 LOOP
    PERFORM bench.candidate_ids_label(org, lbl, q.qtext, 10) FROM bench.queries q
     WHERE q.tenant = p_tenant AND q.q = warm % 20 + 1;
  END LOOP;

  FOR i IN 1..p_iters LOOP
    SELECT q.qtext INTO qu FROM bench.queries q
     WHERE q.tenant = p_tenant AND q.q = i % 20 + 1;
    t0 := clock_timestamp();
    PERFORM bench.candidate_ids_label(org, lbl, qu.qtext, 10);
    INSERT INTO bench.timings
    VALUES (current_setting('bench.engine'), p_arm, p_tenant, i,
            extract(epoch FROM clock_timestamp() - t0) * 1000);
  END LOOP;

  FOR qu IN SELECT * FROM bench.queries WHERE tenant = p_tenant LOOP
    ids := bench.candidate_ids_label(org, lbl, qu.qtext, 10);
    INSERT INTO bench.arm_ids VALUES (p_arm, p_tenant, qu.q, ids)
      ON CONFLICT (arm, tenant, q) DO UPDATE SET ids = EXCLUDED.ids;
  END LOOP;
END $$;

SET enable_seqscan = off;
SELECT bench.run_arm_label('diskann-label', 'S', :iters);
SELECT bench.run_arm_label('diskann-label', 'L', :iters);
RESET ROLE;

DROP INDEX idx_bench_diskann_label;
ALTER TABLE search_embeddings DROP COLUMN labels;
SELECT bench.restore_indexes();
