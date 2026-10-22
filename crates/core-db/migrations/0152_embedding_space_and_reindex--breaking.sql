-- The embedding space tag, the instance selection state, and the re-index runs
-- (Wave 52 lane R14, `p52-embedding-model-catalogue`).
--
-- Why this exists: the embedding model used to be a property of an org's
-- completion provider, so two orgs on one install could write vectors from two
-- different models into one table. Nothing recorded which model a vector came
-- from in a form retrieval could filter on, and the vector leg compared every
-- stored vector against the query vector regardless. R13 named the model and
-- made the local path the default, which also means an org that had run
-- `ollama` without naming a model has `nomic-embed-text` vectors that now
-- resolve to EmbeddingGemma 2. That population is already a mixed space.
--
-- Three facts this migration adds:
--
--   1. `search_embeddings.space` and `rag_embeddings.space` — the tag every
--      stored vector carries: `model@version/dimensions`. Retrieval filters on
--      it, so two spaces are never ranked against one query vector. Existing
--      rows get `@unversioned/`, which is the honest answer: the rows were
--      written before the build was recorded, and a made-up version would make
--      two different builds of one id look interchangeable.
--   2. `embedding_space_state` — one row: what space this instance writes, and
--      which space is still readable while a re-index runs.
--   3. `embedding_reindex_runs` — one row per org per switch, with the counts a
--      progress gauge and a support conversation both need.
--
-- Non-purpose: no ANN index (that is R15's decision, and 0062/0134 explain why
-- v1 is exact KNN), no re-embedding here (a migration cannot call a model, so
-- the job does the work), and no org-level model setting: the model is chosen
-- once per instance on purpose, and a per-org setting is the hazard this
-- migration exists to remove.
--
-- The unique key on `search_embeddings` widens to include `space` so the target
-- space and the previous space can both hold a vector for one entity while the
-- re-index runs. That pair is what makes dual-write / dual-read possible without
-- ever comparing two spaces' distances.

-- ---------------------------------------------------------------------------
-- 1. The space tag on the search namespace
-- ---------------------------------------------------------------------------

ALTER TABLE search_embeddings ADD COLUMN IF NOT EXISTS space text;

-- Pre-tag rows: model known, build unrecorded, width read from the stored
-- vector rather than assumed. `nomic-embed-text@unversioned/768` is exactly what
-- this produces for the population R13's default change left behind, which is
-- the point: the tag shows the mixture instead of hiding it.
UPDATE search_embeddings
   SET space = model || '@unversioned/' || vector_dims(embedding)::text
 WHERE space IS NULL;

ALTER TABLE search_embeddings ALTER COLUMN space SET NOT NULL;
ALTER TABLE search_embeddings ALTER COLUMN space SET DEFAULT '';

-- One vector per entity per space. 0062's key, dropped by name (Postgres names
-- an inline UNIQUE after the table and its columns).
--
-- Data impact, stated for the `--breaking` flag: this drops a constraint and
-- adds a wider one. No row is deleted or rewritten by the drop itself, and the
-- rows it used to reject are now legal — an entity may hold one vector per
-- space while a re-index runs, which is what makes dual-write possible. The
-- only rows at risk are ones that already violate the old three-column key,
-- and there are none, because the constraint enforced it. Existing rows get
-- `@unversioned/` above, so they form their own space rather than being read
-- as the new model's.
ALTER TABLE search_embeddings
  DROP CONSTRAINT IF EXISTS search_embeddings_org_id_entity_kind_entity_id_key;
ALTER TABLE search_embeddings
  ADD CONSTRAINT search_embeddings_org_kind_entity_space_key
  UNIQUE (org_id, entity_kind, entity_id, space);

-- The retrieval filter: org, kind and space together, which is the predicate
-- `vector_candidates` now carries.
CREATE INDEX IF NOT EXISTS idx_search_embeddings_org_kind_space
  ON search_embeddings (org_id, entity_kind, space);

-- ---------------------------------------------------------------------------
-- 2. The space tag on the RAG namespace
-- ---------------------------------------------------------------------------

-- `rag_embeddings` keeps its 0134 key: a chunk is upserted in place, and a model
-- change there already re-embeds through `(model, content_hash)` idempotency.
-- The tag adds the filter and makes the model visible to retrieval; the
-- re-index job does not own these rows, because a media vector arrives
-- precomputed from the caller that produced it.
ALTER TABLE rag_embeddings ADD COLUMN IF NOT EXISTS space text;

UPDATE rag_embeddings
   SET space = model || '@unversioned/' || vector_dims(embedding)::text
 WHERE space IS NULL;

ALTER TABLE rag_embeddings ALTER COLUMN space SET NOT NULL;
ALTER TABLE rag_embeddings ALTER COLUMN space SET DEFAULT '';

CREATE INDEX IF NOT EXISTS idx_rag_embeddings_org_namespace_space
  ON rag_embeddings (org_id, namespace, space);

-- ---------------------------------------------------------------------------
-- 3. Which space this instance writes, and which one is still readable
-- ---------------------------------------------------------------------------

-- One row, `id = 'instance'`: the embedding model is an instance choice, so its
-- state is instance-scoped. `target_space` is NULL until a process with the
-- catalogue resolves it, which is what makes the first boot after this
-- migration record the space rather than assume one.
--
-- `previous_space` is non-NULL only while a re-index is unfinished AND the old
-- population is safe to read beside the new one (same model id, unrecorded
-- build). A different model id leaves it NULL: the vector leg then reads the
-- target space only, so an un-reindexed row is a miss with a gauge instead of a
-- plausible ranking made of two incomparable spaces.
CREATE TABLE IF NOT EXISTS embedding_space_state (
  id text PRIMARY KEY CHECK (id = 'instance'),
  target_space text,
  previous_space text,
  -- Groups the per-org runs one switch created. A fresh uuid per change, so the
  -- worker's enqueue is idempotent across restarts and a retried sweep cannot
  -- double-queue an org.
  switch_id uuid,
  switched_at timestamptz,
  updated_at timestamptz NOT NULL DEFAULT now(),
  CHECK (previous_space IS NULL OR previous_space <> target_space)
);

INSERT INTO embedding_space_state (id) VALUES ('instance')
ON CONFLICT (id) DO NOTHING;

-- ---------------------------------------------------------------------------
-- 4. One re-index run per org per switch
-- ---------------------------------------------------------------------------

-- `total_rows` and `done_rows` are what the progress gauge reports. They are
-- snapshots, not a work queue: the job's selection predicate is "no fresh
-- vector in the target space", so a crash mid-batch repeats at most one batch
-- and cannot skip a row. `done_rows` therefore only ever serves reporting.
CREATE TABLE IF NOT EXISTS embedding_reindex_runs (
  id uuid PRIMARY KEY,
  -- Groups the per-org runs one switch created, so an operator sees the whole
  -- switch as one operation and the audit trail can name it.
  switch_id uuid NOT NULL,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- NULL when the switch has no single previous space (a mixed population, or
  -- the first boot after the tag arrived).
  from_space text,
  to_space text NOT NULL,
  status text NOT NULL DEFAULT 'queued'
    CHECK (status IN ('queued', 'running', 'complete', 'failed')),
  total_rows bigint NOT NULL DEFAULT 0 CHECK (total_rows >= 0),
  done_rows bigint NOT NULL DEFAULT 0 CHECK (done_rows >= 0),
  -- A claim, so two housekeeping workers cannot drive one org's run twice.
  leased_until timestamptz,
  last_error text,
  started_at timestamptz,
  finished_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (switch_id, org_id)
);

-- The sweep's scan: unfinished runs only, oldest first. Partial because a
-- completed run is never selected again and the settled majority must not be
-- walked on every pass.
CREATE INDEX IF NOT EXISTS idx_embedding_reindex_due
  ON embedding_reindex_runs (created_at)
  WHERE status IN ('queued', 'running');

CREATE INDEX IF NOT EXISTS idx_embedding_reindex_org
  ON embedding_reindex_runs (org_id, created_at DESC);

ALTER TABLE embedding_reindex_runs ENABLE ROW LEVEL SECURITY;
CREATE POLICY embedding_reindex_runs_org ON embedding_reindex_runs
  USING (org_id = current_setting('app.org_id', true)::uuid);
