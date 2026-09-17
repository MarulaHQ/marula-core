-- Plugin & skill distribution lifecycle (Wave 16 lane 7, p16-plugin-skill-mgmt,
-- ADR 0009). Lane-exclusive migration pair 0079 (PG port 54379 is the
-- branch-local test port). Head ride: 0078 (secret_vault_trees).
--
-- Purpose: the DISTRIBUTION half of the Wave 14 runtime-v2 seam — sources,
-- staged member imports, the update re-consent path, manifest-rendered
-- settings, and agent skills riding the skills half of one lifecycle. This
-- migration ADDS beside migration 0076 (plugin_catalog / plugin_versions /
-- plugin_installs) — it never alters the shipped install semantics:
--
-- (1) `plugin_source` enum + `plugin_artifact_kind` + `plugin_staged_status`
--     — the closed source vocabulary every adapter records (the SAME
--     provenance chain shape for all of them) and the staged state machine.
--
-- (2) `plugin_staged_items` — a member upload (skill or plugin) lands
--     PENDING/staged: validated, never executed, visible only to its
--     uploader and admins. Staging is the row BEFORE install, not a bypass:
--     approval (owner/admin consent diff) is the path that MINTS the
--     catalog entry/version + install (plugins) or the agent skill (skills).
--     Refusal is terminal for the staged row; the uploader can re-stage.
--
-- (3) `plugin_install_provenance` — the append-only provenance chain
--     `{source_kind, source_url, fetched_ref, artifact_digest, uploader}`
--     typed by source kind, keyed by install id WITHOUT a cascading DELETE
--     reference: uninstall must never erase the supply chain (ADR 0009 §5
--     terminal-uninstall data survival; rows keep feeding the audit trail
--     after the adoption row goes away).
--
-- (4) `plugin_installs` update-reconsent columns — an updateProposal pins
--     the OLD version (distributor semantics: "old version pinned until
--     consented"): `pending_version`/`pending_digest` carry the new
--     candidate, `pending_manifest_json` carries its full declarative
--     manifest for the consents diff, and `pending_capabilities` carries
--     the granted set that would apply post-consent. While the pending
--     columns are set the install keeps running its CURRENT pinned version;
--     consent (re-consent diff accepted) is the swap transaction.
--
-- (5) `plugin_install_settings` — per-install configuration rendered FROM
--     the manifest's declared JSON-Schema (`settings_schema` additive
--     manifest field). One row per install; the config is DATA validated
--     against the pinned manifest's schema at the write boundary (same
--     typed-registry doctrine as KB embeds / widget registry props — no
--     hand-authored per-plugin pages). Die with the install row.
--
-- (6) `agent_skills` + `agent_skill_versions` — the skills half of the
--     lifecycle, reused ONLY for skills (no executable lifecycle):
--     import/edit/enable/version/rollback. Skill content is declarative
--     prompt/rule manifests handled as UNTRUSTED RAG-class data
--     (docs/10-agent-platform.md §4 — never executed as instructions);
--     each version snapshots the full skill manifest + its provenance
--     (source kind/url/ref/digest) so history is append-only, diffable,
--     rollback-able. Roam: enable is deny-by-default (`enabled` DEFAULT
--     false, an explicit admin/owner action).
--
-- Invariants encoded here:
--   * deny-by-default: staged rows start 'pending'; agent skills start
--     disabled; the runtime seam (crates/plugin-host authorize_invocation)
--     still refuses anything not enabled/granted/catalog-live.
--   * append-only provenance: no UPDATE path is granted for
--     `plugin_install_provenance` columns — the API layer only INSERTs;
--     rows survive uninstall (no cascade reference).
--   * closed vocabularies: source_kind is an ENUM; the write path rejects
--     a GitHub pull without pinned tag+commit (never HEAD) at the API
--     boundary — the DB stores data, core-domain owns meaning.
--
-- RLS doctrine: `plugin_staged_items`, `plugin_install_provenance`,
-- `agent_skills`, `agent_skill_versions` are org-scoped user data and carry
-- the org isolation policy; `plugin_install_settings` inherits the install
-- row's org scope via cascade FK. CI gate `assert_rls_coverage` enforces.

CREATE TYPE plugin_source_kind AS ENUM
  ('marketplace_registry', 'github_repo', 'local_zip', 'skill_community', 'manual_upload');
CREATE TYPE plugin_artifact_kind AS ENUM ('plugin', 'skill');
CREATE TYPE plugin_staged_status AS ENUM ('pending', 'approved', 'refused');

-- (2) staged member imports ---------------------------------------------------

CREATE TABLE plugin_staged_items (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  artifact_kind plugin_artifact_kind NOT NULL,
  source_kind plugin_source_kind NOT NULL,
  -- Where it came from (registry URL, GitHub owner/repo, or the zip's
  -- declared origin/filename for local imports). Always recorded.
  source_url text NOT NULL DEFAULT '',
  -- The pinned fetch reference: a registry/marketplace git tag, a GitHub
  -- `tag@sha` pair, or NULL for one-shot local uploads.
  fetched_ref text,
  -- sha256 of the verified artifact (hex). NULL only while the row is
  -- 'pending' for a skill whose content arrived inline.
  artifact_digest text NOT NULL DEFAULT '',
  -- The staged declarative manifest (plugin manifest v2 incl.
  -- settings_schema, or the skill manifest). Data, never code: nothing
  -- here is ever executed; it is what the consent screen renders.
  manifest jsonb NOT NULL,
  -- The candidate manifest's declared capabilities (skill manifests use
  -- agent capability scopes). The consent diff is staged-declared vs
  -- granted at approval time.
  declared_capabilities jsonb NOT NULL DEFAULT '[]'::jsonb,
  -- Deny-by-default staging state machine: pending → approved|refused.
  status plugin_staged_status NOT NULL DEFAULT 'pending',
  -- Staging identity + visibility: uploader sees own rows, admins see all.
  uploaded_by uuid NOT NULL REFERENCES users(id) ON DELETE SET NULL,
  uploaded_at timestamptz NOT NULL DEFAULT now(),
  review_note text,
  reviewed_by uuid REFERENCES users(id) ON DELETE SET NULL,
  reviewed_at timestamptz,
  -- The row minted on approval (catalog entry or agent skill), for trace.
  adopted_install_id uuid,
  adopted_skill_id uuid,
  updated_at timestamptz NOT NULL DEFAULT now(),
  CHECK ((status = 'pending' AND reviewed_at IS NULL)
      OR (status <> 'pending' AND reviewed_at IS NOT NULL)),
  CHECK ((artifact_kind = 'skill' AND source_kind IN ('skill_community', 'manual_upload', 'marketplace_registry', 'github_repo'))
      OR (artifact_kind = 'plugin' AND source_kind IN ('marketplace_registry', 'github_repo', 'local_zip'))),
  CHECK (status <> 'approved' OR (adopted_install_id IS NOT NULL OR adopted_skill_id IS NOT NULL))
);

-- One live staged row per (org, uploader-diagnosable artifact identity):
-- same name+version may not stage twice while pending (re-upload replaces
-- content by refusing the older row first at the API boundary).
CREATE UNIQUE INDEX plugin_staged_items_pending_org_artifact_key
  ON plugin_staged_items (org_id,
    (manifest ->> 'name'),
    (manifest ->> 'version'))
  WHERE status = 'pending';

ALTER TABLE plugin_staged_items ENABLE ROW LEVEL SECURITY;
CREATE POLICY plugin_staged_items_org ON plugin_staged_items
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- (3) append-only provenance chain (survives uninstall — NO cascade FK) -------

CREATE TABLE plugin_install_provenance (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- Plain uuid BY DESIGN: the install row may be deleted (terminal
  -- uninstall) while the chain must keep feeding the audit trail.
  install_id uuid NOT NULL,
  source_kind plugin_source_kind NOT NULL,
  source_url text NOT NULL DEFAULT '',
  fetched_ref text,
  artifact_digest text NOT NULL DEFAULT '',
  uploader uuid REFERENCES users(id) ON DELETE SET NULL,
  -- 'staged' | 'installed' | 'updated' | 're_consented' | 'enabled' | 'disabled'
  action text NOT NULL,
  detail jsonb,
  recorded_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX idx_plugin_install_provenance_install
  ON plugin_install_provenance (install_id, recorded_at);

-- Append-only is a POSTURE (no grantees mutate it): a plain materialized
-- truth check — the table only receives INSERTs from the route layer.
ALTER TABLE plugin_install_provenance ENABLE ROW LEVEL SECURITY;
CREATE POLICY plugin_install_provenance_org ON plugin_install_provenance
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- (4) update re-consent columns on the shipped install table ------------------

ALTER TABLE plugin_installs
  ADD COLUMN pending_version text,
  ADD COLUMN pending_manifest jsonb,
  ADD COLUMN pending_digest text,
  ADD COLUMN pending_capabilities jsonb,
  ADD COLUMN pending_consent_at timestamptz,
  ADD CONSTRAINT plugin_installs_pending_version_manifest
    CHECK (pending_version IS NULL OR pending_manifest IS NOT NULL);

-- (5) manifest-rendered per-install settings ----------------------------------

CREATE TABLE plugin_install_settings (
  install_id uuid PRIMARY KEY REFERENCES plugin_installs(id) ON DELETE CASCADE,
  -- Validated against the pinned manifest's settings_schema JSON-Schema at
  -- the write boundary BEFORE this row exists.
  config jsonb NOT NULL DEFAULT '{}'::jsonb,
  updated_by uuid REFERENCES users(id) ON DELETE SET NULL,
  updated_at timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE plugin_install_settings ENABLE ROW LEVEL SECURITY;
CREATE POLICY plugin_install_settings_org ON plugin_install_settings
  USING (EXISTS (
    SELECT 1 FROM plugin_installs i
    WHERE i.id = plugin_install_settings.install_id
      AND i.org_id = current_setting('app.org_id', true)::uuid));

-- (6) agent skills EXTENSION — the distribution columns ride the
--     existing `agent_skills` versioned-rows table (migration 0040:
--     one row per skill version, append-only history; rollback = a NEW
--     version carrying an old manifest — never rewritten). Each version
--     row carries its OWN provenance (source kind/url/ref/digest);
--     enable is deny-by-default on the active version row.

ALTER TABLE agent_skills
  ADD COLUMN description text NOT NULL DEFAULT '',
  ADD COLUMN enabled boolean NOT NULL DEFAULT false,
  ADD COLUMN enabled_by uuid REFERENCES users(id) ON DELETE SET NULL,
  ADD COLUMN enabled_at timestamptz,
  ADD COLUMN source_kind plugin_source_kind,
  ADD COLUMN source_url text NOT NULL DEFAULT '',
  ADD COLUMN fetched_ref text,
  ADD COLUMN artifact_digest text NOT NULL DEFAULT '';

CREATE INDEX idx_agent_skills_org_enabled ON agent_skills (org_id, enabled);
