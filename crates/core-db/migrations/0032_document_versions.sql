-- DocumentVersion entities (the private build tracker D4, docs/10-agent-platform.md §6).
--
-- Purpose: documents with immutable version history. Two org-scoped tables:
--
--   * documents — the living handle: current title, and `current_version`
--     pointing at the newest document_versions row. No content lives here;
--     content is only ever a version.
--
--   * document_versions — the append-only history. One row per version
--     with the full title + body snapshot, the author, an optional change
--     summary, and `restored_from_version` (set when this version was
--     created by a RESTORE, naming the historical version whose content
--     was copied). Restores create NEW versions; history is never
--     rewritten — the binding overlay rule (§6) and the same posture as
--     audit_events: no code path UPDATEs or DELETEs version rows. The
--     UNIQUE (org_id, document_id, version) gate plus app-layer
--     MAX(version)+1 allocation under the instant-first entity seq lock
--     keeps numbering gap-free per document.
--
-- Diffability: every version stores its complete body snapshot, so any
-- pair of versions can be diffed client-side or via core-documents'
-- diff helper without reconstructing intermediate state.
--
-- Search integration follows the 2.16 pattern: the indexer re-reads the
-- CURRENT version content on document events and upserts `search_index`
-- kind `document` (vocabulary widened once in 0031).

CREATE TABLE documents (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  title text NOT NULL,
  current_version integer NOT NULL DEFAULT 1,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  updated_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE document_versions (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  document_id uuid NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
  version integer NOT NULL,
  title text NOT NULL,
  body text NOT NULL DEFAULT '',
  change_summary text NOT NULL DEFAULT '',
  author_id uuid REFERENCES users(id) ON DELETE SET NULL,
  restored_from_version integer,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, document_id, version)
);

CREATE INDEX idx_document_versions_doc
  ON document_versions (org_id, document_id, version DESC);

ALTER TABLE documents ENABLE ROW LEVEL SECURITY;
CREATE POLICY documents_org ON documents
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE document_versions ENABLE ROW LEVEL SECURITY;
CREATE POLICY document_versions_org ON document_versions
  USING (org_id = current_setting('app.org_id', true)::uuid);
