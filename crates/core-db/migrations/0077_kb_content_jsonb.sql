-- KB article content → canonical Tiptap JSON (Wave 16 lane 2
-- `p16-kb-tiptap-model`, ADR 0007; brief:
-- docs/reference/wave16-content-model-and-vault.md).
--
-- Purpose: introduce the canonical rich-content column for kb_articles.
-- The stored representation is a VERSIONED ENVELOPE:
--
--     {"schema_ver": 1, "doc": <tiptap-doc>}
--
-- so future schema versions can coexist (U6 preflight-friendly: this
-- additive migration holds every existing article on the v1 render
-- path). The `doc` member is Tiptap JSON — a platform-neutral
-- ProseMirror tree — validated server-side in `crates/core-kb
-- ::content` against the PINNED published Tiptap JSON schema (pinned
-- in-repo at adoption) on every write. Nothing is dropped: `body`
-- (plain/markdown text) remains until a major-release contract step
-- (upgrade policy §4 expand/backfill/contract); reads fall back to
-- `body` when `content_jsonb` IS NULL.
--
-- Backfill (in-transaction, per the policy §4.5 conventions): each
-- existing article's text body is wrapped INTO the v1 envelope as a
-- plain-paragraph document — one paragraph per newline-delimited line,
-- escaped text nodes, no markup interpretation — so autosave history,
-- undo, and the read-only renderer see ONE canonical shape from the
-- instant this migration lands. This is a deterministic data wrap,
-- not a content transformation: stored text is never altered.
--
-- Non-purpose: no embed rows, no providers (0069 owns those), no old
-- column drop (never in an additive migration).

ALTER TABLE kb_articles
  ADD COLUMN content_jsonb jsonb;

-- FTS preflight support (jsonb_path_ops: smallest operator class that
-- still answers @@/jsonb_path queries; per-tree GIN stays cheap for
-- article-sized documents).
CREATE INDEX idx_kb_articles_content_fts_preflight
  ON kb_articles USING GIN (content_jsonb jsonb_path_ops);

-- Backfill: wrap every existing body into the v1 envelope. Empty
-- bodies become the Tiptap empty document (a doc holding one empty
-- paragraph); non-empty bodies become one plain text paragraph per
-- line. The WHERE keeps this statement a guarded incremental backfill
-- (idempotent when re-run against any replayed environment).
UPDATE kb_articles
SET content_jsonb = CASE
  WHEN body = '' THEN
    jsonb_build_object(
      'schema_ver', 1,
      'doc', jsonb_build_object(
        'type', 'doc',
        'content', jsonb_build_array(
          jsonb_build_object('type', 'paragraph')
        )
      )
    )
  ELSE
    jsonb_build_object(
      'schema_ver', 1,
      'doc', jsonb_build_object(
        'type', 'doc',
        'content', (
          SELECT jsonb_agg(
            jsonb_build_object('type', 'paragraph', 'content',
              jsonb_build_array(
                jsonb_build_object('type', 'text', 'text', t.line)
              )
            )
            ORDER BY t.ordinality
          )
          FROM unnest(string_to_array(body, E'\n')) WITH ORDINALITY
            AS t(line, ordinality)
        )
      )
    )
END
WHERE content_jsonb IS NULL;
