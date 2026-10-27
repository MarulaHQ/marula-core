-- Manifest authentication on a catalog version (Wave 59 lane `p59-digest-only`).
--
-- Record `plugin-distribution.digest-only-is-adoptable-not-distributable`.
-- Migration 0167 put `manifest_authenticated` on the STAGED row, which answered
-- the approver's question ("is this manifest signed?") and stopped there. It
-- could not answer the registry's question, because the staged row is the
-- uploading org's and the catalog is server-wide: by the time another tenant
-- installs by slug, the only facts left are the entry and its version. So the
-- weaker claim fell off exactly where it mattered, and a GitHub pull whose
-- manifest nobody signed was installable by any org as if a publisher had
-- signed it.
--
-- WHAT THE COLUMN MEANS. TRUE only when a signature the operator's trust set
-- verified covers THIS manifest. FALSE for a digest-only legacy registry
-- statement, a GitHub pull, a local zip, a manual skill upload, a declarative
-- publish (no bytes and no statement pass through that route), and any ingest
-- run with no trusted key configured. It is the same claim 0167 records, moved
-- to the row that outlives the staged item.
--
-- WHAT THE ROUTE DOES WITH IT. An unauthenticated version stays adoptable by the
-- org that staged it (that org is the one that read the consent screen) and is
-- refused to every other org with the typed `manifest_unauthenticated` 409. An
-- authenticated version installs across orgs as before. An entry with a NULL
-- `origin_org_id` belongs to no org, so it is adoptable by none — the
-- "authentication unknown" case fails closed rather than open.
--
-- THE BACKFILL IS THE HONEST HALF, AND IT IS MOSTLY `false`. No existing version
-- row recorded the verdict, so nothing in `plugin_versions` can be promoted on
-- its own. The one link that proves a version came from a verified statement is
-- the adoption chain: `plugin_staged_items.adopted_install_id` names the install
-- approval minted, and that install names the (catalog, version) it pinned. The
-- UPDATE below promotes exactly those rows, and only when the staged item's own
-- 0167 verdict was TRUE. Everything else stays `false`:
--
--   * a declarative publish (no staged row, no statement);
--   * a version approved from a digest-only staged row;
--   * a version whose staged row was refused, or whose install was since
--     uninstalled (the chain is gone, and a lost proof is not a proof);
--   * any row this join cannot reach.
--
-- The consequence of a wrong `false` is a legitimate cross-org install refused
-- with a reason an operator can read and a re-stage that fixes it. The
-- consequence of a wrong `true` is an unsigned manifest distributed under a
-- publisher claim, which is the defect this lane closes. So the default goes on
-- the weaker side.
--
-- Additive and forward-only: one boolean with a default, so the backfill is a
-- catalog write and no row is rewritten. `plugin_versions` is the global
-- registry (0076 documents it as server-wide, no RLS); no new table, no new
-- policy, no access path.

ALTER TABLE plugin_versions
  -- TRUE only when a trusted signing key verified a statement that covers THIS
  -- manifest. FALSE for a digest-only statement, a repository pull, a local
  -- archive, a declarative publish, and any ingest with no trusted key.
  ADD COLUMN manifest_authenticated boolean NOT NULL DEFAULT false;

-- The adoption chain, and the only promotion this migration can prove: the
-- staged item that minted the install was itself manifest-authenticated.
UPDATE plugin_versions v
SET manifest_authenticated = true
FROM plugin_installs i
JOIN plugin_staged_items s
  ON s.adopted_install_id = i.id
 AND s.org_id = i.org_id
WHERE i.catalog_id = v.catalog_id
  AND i.version = v.version
  AND s.manifest_authenticated
  AND s.artifact_kind = 'plugin';

COMMENT ON COLUMN plugin_versions.manifest_authenticated IS
  'Whether a signature the operator trust set verified covers this version''s '
  'manifest, not just its artifact bytes. TRUE installs across orgs; FALSE is '
  'adoptable only by the org that staged it (migration 0172, backfilled from '
  'the staged item where the adoption chain proves it, FALSE otherwise).';
