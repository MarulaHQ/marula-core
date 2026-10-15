-- RAG and media vector namespace + provable deletion receipts (TASKLIST Wave 45
-- lane 1, `p45-rag-vector-namespace`; decision of record: ADR 0020).
--
-- Head on main at claim time: `0131_org_usage.sql`.
--
-- Purpose:
--   1. `rag_embeddings` — the tenant-scoped, modality-aware second vector
--      namespace beside `search_embeddings` (migration 0062). Keyed by
--      `(org_id, namespace, source_kind, source_id, chunk_index, modality)`
--      so passage-level chunks and multimodal media assets (`text | image |
--      video | audio`) can be stored, re-embedded idempotently on model or
--      content changes, and queried via hybrid BM25 (`tsv`) + exact-KNN
--      cosine distance (`embedding <=> $vec`) with Reciprocal Rank Fusion
--      (RRF) and mandatory source attribution.
--   2. Fixed width `vector(768)` (`SEARCH_EMBEDDING_DIM = 768` in
--      `core_domain::completion`), shared across text and media modalities
--      within a namespace so text->media and media->text retrieval share one
--      vector space without schema churn.
--   3. Exact KNN in v1 (no HNSW/IVFFlat ANN index, per ADR 0020): cross-tenant
--      ANN indexes filtered post-hoc by RLS under-return for smaller tenants;
--      exact KNN scoped by `(org_id, namespace)` is exact under RLS.
--   4. `rag_deletion_receipts` — append-only, immutable deletion receipts
--      recording `(receipt_id, org_id, namespace, source_kind, source_id,
--      deleted_chunks, content_hashes, deleted_at)` whenever a source's chunks
--      are removed (via the published DELETE surface or via automatic cascade
--      triggers when underlying core entities/attachments are deleted), so
--      DSAR/erasure and source removals are provable rather than asserted.
--
-- Non-purpose (explicitly out of scope per ADR 0020 §5):
--   Core never decodes images, samples video frames, resamples audio, runs
--   transcription, or ships native model weights. Media vectors arrive
--   precomputed at width 768 from the caller.

CREATE EXTENSION IF NOT EXISTS vector;

-- ============================================================================
-- (A) `rag_embeddings` — passage and multimodal chunk store
-- ============================================================================

CREATE TABLE rag_embeddings (
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  namespace text NOT NULL CHECK (length(btrim(namespace)) > 0 AND length(namespace) <= 128),
  source_kind text NOT NULL CHECK (length(btrim(source_kind)) > 0 AND length(source_kind) <= 128),
  source_id text NOT NULL CHECK (length(btrim(source_id)) > 0 AND length(source_id) <= 256),
  chunk_index integer NOT NULL CHECK (chunk_index >= 0),
  modality text NOT NULL CHECK (modality IN ('text', 'image', 'video', 'audio')),
  -- Optional passage text or media caption/transcript snippet powering the
  -- lexical BM25/FTS leg of hybrid retrieval. Nullable for pure-vector media
  -- chunks that carry no text representation.
  content_text text,
  tsv tsvector GENERATED ALWAYS AS (
    to_tsvector('english', coalesce(content_text, ''))
  ) STORED,
  -- Attribution metadata (title, uri/locator, span/page/timecode, mime_type,
  -- etc.) returned verbatim with every query hit. Must be a JSON object.
  metadata jsonb NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(metadata) = 'object'),
  -- Provenance: embedding model identifier + content hash. Re-ingesting the
  -- same key with identical (model, content_hash) is a no-op; a model upgrade
  -- or content change overwrites the vector and provenance in place.
  model text NOT NULL CHECK (length(btrim(model)) > 0 AND length(model) <= 256),
  content_hash text NOT NULL CHECK (length(btrim(content_hash)) > 0 AND length(content_hash) <= 256),
  embedding vector(768) NOT NULL,
  embedded_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (org_id, namespace, source_kind, source_id, chunk_index, modality)
);

-- Serves source-level lookup and deletion cascades across all namespaces or
-- within a specific namespace.
CREATE INDEX idx_rag_embeddings_org_source
  ON rag_embeddings (org_id, source_kind, source_id);

-- Serves modality-filtered exact-KNN scans within an org + namespace.
CREATE INDEX idx_rag_embeddings_org_ns_modality
  ON rag_embeddings (org_id, namespace, modality);

-- Serves the lexical BM25 / websearch_to_tsquery leg of hybrid retrieval.
CREATE INDEX idx_rag_embeddings_tsv
  ON rag_embeddings USING gin (tsv);

ALTER TABLE rag_embeddings ENABLE ROW LEVEL SECURITY;
ALTER TABLE rag_embeddings FORCE ROW LEVEL SECURITY;
CREATE POLICY rag_embeddings_org ON rag_embeddings
  USING (org_id = NULLIF(current_setting('app.org_id', true), '')::uuid)
  WITH CHECK (org_id = NULLIF(current_setting('app.org_id', true), '')::uuid);

-- ============================================================================
-- (B) `rag_deletion_receipts` — provable source & DSAR erasure ledger
-- ============================================================================

CREATE TABLE rag_deletion_receipts (
  receipt_id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  namespace text NOT NULL CHECK (length(btrim(namespace)) > 0 AND length(namespace) <= 128),
  source_kind text NOT NULL CHECK (length(btrim(source_kind)) > 0 AND length(source_kind) <= 128),
  source_id text NOT NULL CHECK (length(btrim(source_id)) > 0 AND length(source_id) <= 256),
  deleted_chunks integer NOT NULL CHECK (deleted_chunks >= 0),
  content_hashes text[] NOT NULL DEFAULT '{}'::text[],
  deleted_by uuid REFERENCES users(id) ON DELETE SET NULL,
  deleted_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX idx_rag_deletion_receipts_org_source
  ON rag_deletion_receipts (org_id, namespace, source_kind, source_id, deleted_at DESC);

CREATE INDEX idx_rag_deletion_receipts_org_time
  ON rag_deletion_receipts (org_id, deleted_at DESC, receipt_id DESC);

ALTER TABLE rag_deletion_receipts ENABLE ROW LEVEL SECURITY;
ALTER TABLE rag_deletion_receipts FORCE ROW LEVEL SECURITY;
CREATE POLICY rag_deletion_receipts_org ON rag_deletion_receipts
  USING (org_id = NULLIF(current_setting('app.org_id', true), '')::uuid)
  WITH CHECK (org_id = NULLIF(current_setting('app.org_id', true), '')::uuid);

-- Append-only guard on deletion receipts: once minted, a receipt's evidence
-- cannot be altered or deleted except during an org cascade or dev reset.
--
-- SECURITY DEFINER with a pinned search_path (the same posture as the
-- auth-resolver seams): the guard has to answer a question about the PARENT
-- org, and under row-level security an unset scope would hide `orgs` and make
-- a manual delete look like a cascade. Running as the definer reads the real
-- org state, so a receipt is writable only when its org is actually going away
-- or the dev-reset bypass is set. The check touches no cross-org data.
CREATE FUNCTION rag_deletion_receipts_append_only() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public AS $$
BEGIN
  IF current_setting('app.dev_reset_bypass', true) = 'on' THEN
    RETURN COALESCE(NEW, OLD);
  END IF;
  IF TG_OP = 'DELETE' THEN
    -- Allow ON DELETE CASCADE when the parent org itself is being removed.
    IF NOT EXISTS (SELECT 1 FROM orgs WHERE id = OLD.org_id) THEN
      RETURN OLD;
    END IF;
  END IF;
  RAISE EXCEPTION 'rag_deletion_receipts is append-only: % rejected (receipt %)', TG_OP, OLD.receipt_id;
END;
$$;

CREATE TRIGGER trg_rag_deletion_receipts_append_only
  BEFORE UPDATE OR DELETE ON rag_deletion_receipts
  FOR EACH ROW EXECUTE FUNCTION rag_deletion_receipts_append_only();

-- ============================================================================
-- (C) Automatic cascade cleanup when core entities / attachments are deleted
-- ============================================================================
--
-- `rag_embeddings.source_id` is polymorphic text (covering UUIDs of core
-- records as well as external connector locators), so a single table FK cannot
-- express cascades from every core table. Instead, an AFTER DELETE trigger on
-- each core source table removes any matching `(org_id, source_kind,
-- source_id)` chunks across every namespace and mints one `rag_deletion_receipts`
-- row per affected namespace in the same transaction.
--
-- SECURITY DEFINER with a pinned search_path, for the same reason as the
-- append-only guard: a maintenance delete (a system cleanup or a dev reset)
-- may run without an `app.org_id` scope, and under row-level security an
-- invoker-rights trigger would either delete nothing or abort the parent
-- delete when it tried to write the receipt. The function only ever touches
-- rows whose `org_id` equals the deleted row's `org_id`, so it cannot cross a
-- tenant boundary.

CREATE FUNCTION rag_cascade_source_delete() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public AS $$
DECLARE
  v_source_kind text := TG_ARGV[0];
  v_source_id text := OLD.id::text;
BEGIN
  -- If the parent org itself is being dropped, `rag_embeddings` and
  -- `rag_deletion_receipts` cascade via `org_id REFERENCES orgs(id) ON DELETE CASCADE`.
  IF NOT EXISTS (SELECT 1 FROM orgs WHERE id = OLD.org_id) THEN
    RETURN OLD;
  END IF;

  WITH removed AS (
    DELETE FROM rag_embeddings
     WHERE org_id = OLD.org_id
       AND source_kind = v_source_kind
       AND source_id = v_source_id
    RETURNING org_id, namespace, source_kind, source_id, chunk_index, modality, content_hash
  ),
  grouped AS (
    SELECT
      org_id,
      namespace,
      source_kind,
      source_id,
      count(*)::integer AS deleted_chunks,
      array_agg(content_hash ORDER BY chunk_index, modality) AS content_hashes
    FROM removed
    GROUP BY org_id, namespace, source_kind, source_id
  )
  INSERT INTO rag_deletion_receipts (
    receipt_id,
    org_id,
    namespace,
    source_kind,
    source_id,
    deleted_chunks,
    content_hashes,
    deleted_by
  )
  SELECT
    gen_random_uuid(),
    g.org_id,
    g.namespace,
    g.source_kind,
    g.source_id,
    g.deleted_chunks,
    g.content_hashes,
    NULLIF(current_setting('app.user_id', true), '')::uuid
  FROM grouped g;

  RETURN OLD;
END;
$$;

CREATE TRIGGER trg_rag_cascade_attachments
  AFTER DELETE ON attachments
  FOR EACH ROW EXECUTE FUNCTION rag_cascade_source_delete('attachment');

CREATE TRIGGER trg_rag_cascade_documents_attachments
  AFTER DELETE ON documents_attachments
  FOR EACH ROW EXECUTE FUNCTION rag_cascade_source_delete('document_attachment');

CREATE TRIGGER trg_rag_cascade_tickets
  AFTER DELETE ON tickets
  FOR EACH ROW EXECUTE FUNCTION rag_cascade_source_delete('ticket');

CREATE TRIGGER trg_rag_cascade_ticket_comments
  AFTER DELETE ON ticket_comments
  FOR EACH ROW EXECUTE FUNCTION rag_cascade_source_delete('comment');

CREATE TRIGGER trg_rag_cascade_kb_articles
  AFTER DELETE ON kb_articles
  FOR EACH ROW EXECUTE FUNCTION rag_cascade_source_delete('kb_article');

CREATE TRIGGER trg_rag_cascade_documents
  AFTER DELETE ON documents
  FOR EACH ROW EXECUTE FUNCTION rag_cascade_source_delete('document');

CREATE TRIGGER trg_rag_cascade_clients
  AFTER DELETE ON clients
  FOR EACH ROW EXECUTE FUNCTION rag_cascade_source_delete('client');

-- Platform event types for RAG ingest and deletion receipts.
INSERT INTO event_types (event_type, source, label, description) VALUES
  ('rag.chunks_ingested', 'platform', 'RAG chunks ingested', 'A batch of RAG or media vector chunks was upserted into a namespace'),
  ('rag.source_deleted',  'platform', 'RAG source deleted',  'RAG vector chunks for a source were deleted and a deletion receipt was minted')
ON CONFLICT (event_type) DO NOTHING;

-- The two SECURITY DEFINER trigger seams run on behalf of the constrained
-- `marula_app` role during every in-app delete, so the app role needs EXECUTE
-- (PUBLIC already holds it by default; this makes the surface explicit and
-- survives a deployment that revokes the default).
GRANT EXECUTE ON FUNCTION rag_cascade_source_delete() TO marula_app;
GRANT EXECUTE ON FUNCTION rag_deletion_receipts_append_only() TO marula_app;
