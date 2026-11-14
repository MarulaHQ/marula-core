-- 0186 — the connector write path (ADR 0032 §"Connection", ADR 0033).
--
-- 0185 shipped the connection model read-only: the tables exist, the resolver
-- reads them, and nothing in the product can create a row. This migration
-- carries the two columns that write path needs and nothing else. It adds no
-- table, so it adds no RLS policy: every table it touches is already covered by
-- `connector_connections_org` (0185) and `plugin_installs_org` (0076), and
-- `cargo test -p core-db --test rls_coverage` still sees full coverage.
--
-- 1. Audience --------------------------------------------------------------

-- Who may SEE and ACT ON a connection. The empty array is the shipped default
-- and means "everyone the module permission already allows"; a non-empty array
-- is the closed list of org roles allowed to. Values are `roles.id::text`,
-- not role names: a renamed role keeps its id, and an audience that silently
-- stopped matching after a rename would be the failure mode this column must
-- not have. Enforced in `resolve_external_actions` (the resolver) and in the
-- roster route, not in RLS: RLS is org-level in this product, and a policy
-- encoding role membership would duplicate `core_db::client_access` in a
-- second language.
ALTER TABLE connector_connections
  ADD COLUMN audience_roles text[] NOT NULL DEFAULT '{}';

-- A cap, not a design constraint: an audience naming more roles than this is
-- "everyone" written the long way, and an unbounded array is a place to park
-- data the schema has no column for.
ALTER TABLE connector_connections
  ADD CONSTRAINT connector_connections_audience_roles_bounded
  CHECK (cardinality(audience_roles) <= 32);
-- No empty strings, so a malformed write cannot produce a member that matches
-- nothing and is indistinguishable from a role that was deleted.
ALTER TABLE connector_connections
  ADD CONSTRAINT connector_connections_audience_roles_not_blank
  CHECK (array_position(audience_roles, '') IS NULL);

COMMENT ON COLUMN connector_connections.audience_roles IS
  'Roles allowed to see and act on this connection, as roles.id::text. Empty '
  'means every caller the module permission already allows. Enforced by the '
  'resolver and the roster, never by RLS.';

-- 2. The install proves its own kind ---------------------------------------

-- `plugin_installs` joins `plugin_versions` to learn what it installed. When
-- that version row is gone — an upgrade that moved the pin, a catalogue revoke,
-- a restored backup — an `http_connector` install disappears from the roster
-- silently, which reads to an operator as "the connector uninstalled itself".
--
-- Denormalising the kind onto the install makes the install able to answer the
-- one question the roster asks without the join. Nullable on purpose: an install
-- whose version row is already gone cannot prove a kind, and inventing
-- `wasm` would be worse than saying nothing. The roster falls back to the pinned
-- manifest when this is NULL, so both directions stay honest.
ALTER TABLE plugin_installs ADD COLUMN artifact_kind text;

ALTER TABLE plugin_installs
  ADD CONSTRAINT plugin_installs_artifact_kind_known
  CHECK (artifact_kind IS NULL OR artifact_kind IN ('wasm', 'http_connector'));

COMMENT ON COLUMN plugin_installs.artifact_kind IS
  'The pinned manifest''s artifact_kind, denormalised at install time so an '
  'install whose plugin_versions row is missing can still prove what it is. '
  'NULL means unknown — the roster falls back to the pinned manifest.';

-- Backfill from the pinned manifest. A version row whose manifest carries no
-- artifact_kind is a wasm install (the field defaults to Wasm at parse time),
-- so that case lands as 'wasm' rather than NULL: the manifest DID prove it.
UPDATE plugin_installs i
   SET artifact_kind = COALESCE(v.manifest->>'artifact_kind', 'wasm')
  FROM plugin_versions v
 WHERE v.catalog_id = i.catalog_id
   AND v.version = i.version;

-- Keep it populated for every future install, including installs written by
-- paths that do not know this column exists. A trigger rather than a column
-- default because the value lives in another row: a default cannot read it, and
-- duplicating the lookup in `create_install` would leave the next writer to
-- remember. When the version row is written after the install (fixtures and
-- demo seeding do that), this leaves NULL and the roster's fallback covers it —
-- the same state as an install that never had a version row.
CREATE FUNCTION plugin_installs_stamp_artifact_kind() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  SELECT COALESCE(v.manifest->>'artifact_kind', 'wasm')
    INTO NEW.artifact_kind
    FROM plugin_versions v
   WHERE v.catalog_id = NEW.catalog_id
     AND v.version = NEW.version;
  RETURN NEW;
END;
$$;

CREATE TRIGGER plugin_installs_stamp_artifact_kind
  BEFORE INSERT ON plugin_installs FOR EACH ROW
  EXECUTE FUNCTION plugin_installs_stamp_artifact_kind();

-- No index on `artifact_kind`. The roster filters on
-- `COALESCE(pinned manifest, install column)` because the manifest is the
-- truth whenever it is still there, and no index can serve a COALESCE. The
-- scan it replaces is the org's own install list, which is dozens of rows —
-- the same list the roster already reads.
