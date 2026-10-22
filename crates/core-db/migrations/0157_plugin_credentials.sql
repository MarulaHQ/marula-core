-- Credentialed egress, phase 1: the binding between an install's egress
-- profile and an org's vault record (Wave 53 lane 12, `p53-credentialed-egress`).
--
-- Why this table exists. Wave 53 shipped `http_request`: a guest calls an
-- allowlisted host, and `crates/plugin-host/src/http.rs` has a
-- `HostCredentials` seam that nothing fills. A secret is only decryptable by a
-- process that must never run untrusted wasm, so a credentialed request is
-- performed parent-side, in core-api, and the seam takes a header NAME and a
-- VALUE at the moment of the request. Between "the guest named a profile" and
-- "core-api injects a header" there has to be a durable answer to one
-- question: which vault record, in which tenant, may this install wear?
--
-- Why the binding cannot come from the manifest. A connector profile is
-- vendor-authored (`PluginManifestV2::connector_profile`) and ships inside an
-- artifact any publisher can write. If the manifest named a vault record, a
-- stranger would be choosing which of the org's secrets an install reads; a
-- record id is also an org-scoped uuid a vendor has no way to know. So the
-- manifest names the PROFILE (`outbound_scopes`: profile -> allowlisted URL
-- prefixes) and the org's own admin names the record. This table is that
-- decision, per install, taken once, visible before it is used.
--
-- Why the row carries no value and no field. `secret_record_id` is a pointer;
-- the material stays in `secret_fields.value_ciphertext` and is never copied.
-- The field is not a column because the vault already has one canonical
-- material key (`secret`, the closed vocabulary in
-- `crates/core-api/src/routes/secrets.rs`), and a second, plugin-chosen field
-- name would be a second way to say "the secret part of this record". The
-- resolve therefore reads `field_key = 'secret'`; a record without that field
-- refuses rather than injecting `notes` or `username` in its place.
--
-- The vault's category grant is the authority, and there is no plugin grant
-- table here on purpose. A binding records the roles that consented
-- (`bound_role_ids`), and both the bind and the resolve walk
-- `secret_vault::folder_access` — the same recursive role x folder check the
-- audited reveal endpoint runs. `bound_role_ids` is an array of uuids with no
-- foreign key: a deleted role leaves a stale id, `folder_access` then finds no
-- grant row for it and the resolve refuses. Stale fails closed, which is the
-- only way this column may fail.
--
-- What a pin move does to a binding: it deletes it, and the trigger at the
-- bottom is why. `outbound_scopes` is manifest-declared, so the SAME profile
-- name in the next version can allowlist a different host. A binding that
-- survived an update would be a consent to "profile `acme`" quietly extended
-- to whatever host the new publisher named, with the org's credential attached
-- and no screen in between. A rollback is the same move in the other
-- direction and is cleared the same way. The operator re-binds against the
-- manifest they consented to; the update proposal names the bindings consent
-- would clear, so the loss is visible before the click and not after.
--
-- Scope: ORG-level vault records only. A client-scope record (`org_id IS NULL`
-- and `client_id` set, migration 0078's exactly-one-of pair) is deliberately
-- not bindable. An install is org-wide, and a client's stored credential
-- handed to a vendor endpoint on every ticket that client appears in is a
-- disclosure the client never consented to. Widening this is a decision with
-- its own consent surface, not a predicate relaxed here.
--
-- RLS doctrine: org-scoped user data, org-bound reads (assert_rls_coverage).
-- The policy is 0154/0155's, verbatim: one plugin-state family should not ship
-- two shapes. The policy is not what stops a foreign-org record being bound —
-- the INSERT in `plugin_credentials::bind` selects the record through the
-- caller's org and inserts nothing when the record is not there, and
-- `crates/core-db/tests/plugin_credential_queries.rs` proves it. A foreign key
-- cannot express that rule: the database checks that the record exists, not
-- that it is ours.
--
-- Additive and forward-only: one new table and one trigger. No existing column
-- changes shape, and a pre-existing install simply has no bindings, which is
-- exactly the pre-feature behaviour.

CREATE TABLE plugin_credential_bindings (
  id uuid PRIMARY KEY,
  -- The tenant. Bound by the query, and what the policy below isolates.
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- The install that wears the credential. `ON DELETE CASCADE` is the
  -- uninstall path: an install that goes away takes its consents with it, with
  -- no code required to remember.
  install_id uuid NOT NULL REFERENCES plugin_installs(id) ON DELETE CASCADE,
  -- The key in the pinned manifest's `outbound_scopes`. Shape mirrors
  -- `PluginManifestV2::validate_outbound_scopes` ([a-z][a-z0-9_-]{0,63}), so a
  -- profile name the manifest would refuse cannot be parked here first.
  profile text NOT NULL,
  -- The vault record the org consented to. `ON DELETE CASCADE` for the same
  -- reason as the install: a deleted credential must not leave a binding that
  -- looks live on the consent screen.
  secret_record_id uuid NOT NULL REFERENCES secret_records(id) ON DELETE CASCADE,
  -- The header the value is injected as, lowercased at write time (one header
  -- has one spelling). The transport refuses a name that is hops or
  -- framing (`HOST_OWNED_HEADERS`, plugin-host/src/http.rs); this CHECK only
  -- keeps a name the ABI could never accept out of the table.
  header_name text NOT NULL,
  -- Who consented, and which roles they held when they did. The roles are the
  -- authority the resolve re-checks; the user is the trail.
  bound_by uuid REFERENCES users(id) ON DELETE SET NULL,
  bound_role_ids uuid[] NOT NULL DEFAULT '{}'::uuid[],
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  -- One credential per profile: a guest naming a profile gets one header, not
  -- a list whose order decides which secret went out.
  CONSTRAINT plugin_credential_bindings_install_profile_key
    UNIQUE (install_id, profile),
  CONSTRAINT plugin_credential_bindings_profile_shape
    CHECK (profile ~ '^[a-z][a-z0-9_-]{0,63}$'),
  CONSTRAINT plugin_credential_bindings_header_shape
    CHECK (header_name ~ '^[a-z0-9][a-z0-9!#$%&''*+.^_`|~-]{0,63}$')
);

ALTER TABLE plugin_credential_bindings ENABLE ROW LEVEL SECURITY;
CREATE POLICY plugin_credential_bindings_org ON plugin_credential_bindings
  USING (org_id = current_setting('app.org_id', true)::uuid);

COMMENT ON TABLE plugin_credential_bindings IS
  'Per-install consent: this egress profile may present this vault record as '
  'this header. Points at secret_records and stores no value, no field and no '
  'key; the material stays in secret_fields, sealed. Bind and resolve both '
  'walk secret_vault::folder_access, so the vault category grant is the only '
  'authority and there is no plugin-specific grant vocabulary.';

COMMENT ON COLUMN plugin_credential_bindings.bound_role_ids IS
  'The roles the binding operator held when they consented. The resolve '
  're-runs folder_access with exactly these roles and refuses when none of '
  'them still holds read on the record''s category. No foreign key, on '
  'purpose: a deleted role leaves a stale uuid and the check fails closed.';

COMMENT ON COLUMN plugin_credential_bindings.header_name IS
  'Lowercased header the credential is injected as. Chosen by the org admin, '
  'never by the manifest: a publisher that could name the header could name '
  'Authorization.';

-- A pin move clears the bindings, and it cannot be forgotten by a caller.
--
-- `commit_pending_version` and `rollback_pin` both move
-- (version, module_digest), and both are ordinary UPDATEs that a future route
-- could reimplement. Putting the rule in a trigger means the guarantee holds
-- for a hand-written statement too. The guard is IS DISTINCT FROM, so an
-- update that re-states the same version (PATCH /installed/{id} does) leaves
-- the bindings alone.
--
-- The audit row is written here rather than in the route for the same reason:
-- the trail should exist even when the caller did not ask for it. actor_kind
-- is 'system' because a trigger has no caller identity; the detail names both
-- versions and how many bindings went, which is what a reviewer needs.
CREATE OR REPLACE FUNCTION plugin_credential_bindings_clear_on_pin_move()
RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  cleared integer;
BEGIN
  IF NEW.version IS DISTINCT FROM OLD.version
     OR NEW.module_digest IS DISTINCT FROM OLD.module_digest THEN
    DELETE FROM plugin_credential_bindings WHERE install_id = NEW.id;
    GET DIAGNOSTICS cleared = ROW_COUNT;
    IF cleared > 0 THEN
      INSERT INTO audit_events
        (id, org_id, actor_kind, actor_id, action, entity_type, entity_id, detail, occurred_at)
      VALUES
        (gen_random_uuid(), NEW.org_id, 'system', NULL,
         'plugin.credentials_cleared_on_pin_move', 'plugin_install', NEW.id::text,
         jsonb_build_object(
           'cleared_bindings', cleared,
           'from_version', OLD.version,
           'to_version', NEW.version,
           'from_digest', OLD.module_digest,
           'to_digest', NEW.module_digest
         ),
         now());
    END IF;
  END IF;
  RETURN NULL;
END;
$$;

CREATE TRIGGER plugin_installs_clear_credentials_on_pin_move
  AFTER UPDATE OF version, module_digest ON plugin_installs
  FOR EACH ROW EXECUTE FUNCTION plugin_credential_bindings_clear_on_pin_move();
