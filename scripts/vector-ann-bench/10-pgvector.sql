-- Engine A: the shipped image family, pgvector 0.8.x on PG18.
--
-- Arms, in the order they run:
--   exact        no ANN index, planner free. What production runs today.
--   exact-btree  the same query with the tenant index forced, which is the plan
--                the planner picks on its own as a tenant grows. It is the real
--                alternative to an ANN index and it costs no extension.
--   hnsw-ef*     HNSW, tenant indexes removed so the graph is the only ordered
--                path, at three ef_search settings. A recall target needs a
--                recall/latency curve, not one point on it.
--   hnsw-iter    HNSW with pgvector's iterative scan, which is the "no new
--                extension" answer to the filtered-scan objection.
--   hnsw-global  the same index with no tenant filter at all, run as the table
--                owner. This is the control that separates the index's own
--                quality from what the filter costs it.

\set ON_ERROR_STOP on

SET bench.engine = 'pgvector-0.8.6';
SET maintenance_work_mem = '2GB';

SET ROLE marula_app;
SELECT bench.exact_pass('S', :iters);
SELECT bench.exact_pass('L', :iters);
SELECT bench.capture_plan('exact', 'S');
SELECT bench.capture_plan('exact', 'L');

SET enable_seqscan = off;
SELECT bench.run_arm('exact-btree', 'S', :iters);
SELECT bench.run_arm('exact-btree', 'L', :iters);
SELECT bench.capture_plan('exact-btree', 'S');
SELECT bench.capture_plan('exact-btree', 'L');
RESET ROLE;

-- Global ground truth, before any ANN index exists.
SET enable_seqscan = on;
SELECT bench.exact_global_pass('S', :iters);
SELECT bench.exact_global_pass('L', :iters);

SELECT bench.isolate();

SELECT bench.build_index('hnsw', 'CREATE INDEX idx_bench_hnsw ON search_embeddings USING hnsw (embedding vector_cosine_ops)');
SELECT bench.record_size('hnsw', 'idx_bench_hnsw');

SET ROLE marula_app;
SET enable_seqscan = off;

SET hnsw.ef_search = 40;
SELECT bench.run_arm('hnsw-ef40', 'S', :iters);
SELECT bench.run_arm('hnsw-ef40', 'L', :iters);
SELECT bench.capture_plan('hnsw-ef40', 'S');
SELECT bench.capture_plan('hnsw-ef40', 'L');

SET hnsw.ef_search = 200;
SELECT bench.run_arm('hnsw-ef200', 'S', :iters);
SELECT bench.run_arm('hnsw-ef200', 'L', :iters);

SET hnsw.ef_search = 800;
SELECT bench.run_arm('hnsw-ef800', 'S', :iters);
SELECT bench.run_arm('hnsw-ef800', 'L', :iters);
SELECT bench.capture_plan('hnsw-ef800', 'S');

-- relaxed_order returns approximately the k nearest but not necessarily in
-- distance order. The vector leg feeds reciprocal rank fusion, where position
-- is the input, so strict_order is the setting a ranked product path can
-- actually ship. Both are measured so the choice has a number behind it.
SET hnsw.ef_search = 40;
SET hnsw.iterative_scan = relaxed_order;
SELECT bench.run_arm('hnsw-iter', 'S', :iters);
SELECT bench.run_arm('hnsw-iter', 'L', :iters);
SELECT bench.capture_plan('hnsw-iter', 'S');

SET hnsw.iterative_scan = strict_order;
SELECT bench.run_arm('hnsw-iter-strict', 'S', :iters);
SELECT bench.run_arm('hnsw-iter-strict', 'L', :iters);
SELECT bench.capture_plan('hnsw-iter-strict', 'S');
RESET ROLE;

-- The control: same index, same queries, no policy and no tenant predicate.
SET enable_seqscan = off;
SET hnsw.ef_search = 40;
SELECT bench.run_arm_global('hnsw-global-ef40', 'S', :iters);
SELECT bench.run_arm_global('hnsw-global-ef40', 'L', :iters);
SET hnsw.ef_search = 200;
SELECT bench.run_arm_global('hnsw-global-ef200', 'S', :iters);
SELECT bench.run_arm_global('hnsw-global-ef200', 'L', :iters);
SET hnsw.ef_search = 800;
SELECT bench.run_arm_global('hnsw-global-ef800', 'S', :iters);
SELECT bench.run_arm_global('hnsw-global-ef800', 'L', :iters);

DROP INDEX idx_bench_hnsw;
SELECT bench.restore_indexes();
