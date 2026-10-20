-- Unified search v1 (the private build tracker 2.16): one per-entity index table backing
-- GET /api/search. Rows are written ONLY by core-api's internal effect
-- surface (`/internal/search/upsert|delete`) on behalf of the worker's
-- event-driven indexer; nothing else may write it.
--
-- The vector/pgvector leg is Stage 3: `tsv` + the trigram column below are
-- the BM25-ish/websearch leg; a future migration adds an embedding column
-- and RRF fusion without touching this contract.

CREATE EXTENSION IF NOT EXISTS pg_trgm;

CREATE TABLE search_index (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- Polymorphic target: no FK possible; integrity is owned by the indexer.
  entity_kind text NOT NULL CHECK (entity_kind IN ('ticket','comment','client','site','contact')),
  entity_id uuid NOT NULL,
  -- comment → its ticket; NULL for other kinds.
  parent_id uuid,
  title text NOT NULL DEFAULT '',
  body text NOT NULL DEFAULT '',
  tsv tsvector GENERATED ALWAYS AS (
    setweight(to_tsvector('english', coalesce(title, '')), 'A') ||
    setweight(to_tsvector('english', coalesce(body, '')), 'B')
  ) STORED,
  indexed_at timestamptz NOT NULL DEFAULT now(),
  -- Exactly-once semantics: replaying an indexing event re-upserts one row.
  UNIQUE (org_id, entity_kind, entity_id)
);

CREATE INDEX idx_search_fts ON search_index USING gin (tsv);
-- Partial-token/prefix fallback leg ("net" finds "network") via ILIKE.
CREATE INDEX idx_search_trgm ON search_index USING gin ((title || ' ' || body) gin_trgm_ops);
CREATE INDEX idx_search_kind ON search_index (org_id, entity_kind);

ALTER TABLE search_index ENABLE ROW LEVEL SECURITY;
CREATE POLICY search_index_org ON search_index
  USING (org_id = current_setting('app.org_id', true)::uuid);
