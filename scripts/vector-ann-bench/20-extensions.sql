-- Engine B: one Postgres carrying pgvector 0.8.x, VectorChord 0.5.3 and
-- pgvectorscale 0.9.1, so every arm shares one engine build and one corpus.
-- The exact pass runs here too: a cross-engine reading needs each engine's own
-- baseline, not the other engine's.
--
-- The tenant index is dropped for the ANN arms for the reason given in
-- 10-pgvector.sql: with it present the planner reroutes to the filtered exact
-- scan, which measures the reroute rather than the index.

\set ON_ERROR_STOP on

SET bench.engine = 'pg18+vchord+vectorscale';
SET maintenance_work_mem = '2GB';

SET ROLE marula_app;
SELECT bench.exact_pass('S', :iters);
SELECT bench.exact_pass('L', :iters);
SELECT bench.capture_plan('exact', 'S');
SELECT bench.capture_plan('exact', 'L');

SET enable_seqscan = off;
SELECT bench.run_arm('exact-btree', 'S', :iters);
SELECT bench.run_arm('exact-btree', 'L', :iters);
RESET ROLE;

-- Global ground truth, before any ANN index exists, so the index-quality
-- control has a reference on this engine too.
SET enable_seqscan = on;
SELECT bench.exact_global_pass('S', :iters);
SELECT bench.exact_global_pass('L', :iters);

SELECT bench.isolate();

-- VectorChord, hierarchical quantisation index. Built with the extension's
-- defaults: 0.5.3 rejects the `lists` reloption that earlier releases took
-- (`unrecognized parameter "lists"`), and its documented build knobs are not
-- reloptions at all. Defaults are also the fair setting, since tuning one
-- index and not the others would flatter it.
SELECT bench.build_index('vchordrq',
  'CREATE INDEX idx_bench_vchordrq ON search_embeddings USING vchordrq (embedding vector_cosine_ops)');
SELECT bench.record_size('vchordrq', 'idx_bench_vchordrq');

SET ROLE marula_app;
SET enable_seqscan = off;
SET vchordrq.prefilter = off;
SELECT bench.run_arm('vchordrq', 'S', :iters);
SELECT bench.run_arm('vchordrq', 'L', :iters);
SELECT bench.capture_plan('vchordrq', 'S');
SELECT bench.capture_plan('vchordrq', 'L');

-- The filtered path: VectorChord evaluates the predicate before the scan.
SET vchordrq.prefilter = on;
SELECT bench.run_arm('vchordrq-prefilter', 'S', :iters);
SELECT bench.run_arm('vchordrq-prefilter', 'L', :iters);
SELECT bench.capture_plan('vchordrq-prefilter', 'S');
SELECT bench.capture_plan('vchordrq-prefilter', 'L');
RESET ROLE;

DROP INDEX idx_bench_vchordrq;

-- VectorChord's graph index, the memory-resident alternative.
SELECT bench.build_index('vchordg',
  'CREATE INDEX idx_bench_vchordg ON search_embeddings USING vchordg (embedding vector_cosine_ops)');
SELECT bench.record_size('vchordg', 'idx_bench_vchordg');

SET ROLE marula_app;
SET enable_seqscan = off;
SELECT bench.run_arm('vchordg', 'S', :iters);
SELECT bench.run_arm('vchordg', 'L', :iters);
SELECT bench.capture_plan('vchordg', 'S');
SELECT bench.capture_plan('vchordg', 'L');
RESET ROLE;

DROP INDEX idx_bench_vchordg;

SELECT bench.restore_indexes();
