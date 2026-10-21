-- The plan each arm actually took, so nobody has to trust that the index was
-- used. COSTS OFF, ANALYZE and BUFFERS on: the shape, the rows touched and the
-- buffers read are the evidence, not the planner's estimate.
--
-- The query vector is a 768-number literal in the plan text. It is elided
-- here, because it is the same vector the fixture stored and it buries the
-- plan shape that this file exists to show.

\pset format unaligned
\pset tuples_only on

SELECT '=== ' || engine || ' / ' || arm || ' / tenant ' || tenant || ' ==='
       || E'\n'
       || regexp_replace(plan, '\[[-0-9.,eE ]{200,}\]', '[768-dim vector elided]', 'g')
  FROM bench.plans
 ORDER BY engine, arm, tenant;
