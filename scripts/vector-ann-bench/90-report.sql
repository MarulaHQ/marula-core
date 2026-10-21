-- Machine-readable result: one row per arm and tenant.
--
-- recall_at_10 is the mean over the twenty query vectors of
-- |arm result ∩ ground truth| / |ground truth|.
--
-- Which ground truth depends on the arm, and the difference matters:
--   * a filtered arm is scored against the tenant's exact top ten, so its
--     recall is what a user of that tenant would experience;
--   * a -global arm is scored against the unfiltered exact top ten, so its
--     recall is the index's own quality with no tenant filter in the way.
-- Scoring a global arm against a tenant's neighbours would report the filter
-- twice and call it an index defect.
--
-- avg_returned is how many rows the arm actually returned when ten were asked
-- for. This is the under-return question in its plainest form and it does not
-- depend on the corpus at all.

\pset format unaligned
\pset tuples_only on
\pset fieldsep '\t'

SELECT 'engine', 'arm', 'tenant', 'n', 'exact_rows', 'avg_returned', 'p50_ms', 'p95_ms',
       'max_ms', 'recall_at_10', 'index_mb', 'build_s';

WITH recall AS (
  SELECT a.arm, a.tenant,
         avg(cardinality(ARRAY(SELECT unnest(a.ids)
                                 INTERSECT
                               SELECT unnest(CASE WHEN a.arm LIKE '%global%' THEN g.ids ELSE e.ids END)))::numeric
             / greatest(cardinality(CASE WHEN a.arm LIKE '%global%' THEN g.ids ELSE e.ids END), 1))
           AS recall_at_10
    FROM bench.arm_ids a
    JOIN bench.exact_ids e ON e.tenant = a.tenant AND e.q = a.q
    JOIN bench.exact_global_ids g ON g.tenant = a.tenant AND g.q = a.q
   GROUP BY 1, 2
), returned AS (
  SELECT a.arm, a.tenant, avg(cardinality(a.ids))::numeric AS avg_returned
    FROM bench.arm_ids a
   GROUP BY 1, 2
)
SELECT t.engine,
       t.arm,
       t.tenant,
       count(*) AS n,
       (SELECT round(avg(cardinality(ids))::numeric, 1) FROM bench.exact_ids x WHERE x.tenant = t.tenant) AS exact_rows,
       round(coalesce(max(rt.avg_returned), 0), 1) AS avg_returned,
       round(percentile_disc(0.5) WITHIN GROUP (ORDER BY t.ms)::numeric, 2) AS p50_ms,
       round(percentile_disc(0.95) WITHIN GROUP (ORDER BY t.ms)::numeric, 2) AS p95_ms,
       round(max(t.ms)::numeric, 2) AS max_ms,
       round(r.recall_at_10::numeric, 3) AS recall_at_10,
       round((coalesce(b.size_bytes, 0) / 1048576.0)::numeric, 1) AS index_mb,
       round((coalesce(b.build_ms, 0) / 1000.0)::numeric, 1) AS build_s
  FROM bench.timings t
  LEFT JOIN recall r ON r.arm = t.arm AND r.tenant = t.tenant
  LEFT JOIN returned rt ON rt.arm = t.arm AND rt.tenant = t.tenant
  -- An index is built once and used by several arms (hnsw, then hnsw-ef40,
  -- hnsw-ef200, ...). Attaching the build row by prefix keeps its size and
  -- build time on every arm that shares the index instead of reporting zero.
  LEFT JOIN bench.builds b
         ON b.engine = t.engine
        AND (b.arm = t.arm OR t.arm LIKE b.arm || '-%')
 GROUP BY t.engine, t.arm, t.tenant, r.recall_at_10, rt.avg_returned, b.size_bytes, b.build_ms
 ORDER BY t.tenant, p95_ms;
