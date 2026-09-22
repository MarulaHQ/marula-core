-- Wave 22 lane 1 `p22-ecosystem-stretch` — Marketplace v2 (leg 1),
-- ADDITIVE over the ADR 0009 v1 distribution surfaces (registry /
-- GitHub / zip sources, staged imports, provenance). Two state tables:
--
-- 1. `plugin_publisher_verifications` — the org-curated VERIFIED
--    PUBLISHER registry. A publisher is a NORMALIZED SOURCE IDENTITY
--    text key (e.g. `github.com/owner`, `registry://slug`, or a bare
--    zip host); verification is per-org (this org vouches THIS
--    publisher), not global. `verified` is the active badge; `revoked`
--    keeps history and suppresses the badge. Every verification /
--    revocation rides the code-side `audit_events` trail and the typed
--    plugin.publisher_verified event — the table is state, not audit.
--
-- 2. `plugin_ratings` — per-user 1–5 star ratings on a distributable
--    artifact release, pinned by (source_kind, source_url,
--    artifact_digest) — the SAME triple that identifies a staged row or
--    provenance entry, so ratings survive stale staged rows and attach
--    across installs. One rating per user per release (upsert wins —
--    see `uq_plugin_rating_per_user`, which is registered in
--    `map_unique_constraint` as `rating_targeted`).

-- ---------------------------------------------------------------------------
-- 1. Verified publishers (per-org, additive — v1 formats untouched).
-- ---------------------------------------------------------------------------

CREATE TABLE plugin_publisher_verifications (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- Normalized source identity; verified via the same prefix the
  -- distribution routes validate (github owner/repo components,
  -- registry slug, host only for zips).
  publisher text NOT NULL,
  display_name text NOT NULL,
  status text NOT NULL CHECK (status IN ('verified', 'revoked')),
  verified_by uuid REFERENCES users(id) ON DELETE SET NULL,
  verified_at timestamptz NOT NULL DEFAULT now(),
  revoked_at timestamptz,
  revoked_by uuid REFERENCES users(id) ON DELETE SET NULL,
  note text NOT NULL DEFAULT '',
  -- badge state is either active or explicitly history:
  CHECK (status = 'verified' OR revoked_at IS NOT NULL)
);

-- A publisher identity verifies ONCE per org (re-verify = status flip).
CREATE UNIQUE INDEX uq_plugin_publisher_identity
  ON plugin_publisher_verifications (org_id, publisher);
CREATE INDEX idx_plugin_publisher_status
  ON plugin_publisher_verifications (org_id, status);

ALTER TABLE plugin_publisher_verifications ENABLE ROW LEVEL SECURITY;
CREATE POLICY plugin_publisher_verifications_org
  ON plugin_publisher_verifications
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

-- ---------------------------------------------------------------------------
-- 2. Artifacts ratings, the Marketplace v2 social layer.
-- ---------------------------------------------------------------------------

CREATE TABLE plugin_ratings (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  source_kind text NOT NULL,
  source_url text NOT NULL,
  artifact_digest text NOT NULL CHECK (artifact_digest <> ''),
  -- 1..5 stars; typed validation (API) + the storage CHECK both hold.
  rating smallint NOT NULL CHECK (rating >= 1 AND rating <= 5),
  comment text NOT NULL DEFAULT '' CHECK (length(comment) <= 2000),
  rated_by uuid REFERENCES users(id) ON DELETE SET NULL,
  rated_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

-- One rating per user per artifact release; the re-rate is an UPSERT
-- (added: the upsert path) — the constraint name is registered in
-- `map_unique_constraint` so a direct violation degrades to a typed
-- reason, never a sanitized generic.
CREATE UNIQUE INDEX uq_plugin_rating_per_user
  ON plugin_ratings (org_id, rated_by, source_url, artifact_digest);
CREATE INDEX idx_plugin_ratings_target
  ON plugin_ratings (org_id, source_kind, source_url, artifact_digest);

ALTER TABLE plugin_ratings ENABLE ROW LEVEL SECURITY;
CREATE POLICY plugin_ratings_org
  ON plugin_ratings
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);
