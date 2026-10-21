-- Deterministic fixture for the vector ANN benchmark.
--
-- Sizes come from the driver: -v small=2000 -v large=100000 -v fillers=20
-- -v filler_rows=10000. Defaults are the documented scale points
-- (docs/perf.md §1: 50k/org primary budget, 100k/org nightly scale harness),
-- with the small tenant at 2,000 rows.
--
-- Vectors: 200 topic centres, 50 subtopics under each, and every row is its
-- subtopic centre plus jitter of norm 0.05. Topic and subtopic assignment are
-- by row number, not by tenant, so a tenant occupies no region of the space.
-- If tenants clustered, a global index would happen to return the right
-- tenant's rows and the fixture would hide the filter effect this benchmark
-- exists to measure.
--
-- The jitter has to be drawn per row. The correlated subquery that does it
-- references i, because an uncorrelated one is evaluated once and every row in
-- a branch then holds the same vector.
--
-- setseed makes the whole corpus reproducible from the same inputs.

\set ON_ERROR_STOP on

SELECT setseed(0.42) AS seeded;

CREATE TEMP TABLE centers AS
SELECT t AS topic,
       l2_normalize((SELECT array_agg(random()::real)::vector
                       FROM generate_series(1, 768))) AS vec
  FROM generate_series(0, 199) t;

-- Subtopic centres: 200 topics, 50 subtopics each. A corpus of one centre and
-- per-row noise is a single dense shell, and cosine distance around a shell
-- gives a tie at the tenth row no matter how the noise is scaled. Real
-- embeddings are hierarchical: a neighbourhood of ten sits inside a small
-- branch that is clearly separated from its sibling branches, which is what
-- makes an approximate index able to find it at all.
CREATE TEMP TABLE subtopics AS
SELECT k,
       l2_normalize(c.vec + (SELECT array_agg((random() * 0.7)::real)::vector
                                     FROM generate_series(1, 768))) AS vec
  FROM generate_series(0, 9999) k
  JOIN centers c ON c.topic = k % 200;

INSERT INTO bench.tenant_ids
SELECT 'S', md5('org-S')::uuid, :small
UNION ALL
SELECT 'L', md5('org-L')::uuid, :large;

INSERT INTO bench.tenant_ids
SELECT 'f' || g, md5('org-f' || g)::uuid, :filler_rows
  FROM generate_series(1, :fillers) g;

-- Local copy for the generation loops in this session.
CREATE TEMP TABLE tenants AS SELECT label, id, n FROM bench.tenant_ids;

INSERT INTO orgs SELECT id FROM tenants;

-- search_index: one row per entity, so the document-scope EXISTS can resolve.
INSERT INTO search_index (id, org_id, entity_kind, entity_id, title, body, client_scope, indexed_at)
SELECT md5(t.id::text || ':i:' || i)::uuid,
       t.id,
       CASE WHEN i % 100 < 60 THEN 'ticket'
            WHEN i % 100 < 90 THEN 'comment'
            WHEN i % 100 < 96 THEN 'client'
            WHEN i % 100 < 99 THEN 'contact'
            ELSE 'document' END,
       md5(t.id::text || ':e:' || i)::uuid,
       'title ' || i,
       'body ' || i,
       CASE WHEN i % 97 = 0 THEN md5('client-' || (i % 7))::uuid END,
       now() - (i || ' minutes')::interval
  FROM tenants t
  CROSS JOIN LATERAL generate_series(1, t.n) i;

-- search_embeddings: identical key set plus the 768-dim vector.
--
-- Each row is its subtopic centre plus a jitter vector of norm 0.05, so the
-- ten nearest rows of a query are its subtopic siblings at about 0.07 while
-- the next branch sits at about 0.5. That gap is what an approximate index
-- needs, and without it a recall number describes the fixture rather than the
-- index.
--
-- Three earlier versions of this file produced corpora where the true nearest
-- ten was an arbitrary member of a tie: a finite noise pool that left 102,400
-- distinct vectors over 302,000 rows, uniform per-row noise at a scale wider
-- than the gaps between neighbours, and a radial scale that cosine distance
-- discards because it ignores magnitude. Each one made recall read 0.09 to
-- 0.25 for reasons that were properties of the fixture.
INSERT INTO search_embeddings (id, org_id, entity_kind, entity_id, model, content_hash, embedding)
SELECT md5(t.id::text || ':i:' || i)::uuid,
       t.id,
       CASE WHEN i % 100 < 60 THEN 'ticket'
            WHEN i % 100 < 90 THEN 'comment'
            WHEN i % 100 < 96 THEN 'client'
            WHEN i % 100 < 99 THEN 'contact'
            ELSE 'document' END,
       md5(t.id::text || ':e:' || i)::uuid,
       'bench',
       md5('hash' || i),
       sb.vec + (SELECT array_agg((random() * 0.0018)::real)::vector
                   FROM generate_series(1, 768) s
                  WHERE i > 0)
  FROM tenants t
  CROSS JOIN LATERAL generate_series(1, t.n) i
  JOIN subtopics sb ON sb.k = (i * 6151) % 10000;

ANALYZE search_index;
ANALYZE search_embeddings;

SELECT t.label,
       count(*) AS rows,
       round(100.0 * count(*) / (SELECT count(*) FROM search_embeddings), 2) AS pct_of_table
  FROM bench.tenant_ids t
  JOIN search_embeddings e ON e.org_id = t.id
 WHERE t.label IN ('S', 'L')
 GROUP BY t.label
 ORDER BY t.label;

-- Uniqueness check on a sample: a tie between two rows turns a recall
-- measurement into a coin flip, so the fixture proves there are none. The
-- full-table DISTINCT over 302k 768-dim vectors sorts about 900MB and killed
-- the container; a sample is enough to catch a degenerate generator.
SELECT count(*) AS sampled_rows,
       count(DISTINCT md5(embedding::text)) AS distinct_in_sample
  FROM (SELECT embedding FROM search_embeddings LIMIT 50000) x;
