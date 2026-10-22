-- Plugin lifecycle: the digest pin, the rollback target, and revoke's reason
-- (Wave 53 lane 4, `p53-lifecycle-distribution`; ADR 0023 Decisions 6 and 7).
--
-- Why this migration exists. Before it, nothing stored a plugin artifact per
-- install. `distribution.rs` verified and parsed a bundle and then dropped the
-- bytes, and `plugin_installs` carried a version string with no digest and no
-- module. An executor cannot be handed a module by a row that does not name
-- one, so the pin had to become data before anything could run.
--
-- The pin is the module digest, not the archive digest. A local zip import
-- computes `artifact_digest` over the ARCHIVE (that is what the provenance
-- chain records and what the staged row's identity is), while the thing that
-- runs is a `.wasm` member inside it. A pin over the archive cannot address a
-- module, so `plugin_staged_items` and `plugin_versions` carry a second digest
-- (`module_digest`) captured where the bytes were verified, and
-- `plugin_installs.module_digest` carries the one this install adopted.
--
-- Rollback is a pointer move, never a re-download. An update copies the
-- current (version, digest, grants) into the `previous_*` columns and pins the
-- new digest; a rollback swaps them back and clears them. The old bytes are
-- untouched because the object store keys modules by digest — the update wrote
-- a new key, it did not overwrite the old one. `previous_capabilities` is
-- carried because grants are version-scoped: restoring an older module while
-- keeping a newer grant set would run old code with authority its manifest
-- never declared.
--
-- Revoke is a status, not a deletion. `revoked_reason` rides here because
-- 0154's header reserved this: quarantine is not disablement is not revoke, and
-- the columns for the third belong on `plugin_installs`, where the status they
-- explain already lives. The reason is required by the route that sets it, and
-- the CHECK below keeps a reason from outliving its revocation.
--
-- Additive and forward-only: five columns on `plugin_installs`, one on
-- `plugin_versions`, one on `plugin_staged_items`, each with a default that
-- backfills existing rows (a legacy install keeps an empty pin and therefore
-- cannot be executed until it is re-adopted — deny-by-default, which is the
-- right answer for a row that never had an artifact).
--
-- RLS doctrine: no new table. Every column added belongs to a table that
-- already carries the org isolation policy (`plugin_installs`,
-- `plugin_staged_items`), or to the global registry (`plugin_versions`), which
-- 0076 documents as global by design.

ALTER TABLE plugin_staged_items
  -- sha256 of the executable module, when the import carries one. Empty for
  -- declarative-only imports, skills, themes and identity bundles.
  ADD COLUMN module_digest text NOT NULL DEFAULT '';

ALTER TABLE plugin_versions
  -- The digest this version's module was admitted under. Without it an update
  -- proposal has no artifact to name, which is exactly the gap this lane found:
  -- `propose_update` used to write `pending_digest = ''`.
  ADD COLUMN module_digest text NOT NULL DEFAULT '';

ALTER TABLE plugin_installs
  -- THE PIN. Empty = declarative-only install; the load path refuses.
  ADD COLUMN module_digest text NOT NULL DEFAULT '',
  -- The rollback target, set by an update and cleared by a rollback.
  ADD COLUMN previous_version text,
  ADD COLUMN previous_module_digest text,
  ADD COLUMN previous_capabilities jsonb,
  -- Revoke's reason, beside the status it explains.
  ADD COLUMN revoked_reason text,
  ADD CONSTRAINT plugin_installs_previous_pin_coherent
    CHECK ((previous_version IS NULL) = (previous_module_digest IS NULL)),
  ADD CONSTRAINT plugin_installs_previous_capabilities_array
    CHECK (previous_capabilities IS NULL
        OR jsonb_typeof(previous_capabilities) = 'array'),
  -- A reason describes a revocation; it may not exist without one.
  ADD CONSTRAINT plugin_installs_revoked_reason_needs_revocation
    CHECK (revoked_reason IS NULL OR revoked_at IS NOT NULL),
  -- A non-empty pin is a sha256 hex, so a truncated or prose digest cannot be
  -- stored and then trusted by the load path.
  ADD CONSTRAINT plugin_installs_module_digest_sha256
    CHECK (module_digest = '' OR module_digest ~ '^[0-9a-f]{64}$'),
  ADD CONSTRAINT plugin_installs_previous_module_digest_sha256
    CHECK (previous_module_digest IS NULL OR previous_module_digest ~ '^[0-9a-f]{64}$');

ALTER TABLE plugin_versions
  ADD CONSTRAINT plugin_versions_module_digest_sha256
    CHECK (module_digest = '' OR module_digest ~ '^[0-9a-f]{64}$');

ALTER TABLE plugin_staged_items
  ADD CONSTRAINT plugin_staged_items_module_digest_sha256
    CHECK (module_digest = '' OR module_digest ~ '^[0-9a-f]{64}$');

COMMENT ON COLUMN plugin_installs.module_digest IS
  'The install''s digest pin: sha256 hex of the executable module it adopted. '
  'Empty means declarative-only; the artifact load path refuses rather than '
  'guessing. Content-addressed object key: org/<org>/plugins/<digest>.';

COMMENT ON COLUMN plugin_installs.previous_module_digest IS
  'The pin a rollback returns to. Set by an update, cleared by a rollback; '
  'the old bytes stay in the store because keys are content-addressed.';
