-- Wave 17 lane 13 — p17-backend-gaps (branch p17-backend-gaps, PG :54391 is
-- the branch-local test port). ADDITIVE ONLY: every pre-existing column,
-- table, index, CHECK, and policy keeps its shape; the new columns are
-- NULL-or-default-valued for existing rows and the new tables get the
-- one-policy-per-org isolation shape. U9-customization cross-check: none
-- of the constructs referenced by customization rows are touched.
--
-- This migration closes the filebacks the merged area lanes filed because
-- the CONTRACT was missing. One lane-exclusive deliverable per section:
--
-- (1) alerts — triage state: status / acknowledged / resolved fields so
--     /alerts can acknowledge + resolve (routes + events lane-side).
--     'open' is the DEFAULT so every pre-existing row reads as untriaged;
--     the ack/resolve actors + timestamps are NULL until first action.
--     Replay/dedup semantics (0037) are untouched — no UNIQUE involves
--     the new columns, a replayed ingest still answers the stored row.
--
-- (2) board_statuses.wip_limit — true per-column WIP limits. NULL = no
--     limit (pre-existing boards unchanged); 1..=1000 keep ops-sane.
--     Server-side enforcement of the limit on status moves lives in
--     core-api (tickets::change_status); this migration only adds the
--     column, so a replay of old code with the schema is still correct.
--
-- (3) documents_attachments — file attachments on documents over the
--     core-store ObjectStore seam (2.14). A SEPARATE org-scoped table
--     instead of widening the shared `attachments` surface: the original
--     table's structural CHECKs encode exactly-one-ticket-or-email
--     ownership and are legitimately load-bearing for ticket/email
--     semantics — reshaping them is a breaking edit to a shipped shape,
--     while a dedicated table with the SAME content-addressed dedup
--     doctrine (org/<org>/attachments/<sha256>, refcount reclaim) carries
--     none of that risk. No portal uploader (staff-only), no ticket
--     linkage, documents cascade.
--
-- (4) search_index.client_scope — per-client scoping for unified search:
--     the indexer predates client-scoped documents (kind `document` is
--     org-scoped only, 0031). The column mirrors documents.client_id
--     (0053: SET NULL on client delete — a deleted client's docs become
--     org-global, and the index follows). Backfilled from the live rows
--     below; the worker indexer writes it on every document upsert.
--
-- (5) search_pins — operator-curated persistent pins for /search (the
--     local recent-chips stay, untouched). Per-user, per-org; a pin is a
--     specific stored result of a query (kind + id + title + the query
--     snapshot). UNIQUE (org, user, kind, id) makes re-pinning idempotent
--     (the API upserts); the constraint name is mapped to a typed 409
--     reason in core-api's map_unique_constraint.
--
-- (6) org_exports — the export-history ledger: every org bundle created
--     through POST /api/org/exports lands a row here so /settings/export
--     can LIST past exports, ARCHIVE them, and mint one fresh signed
--     download link per row (the link TTL stays 15 minutes — rows store
--     the content facts, never an expiring URL).

-- (1)
ALTER TABLE alerts
  ADD COLUMN status text NOT NULL DEFAULT 'open'
    CHECK (status IN ('open','acknowledged','resolved')),
  ADD COLUMN acked_by uuid REFERENCES users(id) ON DELETE SET NULL,
  ADD COLUMN acked_at timestamptz,
  ADD COLUMN resolved_by uuid REFERENCES users(id) ON DELETE SET NULL,
  ADD COLUMN resolved_at timestamptz;
CREATE INDEX idx_alerts_org_status
  ON alerts (org_id, status, occurred_at DESC);

COMMENT ON COLUMN alerts.status IS
  'Triage state (0089): open | acknowledged | resolved. Transitions are forward-only through the API: acknowledged and resolved rows carry their actor + timestamp.';

-- (2)
ALTER TABLE board_statuses
  ADD COLUMN wip_limit integer
    CHECK (wip_limit BETWEEN 1 AND 1000);

COMMENT ON COLUMN board_statuses.wip_limit IS
  'Per-column WIP cap (0089). NULL = no limit. Enforced server-side at ticket status change (occupancy of the TARGET column, excluding the moving ticket).';

-- (3)
CREATE TABLE documents_attachments (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  document_id uuid NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
  filename text NOT NULL,
  content_type text NOT NULL,
  size_bytes bigint NOT NULL CHECK (size_bytes > 0),
  sha256 text NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
  -- Content-addressed per org (core-store::attachment_key), same doctrine
  -- as the shared attachments table: rows share keys, deletion reclaims
  -- the object only when no sibling row references the key.
  storage_key text NOT NULL,
  uploaded_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX idx_documents_attachments_doc
  ON documents_attachments (org_id, document_id, created_at);
CREATE INDEX idx_documents_attachments_dedupe
  ON documents_attachments (org_id, sha256);
CREATE INDEX idx_documents_attachments_key_refs
  ON documents_attachments (storage_key);

ALTER TABLE documents_attachments ENABLE ROW LEVEL SECURITY;
CREATE POLICY documents_attachments_org ON documents_attachments
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- (4)
ALTER TABLE search_index
  ADD COLUMN client_scope uuid REFERENCES clients(id) ON DELETE SET NULL;

-- Backfill the already-indexed rows from the documents they mirror. No
-- unfiltered statements (WHERE keeps the statement scoped to kind
-- `document` joined by id); a re-run against an empty join set is inert.
UPDATE search_index
SET client_scope = documents.client_id
FROM documents
WHERE search_index.entity_kind = 'document'
  AND search_index.entity_id = documents.id
  AND documents.client_id IS NOT NULL;

-- Client-context scoping filters ride this instead of a seq scan.
CREATE INDEX idx_search_client_scope
  ON search_index (client_scope)
  WHERE client_scope IS NOT NULL;

COMMENT ON COLUMN search_index.client_scope IS
  'The scope (documents.client_id) of an indexed document (0089). NULL = org-global document (or non-document kind). ON DELETE SET NULL mirrors documents.client_id: a deleted client promotes its docs to global.';

-- (5) Re-pin idempotency: one pinned result per (user, kind, id).
CREATE TABLE search_pins (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  -- The /search query that produced the result, snapshotted for honest
  -- rendering (a pin re-runs its query, it does not fake a result).
  query text NOT NULL CHECK (char_length(query) BETWEEN 2 AND 256),
  entity_kind text NOT NULL,
  entity_id uuid NOT NULL,
  parent_id uuid,
  title text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX idx_search_pins_user
  ON search_pins (org_id, user_id, created_at DESC);

-- Re-pin idempotency surface: the API routes pin through an upsert keyed
-- on this identity; the constraint name is mapped to a typed 409
-- ("search_pin_conflict") in map_unique_constraint for the concurrent
-- race, so an unknown name can never leak into a response.
CREATE UNIQUE INDEX uq_search_pins_result
  ON search_pins (org_id, user_id, entity_kind, entity_id);

ALTER TABLE search_pins ENABLE ROW LEVEL SECURITY;
CREATE POLICY search_pins_org ON search_pins
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- (6)
CREATE TABLE org_exports (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  bundle_version integer NOT NULL,
  size_bytes bigint NOT NULL CHECK (size_bytes > 0),
  sha256 text NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
  -- Where the bundle bytes live (core-store export_key): the freshly
  -- signed download link is minted from THIS row's reference at request
  -- time — the ledger never stores a link that is already expired.
  storage_key text NOT NULL,
  archived boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX idx_org_exports_org_created
  ON org_exports (org_id, created_at DESC);

ALTER TABLE org_exports ENABLE ROW LEVEL SECURITY;
CREATE POLICY org_exports_org ON org_exports
  USING (org_id = current_setting('app.org_id', true)::uuid);
