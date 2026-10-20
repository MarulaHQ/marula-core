-- Migration connector API (the private build tracker D3, Wave 9 lane 6).
--
-- Purpose: the checkpoint substrate for third-party bulk pushes that ride
-- the D2 import engine. The engine already owns idempotency of DATA (the
-- `external_ids` mapping makes a re-pushed row resolve to the same internal
-- id); what it does not own is POSITION — a multi-chunk push that dies
-- mid-run needs to know where it stopped. These two tables are that memory:
--
-- `migration_push_sessions` — one connector session over exactly one D2
-- import batch (UNIQUE batch_id: the pairing is 1:1 by construction). The
-- batch keeps its own lifecycle/counters/provenance (actor_kind 'import');
-- the session adds the connector-side view: who drove it (no FK on
-- created_by, same doctrine as import_batches — api-key ids are not users
-- rows), and when the run closed.
--
-- `migration_push_chunks` — one row per COMMITTED chunk: its caller-chosen
-- index, the content fingerprint (sha256 over the decoded row payload) and
-- the full per-row report JSON. The checkpoint cursor IS this table:
-- next_chunk = MAX(chunk_index) + 1. An exact re-send (same index, same
-- fingerprint) replays the stored report without touching data — a retry
-- after a lost response is free; same index with a different fingerprint is
-- a typed 409 (idempotency contract, not a dedupe hint). UNIQUE
-- (session_id, chunk_index) makes concurrent double-commits a constraint
-- error rather than a silent double-apply.
--
-- Non-purpose: no row payload storage (chunks are executed synchronously
-- and their source bytes are never persisted — only the report), no async
-- queueing here (that is D2's staged/worker flow; connector sessions are
-- sync-only), no pull-side state (exports are cursor-paged reads over the
-- versioned data, stateless by design).

CREATE TABLE migration_push_sessions (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- The D2 import batch this session drives; 1:1 by construction.
  batch_id uuid NOT NULL UNIQUE REFERENCES import_batches(id) ON DELETE CASCADE,
  label text NOT NULL DEFAULT 'unnamed',
  -- Free-form note about the third-party source system.
  source_note text,
  -- WHO drove the session; deliberately NO FK (api-key principals are not
  -- users rows — same provenance doctrine as import_batches.created_by).
  created_by uuid NOT NULL,
  created_by_kind text NOT NULL CHECK (created_by_kind IN ('user','api_key')),
  created_at timestamptz NOT NULL DEFAULT now(),
  finished_at timestamptz
);
CREATE INDEX idx_migration_push_sessions_org ON migration_push_sessions (org_id, created_at DESC);

CREATE TABLE migration_push_chunks (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  session_id uuid NOT NULL REFERENCES migration_push_sessions(id) ON DELETE CASCADE,
  -- Caller-chosen, gap-checked position of this chunk in the run.
  chunk_index integer NOT NULL CHECK (chunk_index >= 0),
  -- sha256 over the canonical decoded row payload + template context.
  fingerprint text NOT NULL,
  -- The full per-row RunReport of the committed chunk (replay substrate).
  summary jsonb NOT NULL CHECK (jsonb_typeof(summary) = 'object'),
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (session_id, chunk_index)
);
CREATE INDEX idx_migration_push_chunks_session ON migration_push_chunks (session_id, chunk_index);

ALTER TABLE migration_push_sessions ENABLE ROW LEVEL SECURITY;
CREATE POLICY migration_push_sessions_org ON migration_push_sessions
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE migration_push_chunks ENABLE ROW LEVEL SECURITY;
CREATE POLICY migration_push_chunks_org ON migration_push_chunks
  USING (org_id = current_setting('app.org_id', true)::uuid);
