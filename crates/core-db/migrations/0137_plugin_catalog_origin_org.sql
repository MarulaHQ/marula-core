-- Bind each registry entry to the org that published it.
--
-- `plugin_catalog` is server-wide by design: no org_id, no RLS, one row per
-- slug that every tenant installs from. That made a slug the single handle for
-- a shared resource while the mutation routes were authorized only against the
-- caller's own org, so one tenant's admin could append a version to another
-- publisher's entry or revoke an entry every tenant had installed.
--
-- `origin_org_id` records the publishing org. NULL means an entry authored by
-- the platform operator (seeded or curated), which no tenant's admin may
-- mutate through the tenant catalog routes. ON DELETE SET NULL is deliberate:
-- a departing org must not cascade-delete a shared catalog entry that other
-- tenants still install from; the row reverts to operator ownership instead.
ALTER TABLE plugin_catalog
  ADD COLUMN origin_org_id uuid REFERENCES orgs (id) ON DELETE SET NULL;

CREATE INDEX idx_plugin_catalog_origin_org ON plugin_catalog (origin_org_id);
