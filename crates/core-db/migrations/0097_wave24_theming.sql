-- Wave 24 lane 1 `p24-theming-system` — the theming system.
--
-- 1. `theme_packs` — org-scoped installed custom theme packs (ADR 0009
--    V1 distribution). A pack is the byproduct of an APPROVED themed
--    staged import (artifact_kind `theme`): the manifest carries ONLY
--    token overrides validated against the theme-token schema v1
--    (`core_domain::theme_pack` — closed role vocabulary, hex format,
--    AA/3:1 contrast floors; themes restyle and nothing else, zero
--    capabilities by construction). Provenance (source_kind/url/ref +
--    sha256 digest) is carried on the linked staged row's provenance
--    chain; stamping `approved` on the staged row links the pack. The
--    ACTIVE pack for an org rides the typed settings model
--    (`appearance.theme` value `pack:<slug>`), so uninstalling a pack
--    that the org theme points at must fail until the org repoints the
--    setting.
--
-- 2. `plugin_artifact_kind` enum grows `theme` — the closed distribution
--    kind vocabulary gains the restyle-only kind; the staged-items
--    coherence CHECK constraint is re-created to route theme packs
--    through the SAME denied-by-default staged state machine.
--
-- 3. U6-safe supersede of the flat theme keys: the old user-scope
--    `appearance.theme` rows (light/dark device mode) are renamed to
--    `appearance.mode` in place. Rows keep their scope/value; nothing
--    is dropped, so a prior saved preference keeps working after the
--    rename (the settings registry declares `appearance.mode` user scope
--    and `appearance.theme` becomes the org-wide preset picker).

-- ---------------------------------------------------------------------------
-- 1. Installed theme packs.
-- ---------------------------------------------------------------------------

CREATE TABLE theme_packs (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- Stable id inside the org (slugified manifest name; reused slugs
  -- bump the version on re-install, per the pack "versioned + audited"
  -- posture).
  slug text NOT NULL,
  name text NOT NULL,
  version text NOT NULL,
  description text NOT NULL DEFAULT '',
  -- The FULL validated manifest snapshot (identity + token overrides).
  manifest jsonb NOT NULL,
  schema_version integer NOT NULL DEFAULT 1,
  provenance_source_kind plugin_source_kind NOT NULL,
  provenance_source_url text NOT NULL,
  provenance_fetched_ref text,
  provenance_digest text,
  installed_by uuid NOT NULL,
  installed_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, slug)
);

CREATE INDEX theme_packs_org_idx ON theme_packs (org_id);

-- RLS doctrine (assert_rls_coverage): org-scoped state, org-bounded reads.
ALTER TABLE theme_packs ENABLE ROW LEVEL SECURITY;
CREATE POLICY theme_packs_org ON theme_packs
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- ---------------------------------------------------------------------------
-- 2. Distribution kind `theme` for the staged state machine.
-- ---------------------------------------------------------------------------

ALTER TYPE plugin_artifact_kind ADD VALUE IF NOT EXISTS 'theme';

-- The 0079 source-coherence checks are INLINE unnamed constraints; drop
-- them dynamically (match by definition text) and re-create the named
-- shape with `theme` added.
DO $$
DECLARE c text;
BEGIN
  FOR c IN
    SELECT conname FROM pg_constraint
     WHERE conrelid = 'plugin_staged_items'::regclass
       AND contype = 'c'
       AND (pg_get_constraintdef(oid) LIKE '%source_kind IN (''skill_community''%'
            OR pg_get_constraintdef(oid) LIKE '%adopted_install_id IS NOT NULL%')
  LOOP
    EXECUTE format('ALTER TABLE plugin_staged_items DROP CONSTRAINT %I', c);
  END LOOP;
END $$;

ALTER TABLE plugin_staged_items
  ADD COLUMN adopted_theme_pack_id uuid REFERENCES theme_packs(id),
  ADD CONSTRAINT plugin_staged_items_artifact_source_coherence CHECK (
    -- `::text` on both sides: the new enum value cannot be used as an
    -- enum literal inside this transaction (55P04) — text comparison
    -- avoids the safe-use restriction while keeping the same semantics.
    (artifact_kind::text = 'skill' AND source_kind::text IN ('skill_community', 'manual_upload', 'marketplace_registry', 'github_repo'))
    OR (artifact_kind::text = 'plugin' AND source_kind::text IN ('marketplace_registry', 'github_repo', 'local_zip'))
    OR (artifact_kind::text = 'theme' AND source_kind::text IN ('marketplace_registry', 'github_repo', 'local_zip'))),
  ADD CONSTRAINT plugin_staged_items_approved_adoption CHECK (
    status <> 'approved'
    OR (adopted_install_id IS NOT NULL
        OR adopted_skill_id IS NOT NULL
        OR adopted_theme_pack_id IS NOT NULL));

-- ---------------------------------------------------------------------------
-- 3. U6-safe supersede: flat user theme key → appearance.mode.
-- ---------------------------------------------------------------------------

UPDATE settings
   SET key = 'appearance.mode'
 WHERE key = 'appearance.theme' AND scope = 'user';
