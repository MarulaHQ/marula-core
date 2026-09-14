-- Plugin runtime v2 + marketplace registry (Wave 14 lane 5, p14-plugin-v2).
--
-- Purpose: the data model for the declarative plugin manifest/permissions
-- model and the marketplace registry catalog:
-- (1) `plugin_catalog` — the GLOBAL marketplace registry (server-wide,
--     not org-scoped; mirrors `orgs`). One row per catalog entry: a
--     declarative manifest snapshot (name/version/capabilities — DATA,
--     never code). Install = fork-like ADOPTION of a catalog entry; no
--     plugin code is ever uploaded, inlined, or injected through this
--     surface. `revoked_at`/`revoked_reason` is the registry-level kill
--     switch: a revoked entry denies NEW installs and marks every
--     existing install revoked (deny-by-default — a revoked plugin never
--     runs).
-- (2) `plugin_versions` — per-entry version history. Each version pins
--     the FULL declarative manifest JSON (v2 shape: name, version,
--     interface_version, description, capabilities) so an org adopting
--     version N sees exactly the capability set version N declared.
-- (3) `plugin_installs` — one org's adoption of one catalog entry.
--     Deny-by-default posture (mirrors federation bundle sources,
--     migration 0072): an install is created DISABLED and only an
--     explicit admin action enables it. `granted_capabilities` is the
--     subset of the manifest's declared capabilities the org granted —
--     grants are DATA (capability keys from a CLOSED vocabulary), and
--     each key maps onto the EXISTING permission matrix
--     (core_domain::policy::Permission) — no parallel auth system.
--     `version` pins the adopted version; upgrades are explicit admin
--     actions, never automatic.
--
-- Invariants encoded here:
--   * deny-by-default: status DEFAULT 'disabled'; the runtime seam
--     refuses invocations unless the install row is 'enabled', the
--     catalog entry is NOT revoked, and the capability resolves through
--     the standard permission matrix.
--   * closed capability vocabulary: the granted_capabilities JSONB
--     carries strings validated against the closed Rust vocabulary at
--     the write boundary (core-api) — the DB stores data, the policy
--     crate owns meaning.
--   * audit: every lifecycle step (catalog entry created/revoked,
--     install created/enabled/disabled/uninstalled) is audited through
--     the EXISTING append-only `audit_events` table via the route layer
--     — no parallel audit store.
--
-- RLS doctrine: `plugin_installs` is org-scoped user data and carries
-- the org isolation policy (the CI gate `assert_rls_coverage` fails
-- otherwise). `plugin_catalog` and `plugin_versions` are global
-- registry rows WITHOUT org_id — global by design, readable through the
-- gated API surface only (like `orgs`), so no policy applies.

CREATE TYPE plugin_install_status AS ENUM ('disabled', 'enabled', 'revoked');

CREATE TABLE plugin_catalog (
  id uuid PRIMARY KEY,
  -- Stable marketplace slug (the adoption handle).
  slug text NOT NULL,
  name text NOT NULL,
  description text NOT NULL DEFAULT '',
  publisher text NOT NULL,
  -- Denormalized pointer to the newest version (kept in step by the
  -- write path); the authoritative history lives in plugin_versions.
  latest_version text,
  -- The plugin-runtime interface version this entry was published
  -- against; the runtime refuses version mismatches (versioned
  -- interface, deny-by-default).
  interface_version integer NOT NULL,
  -- Registry-level revocation (the kill switch). NULL = live.
  revoked_at timestamptz,
  revoked_reason text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CHECK (revoked_at IS NULL OR revoked_reason IS NOT NULL)
);

-- The constraint name is consumed by the typed reason-code registry:
-- `plugin_slug_taken`.
CREATE UNIQUE INDEX plugin_catalog_slug_key ON plugin_catalog (slug);

CREATE TABLE plugin_versions (
  id uuid PRIMARY KEY,
  catalog_id uuid NOT NULL REFERENCES plugin_catalog(id) ON DELETE CASCADE,
  version text NOT NULL,
  -- The FULL declarative manifest for this version (v2 shape). Data,
  -- never code: the runtime executes nothing from this document.
  manifest jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);

-- The constraint name is consumed by the typed reason-code registry:
-- `plugin_version_taken`.
CREATE UNIQUE INDEX plugin_versions_catalog_id_version_key
  ON plugin_versions (catalog_id, version);

CREATE TABLE plugin_installs (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  catalog_id uuid NOT NULL REFERENCES plugin_catalog(id) ON DELETE CASCADE,
  -- The adopted (pinned) version. Upgrades are explicit admin actions.
  version text NOT NULL,
  -- Deny-by-default: a new install is DISABLED until an admin
  -- explicitly enables it.
  status plugin_install_status NOT NULL DEFAULT 'disabled',
  -- JSONB array of capability keys granted to this install — a SUBSET
  -- of the pinned version's declared capabilities. Grants are DATA;
-- meaning comes from the existing permission matrix.
  granted_capabilities jsonb NOT NULL DEFAULT '[]'::jsonb,
  installed_by uuid REFERENCES users(id) ON DELETE SET NULL,
  installed_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  revoked_at timestamptz,
  revoked_by uuid REFERENCES users(id) ON DELETE SET NULL,
  CHECK (jsonb_typeof(granted_capabilities) = 'array')
);

-- One adoption per (org, catalog entry); re-install after uninstall is
-- a fresh row. The constraint name is consumed by the typed
-- reason-code registry: `plugin_already_installed`.
CREATE UNIQUE INDEX plugin_installs_org_id_catalog_id_key
  ON plugin_installs (org_id, catalog_id);
CREATE INDEX idx_plugin_installs_org_status
  ON plugin_installs (org_id, status);

ALTER TABLE plugin_installs ENABLE ROW LEVEL SECURITY;
CREATE POLICY plugin_installs_org ON plugin_installs
  USING (org_id = current_setting('app.org_id', true)::uuid);
