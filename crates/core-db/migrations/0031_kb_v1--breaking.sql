-- Native knowledge base v1 (the private build tracker Phase 3 "Native KB v1: articles,
-- categories, FTS, client-facing flags").
--
-- Purpose: two org-scoped tables backing crates/core-kb and the /api/kb
-- surface:
--
--   * kb_categories — ordering + grouping for the article tree. `slug` is
--     the stable per-org handle (UNIQUE) used by portals and imports; the
--     row id never leaks into URLs.
--
--   * kb_articles — the article body of work. `status` is a closed
--     vocabulary (draft | published | archived) so filtering and the
--     client-facing rule stay deterministic; `client_visible` is the
--     per-article flag that portal v2 (the private build tracker Phase 3) will honor — the
--     effective client view is published AND article flag AND category
--     flag (core-kb::client_viewable). Deletion of a category leaves its
--     articles in place (ON DELETE SET NULL) so URLs never rot behind an
--     admin reorganization.
--
--     slugs: UNIQUE (org_id, slug) is the collision gate; a duplicate
--     surfaces as a typed conflict through the central error layer.
--
-- Search integration follows the 2.16 unified-search pattern: articles are
-- indexed into `search_index` as kind `kb_article` by the worker's
-- event-driven indexer (auto-indexing from domain events). Migration 0031
-- therefore ALSO widens the search_index kind vocabulary to the complete
-- Wave-8b set (`kb_article` for this lane, `document` for the D4 lane
-- landing in 0032) so the CHECK is rewritten exactly once:
--
--   BREAKING-FLAG DISCLOSURE (upgrade-compatibility-policy.md §4.3): the
--   filename carries `--breaking` solely because replacing a CHECK
--   constraint requires DROP CONSTRAINT, which the migration lint gates.
--   DATA IMPACT: none. The widened constraint only ACCEPTS MORE values;
--   no row is rewritten, dropped, or retyped, and existing reads are
--   unchanged. The constraint is re-added in the same statement.

CREATE TABLE kb_categories (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  slug text NOT NULL,
  description text NOT NULL DEFAULT '',
  position integer NOT NULL DEFAULT 0,
  client_visible boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, slug)
);

CREATE TABLE kb_articles (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  category_id uuid REFERENCES kb_categories(id) ON DELETE SET NULL,
  title text NOT NULL,
  slug text NOT NULL,
  body text NOT NULL DEFAULT '',
  status text NOT NULL DEFAULT 'draft'
    CHECK (status IN ('draft','published','archived')),
  client_visible boolean NOT NULL DEFAULT false,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  updated_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, slug)
);

CREATE INDEX idx_kb_articles_org_category ON kb_articles (org_id, category_id);
CREATE INDEX idx_kb_articles_org_status ON kb_articles (org_id, status);

-- 2.16 unified-search vocabulary widening (kb_article + document; see the
-- disclosure above for the --breaking flag rationale — data impact: none).
ALTER TABLE search_index
  DROP CONSTRAINT search_index_entity_kind_check,
  ADD CONSTRAINT search_index_entity_kind_check
    CHECK (entity_kind IN ('ticket','comment','client','site','contact',
                           'kb_article','document'));

ALTER TABLE kb_categories ENABLE ROW LEVEL SECURITY;
CREATE POLICY kb_categories_org ON kb_categories
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE kb_articles ENABLE ROW LEVEL SECURITY;
CREATE POLICY kb_articles_org ON kb_articles
  USING (org_id = current_setting('app.org_id', true)::uuid);
