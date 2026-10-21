-- The identity-bundle artifact (lane p52-identity-bundle) — the
-- declarative half of a pluggable identity source.
--
-- Why: `crates/core-api/src/auth/federation.rs` defines the
-- FederationBundle seam (one identity source attached to the SAME
-- session machinery as a password login) and `auth/provider.rs` defines
-- the auth-provider contract behind it. Neither had an artifact, so an
-- operator hand-wrote rows. This migration gives the seam a distributed
-- unit in the shape plugins, skills and theme packs already use.
--
-- 1. `plugin_artifact_kind` enum grows `identity_bundle` — the artifact
--    rides the SAME staged state machine, the SAME owner/admin consent
--    diff and the SAME provenance chain. Nothing new authenticates until
--    an admin enables an approved install.
--
-- 2. `identity_bundle_installs` — org-scoped installed bundles. The row
--    is the byproduct of an APPROVED staged import and lands DISABLED
--    (approval adopts the artifact; activation is a second, audited
--    admin act). The pinned `provenance_digest` is the artifact digest
--    the reviewer consented to, and it survives revocation so the supply
--    chain outlives the install.
--
-- 3. Staged adoption link + the two coherence CHECKs re-created with the
--    new kind folded in. `adopted_identity_bundle_id` is stamped by the
--    approval path, so "approved implies an adoption handle" still holds.
--
-- What the table deliberately does NOT carry: credentials, issuer
-- secrets, session or token state, MFA policy. Sessions, tokens and MFA
-- are the core's (`auth::handlers::issue_tokens`, refresh-chain
-- rotation, `federated_mfa_gate`) and no column here can reach them.
-- The bundle's configuration is the manifest's `settings_schema` applied
-- to the settings registry, not a secret store.
--
-- RLS doctrine: org-scoped user data, org-bound reads (assert_rls_coverage).

-- ---------------------------------------------------------------------------
-- 1. The artifact kind.
-- ---------------------------------------------------------------------------

ALTER TYPE plugin_artifact_kind ADD VALUE IF NOT EXISTS 'identity_bundle';

-- ---------------------------------------------------------------------------
-- 2. Installed identity bundles.
-- ---------------------------------------------------------------------------

CREATE TABLE identity_bundle_installs (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- Slug of the manifest name (lowercase by contract); unique per org so
  -- a re-install of the same bundle replaces rather than forks.
  slug text NOT NULL,
  name text NOT NULL,
  version text NOT NULL,
  -- The bundle contract the artifact was authored against. The API
  -- refuses a mismatch before it reaches this column.
  interface_version integer NOT NULL,
  -- Declared transports (`oidc`, `edge_assertion`). A bundle may name at
  -- most one source per transport; the column is the reviewable record
  -- of what its executable half will attach.
  provider_kinds text[] NOT NULL,
  -- Upstream identity-provider scopes the bundle requests. These are the
  -- IdP's vocabulary, not Marula's: holding one grants nothing.
  scopes text[] NOT NULL DEFAULT '{}',
  -- Full validated manifest snapshot, so the consent screen, the
  -- settings renderer and an audit reconstruction read the same bytes.
  manifest jsonb NOT NULL,
  provenance_source_kind plugin_source_kind NOT NULL,
  provenance_source_url text NOT NULL,
  provenance_fetched_ref text,
  -- sha256 of the verified artifact — the pinning anchor.
  provenance_digest text NOT NULL,
  status plugin_install_status NOT NULL DEFAULT 'disabled',
  -- JIT provisioning is a per-install ADMIN opt-in and default-false;
  -- even when true the core only provisions from a verified email claim.
  jit_enabled boolean NOT NULL DEFAULT false,
  revoked_reason text,
  installed_by uuid NOT NULL,
  installed_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, slug),
  CONSTRAINT identity_bundle_installs_digest_is_sha256
    CHECK (provenance_digest ~ '^[0-9a-f]{64}$'),
  CONSTRAINT identity_bundle_installs_interface_version_positive
    CHECK (interface_version > 0),
  -- An install cannot be born ENABLED: turning one on is an explicit,
  -- audited UPDATE, so `updated_at` must have moved past `installed_at`
  -- by the time a row reads `enabled`.
  CONSTRAINT identity_bundle_installs_enable_is_an_event
    CHECK (status <> 'enabled' OR updated_at > installed_at),
  -- Revocation carries its reason; nothing else may.
  CONSTRAINT identity_bundle_installs_revoked_reason
    CHECK (status = 'revoked' OR revoked_reason IS NULL)
);

CREATE INDEX identity_bundle_installs_org_idx
  ON identity_bundle_installs (org_id);
CREATE INDEX identity_bundle_installs_org_status_idx
  ON identity_bundle_installs (org_id, status);

ALTER TABLE identity_bundle_installs ENABLE ROW LEVEL SECURITY;
CREATE POLICY identity_bundle_installs_org ON identity_bundle_installs
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- ---------------------------------------------------------------------------
-- 3. The staged state machine learns the new kind.
-- ---------------------------------------------------------------------------

-- The 0079/0097 source-coherence and approved-adoption checks are NAMED
-- constraints, and `pg_get_constraintdef` renders their string literals with a
-- `::text` cast (`source_kind::text IN ('skill_community'::text, …)`). A
-- pattern anchored on `source_kind IN ('skill_community'` therefore matches
-- NOTHING and the loop below becomes a no-op, which makes the ADD fail with
-- "constraint … already exists" on a schema built from scratch. Match on the
-- tokens instead of on the rendered syntax, so the cast (and any future OID
-- re-print of the same constraint) cannot break the drop.
DO $$
DECLARE c text;
BEGIN
  FOR c IN
    SELECT conname FROM pg_constraint
     WHERE conrelid = 'plugin_staged_items'::regclass
       AND contype = 'c'
       AND (pg_get_constraintdef(oid) LIKE '%source_kind%skill_community%'
            OR pg_get_constraintdef(oid) LIKE '%adopted_install_id IS NOT NULL%')
  LOOP
    EXECUTE format('ALTER TABLE plugin_staged_items DROP CONSTRAINT %I', c);
  END LOOP;
END $$;

ALTER TABLE plugin_staged_items
  ADD COLUMN adopted_identity_bundle_id uuid
    REFERENCES identity_bundle_installs(id),
  ADD CONSTRAINT plugin_staged_items_artifact_source_coherence CHECK (
    -- `::text` on both sides: the new enum value cannot be used as an
    -- enum literal inside this transaction (55P04) — text comparison
    -- avoids the safe-use restriction while keeping the same semantics.
    (artifact_kind::text = 'skill' AND source_kind::text IN ('skill_community', 'manual_upload', 'marketplace_registry', 'github_repo'))
    OR (artifact_kind::text = 'plugin' AND source_kind::text IN ('marketplace_registry', 'github_repo', 'local_zip'))
    OR (artifact_kind::text = 'theme' AND source_kind::text IN ('marketplace_registry', 'github_repo', 'local_zip'))
    OR (artifact_kind::text = 'identity_bundle' AND source_kind::text IN ('marketplace_registry', 'github_repo', 'local_zip', 'manual_upload'))),
  ADD CONSTRAINT plugin_staged_items_approved_adoption CHECK (
    status <> 'approved'
    OR (adopted_install_id IS NOT NULL
        OR adopted_skill_id IS NOT NULL
        OR adopted_theme_pack_id IS NOT NULL
        OR adopted_identity_bundle_id IS NOT NULL));

CREATE INDEX plugin_staged_items_adopted_identity_bundle_idx
  ON plugin_staged_items (adopted_identity_bundle_id)
  WHERE adopted_identity_bundle_id IS NOT NULL;
