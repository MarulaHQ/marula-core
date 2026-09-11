-- Vector search leg (BUILD-PROMPT §4.5; Wave 12 lane `p12-vector-search`):
-- pgvector embedding storage fused with the existing FTS/trigram leg by
-- reciprocal rank fusion.
--
-- pgvector availability: dev and prod Postgres run the
-- `pgvector/pgvector:pg17` image (deploy/docker-compose.yml, deploy/helm), so
-- `vector` is available. This migration enables the extension explicitly so a
-- stock PG17 with the extension installed works identically, and a Postgres
-- without pgvector fails HERE with a clear "extension vector is not available"
-- rather than silently degrading the vector leg at query time.
--
-- Storage is deliberately a SIDE table, not a column on `search_index`:
--   * the FTS table's generated `tsv`, its exactly-once upsert contract, and
--     every existing reader stay byte-stable (purely additive change);
--   * embedding provenance (model + content hash) rides with the vector and
--     powers the idempotent backfill (stale hash ⇒ re-embed);
--   * vector reads carry their own RLS policy, independently testable.
--
-- Dimension 768 (`SEARCH_EMBEDDING_DIM` in core-domain::completion) is the
-- bundled bge-base-class default, nomic-embed-text's native width, and the
-- OpenAI text-embedding-3 truncated width. A provider that cannot produce 768
-- floats leaves the row unembedded (the vector leg degrades to FTS) and never
-- stores a wrong-width vector.
--
-- No ANN index in v1, on purpose: an HNSW/IVFFlat index over a cross-tenant
-- table plus an RLS filter under-returns for small tenants (the index returns
-- the global k nearest, which the policy then filters). Exact KNN over
-- `org_id = ?` is correct and fast at PSA scale; tenant-partitioned ANN is a
-- documented follow-up, not a silent recall regression.

CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE search_embeddings (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- Mirrors search_index.entity_kind (widened in 0031): vectors exist only
  -- for rows the FTS table already covers.
  entity_kind text NOT NULL CHECK (entity_kind IN ('ticket','comment','client','site','contact',
                                                   'kb_article','document')),
  entity_id uuid NOT NULL,
  -- Embedding model that produced the vector (provenance; a model change
  -- makes the backfill sweep re-embed).
  model text NOT NULL,
  -- md5(title || E'\x1f' || body) at embed time: the backfill re-embeds a row
  -- only when its searchable text has moved on.
  content_hash text NOT NULL,
  embedding vector(768) NOT NULL,
  embedded_at timestamptz NOT NULL DEFAULT now(),
  -- One vector per (org, kind, entity): replays upsert in place.
  UNIQUE (org_id, entity_kind, entity_id)
);

CREATE INDEX idx_search_embeddings_org_kind ON search_embeddings (org_id, entity_kind);

ALTER TABLE search_embeddings ENABLE ROW LEVEL SECURITY;
CREATE POLICY search_embeddings_org ON search_embeddings
  USING (org_id = current_setting('app.org_id', true)::uuid);
