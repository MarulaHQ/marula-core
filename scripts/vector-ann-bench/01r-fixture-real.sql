-- Fixture from real EmbeddingGemma 2 vectors.
--
-- Same schema and same RLS as the synthetic fixture; only the vectors differ,
-- and they are the ones the product's default model produced for text the
-- product would plausibly store. Tenant assignment comes from the corpus
-- builder, which assigns at random so a tenant occupies no region of the
-- embedding space.
--
-- Sizes are whatever the corpus holds: this run exists to measure recall on a
-- real distribution, not to reproduce a scale point. The 302,000-row synthetic
-- run keeps that job. The two are reported separately and never averaged.
--
-- The driver copies vectors.tsv into the container at /tmp/p52-vectors.tsv.

\set ON_ERROR_STOP on

CREATE TEMP TABLE staged (
  tenant text NOT NULL,
  kind text NOT NULL,
  entity_id uuid NOT NULL,
  embedding vector(768) NOT NULL
);

\copy staged FROM '/tmp/p52-vectors.tsv'

INSERT INTO bench.tenant_ids
SELECT tenant, md5('org-' || tenant)::uuid, count(*)::int
  FROM staged GROUP BY tenant;

INSERT INTO orgs SELECT id FROM bench.tenant_ids;

INSERT INTO search_index (id, org_id, entity_kind, entity_id, title, body, indexed_at)
SELECT md5(e.entity_id::text)::uuid,
       t.id,
       e.kind,
       e.entity_id,
       '',
       '',
       now()
  FROM staged e
  JOIN bench.tenant_ids t ON t.label = e.tenant;

INSERT INTO search_embeddings (id, org_id, entity_kind, entity_id, model, content_hash, embedding)
SELECT md5(e.entity_id::text)::uuid,
       t.id,
       e.kind,
       e.entity_id,
       'embeddinggemma-2-768',
       md5(e.entity_id::text),
       e.embedding
  FROM staged e
  JOIN bench.tenant_ids t ON t.label = e.tenant;

ANALYZE search_index;
ANALYZE search_embeddings;

SELECT t.label, t.n AS rows,
       round(100.0 * t.n / (SELECT count(*) FROM search_embeddings), 2) AS pct_of_table
  FROM bench.tenant_ids t
 WHERE t.label IN ('S', 'L')
 ORDER BY t.label;

SELECT count(*) AS total_embeddings,
       count(DISTINCT org_id) AS tenants
  FROM search_embeddings;
