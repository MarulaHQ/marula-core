-- The retrieval mode's database side (Wave 52 lane R15,
-- `p52-retrieval-mode-config`).
--
-- Why this migration is not "create an index": the mode is a deployment
-- decision, and the two shapes it serves want opposite things. A box running
-- hundreds of small tenants wants the exact scan, which is 0.48 ms at recall
-- 1.000 and keeps no index in memory. A per-org deployment whose tenant passed
-- ~20,000 embeddings wants an index, because the exact scan no longer fits the
-- vector leg. Migration 0062 explains why v1 shipped exact only; this migration
-- is where that changes, without changing it for the installs that do not need
-- it.
--
-- So this file installs two functions and builds nothing by itself:
--
--   1. `search_embedding_distribution()` — per-tenant embedding counts for the
--      operator report and the `marula.embedding.count` gauge. It is
--      `SECURITY DEFINER` because `search_embeddings` carries RLS on `org_id`
--      and the app role can only count its own tenant. The output is a count,
--      never content.
--   2. `marula_ensure_retrieval_index(target)` — creates the index the selected
--      mode needs, and only that one. It is called by the `migrate` step, which
--      already runs as the schema owner with `MARULA_RETRIEVAL_MODE` in its
--      environment, so the index exists because a deployment asked for it
--      rather than because everybody migrated.
--
-- A database without the VectorChord extension applies this file cleanly and
-- gets `skipped: ...` from the ensure function; that is the stock
-- `pgvector/pgvector:pg18` path, and it is the path the exact mode keeps.
--
-- The exact-mode statement in `crates/core-db/src/search.rs` turns index scans
-- off for its own statement (`SET LOCAL enable_indexscan = off`, restored
-- afterwards so page hydration keeps its index), so an index that exists cannot
-- be chosen by a deployment that set `MARULA_RETRIEVAL_MODE=exact`. That is
-- stronger than a cost nudge and it survives a planner change: with index scans
-- off, no ordering operator on the index can be used at all.

-- ---------------------------------------------------------------------------
-- 1. The distribution: one row per tenant, for the operator and the gauge
-- ---------------------------------------------------------------------------

CREATE OR REPLACE FUNCTION search_embedding_distribution()
RETURNS TABLE (org_id uuid, embeddings bigint)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
    SELECT e.org_id, count(*)::bigint AS embeddings
      FROM search_embeddings e
     GROUP BY e.org_id
$$;

COMMENT ON FUNCTION search_embedding_distribution() IS
  'Rows in search_embeddings per tenant. SECURITY DEFINER so one gauge can see every tenant past RLS; counts only, never content.';

-- RLS-scoped roles may read the distribution; they may not redefine the
-- function.
REVOKE ALL ON FUNCTION search_embedding_distribution() FROM PUBLIC;
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'marula_app') THEN
    GRANT EXECUTE ON FUNCTION search_embedding_distribution() TO marula_app;
  END IF;
END
$$;

-- ---------------------------------------------------------------------------
-- 2. The index, built only when the deployment selected a mode that needs one
-- ---------------------------------------------------------------------------

CREATE OR REPLACE FUNCTION marula_ensure_retrieval_index(target text)
RETURNS text
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    preload text := coalesce(current_setting('shared_preload_libraries', true), '');
    vector_version text;
BEGIN
    CASE lower(coalesce(btrim(target), 'exact'))
    WHEN 'exact' THEN
        -- The default. Keeping it index-free is the point: the index costs
        -- build time and memory a small-tenant box should not pay.
        RETURN 'skipped: exact needs no index';

    WHEN 'iterative' THEN
        SELECT extversion INTO vector_version
          FROM pg_extension WHERE extname = 'vector';
        IF vector_version IS NULL THEN
            RETURN 'skipped: pgvector is not installed';
        END IF;
        -- 0.8 is the release that introduced `hnsw.iterative_scan`. Compare
        -- numerically, not as strings: 0.10 is newer than 0.8.
        IF string_to_array(split_part(vector_version, '-', 1), '.')::int[] <
           ARRAY[0, 8]::int[] THEN
            RETURN format('skipped: pgvector %s has no iterative scan', vector_version);
        END IF;
        CREATE INDEX IF NOT EXISTS search_embeddings_hnsw
            ON search_embeddings USING hnsw (embedding vector_cosine_ops);
        RETURN 'present';

    WHEN 'index' THEN
        IF NOT EXISTS (SELECT 1 FROM pg_available_extensions WHERE name = 'vchord')
           AND NOT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'vchord') THEN
            RETURN 'skipped: the vchord extension is not available on this image';
        END IF;
        IF position('vchord' in preload) = 0 THEN
            -- Installed-but-not-preloaded is the managed-instance case: there is
            -- nothing to build, and the boot will refuse `index` with the
            -- preload named. Say so here rather than failing the migration.
            RETURN 'skipped: vchord is not in shared_preload_libraries';
        END IF;
        CREATE EXTENSION IF NOT EXISTS vchord CASCADE;
        CREATE INDEX IF NOT EXISTS search_embeddings_vchord
            ON search_embeddings USING vchordrq (embedding vector_cosine_ops);
        RETURN 'present';

    ELSE
        RETURN format('skipped: unknown target %s', coalesce(target, '(null)'));
    END CASE;
END;
$$;

COMMENT ON FUNCTION marula_ensure_retrieval_index(text) IS
  'Builds the ANN index MARULA_RETRIEVAL_MODE names, or returns skipped: <reason>. Called by the migrate step as the schema owner; never by a request path.';

-- Only the schema owner may run DDL through this function. The migrate step is
-- the owner; a request-serving role is not, and granting this to one would make
-- every request a potential CREATE INDEX.
REVOKE ALL ON FUNCTION marula_ensure_retrieval_index(text) FROM PUBLIC;
