-- SSO names resolve before a tenant is known, so `name` must be unique
-- across the deployment, not per org (Wave 54 lane S3e,
-- `p54-preauth-name-scope`).
--
-- The two unauthenticated entry points are `GET /api/auth/oidc/{name}/login`
-- and `POST /api/auth/edge/{name}/exchange`. Neither request carries a
-- tenant: the path segment is the only input, and core-api never reads a
-- host header to pick an org (`orgs.custom_domain` is stored and exposed,
-- never consulted on a request path — 0135 states that the operator's edge
-- terminates TLS and routes, so Core itself has no host→org signal).
-- Both resolve through SECURITY DEFINER bridges (`app_resolve_oidc_provider`,
-- `app_resolve_edge_source`) that match `name` exactly and take
-- `ORDER BY created_at LIMIT 1`.
--
-- Both tables allowed the same name once per org (`UNIQUE (org_id, name)`).
-- Two orgs with a provider called `okta` therefore produced two rows and one
-- resolver answer: the older row. The second org's provider was unreachable
-- on its own login path, and the second org's users were sent to the first
-- org's issuer and client_id. The login screen lists names without an org
-- (`PublicProvider { name, display_name }`), so no UI could tell the rows
-- apart either.
--
-- The write paths validate the same character class on both surfaces
-- (lowercase alphanumeric and `-`), so a plain unique index on the exact
-- column matches the resolver's case-sensitive equality. Folding case with
-- `lower()` would forbid names the resolver never confused.
--
-- REPAIR, not just a guard: a deployment that already holds a shadowed row
-- cannot take the index, and that row is dead configuration — its org has
-- been authenticating against a different tenant's IdP, or not at all. The
-- block below renames each losing row to `<name>--<first 8 hex of its org
-- id>` (with a numeric tail if that is taken) so the org's provider becomes
-- reachable at a name nobody else holds. The winning row — the earliest
-- `created_at`, which is what both resolvers already picked — keeps its name.
-- Operators see one NOTICE per rename in the migration log. The order is
-- `created_at, id` so the rename is deterministic when two rows share a
-- timestamp; the resolvers have no such tiebreak, so for a same-timestamp
-- pair the winner is arbitrary either way and the rename fixes the ambiguity.
--
-- LOCK NOTE: `oidc_providers` and `edge_assertion_configs` are small
-- config tables (a handful of rows per org), read on the login path. The
-- index builds take ACCESS EXCLUSIVE on each table for the length of the
-- build, which is milliseconds at these sizes. Only migrations run this;
-- the login path already reads the tables.

DO $$
DECLARE
  loser record;
  candidate text;
  suffix text;
  n int;
BEGIN
  FOR loser IN
    SELECT id, org_id, name FROM (
      SELECT id, org_id, name,
             row_number() OVER (PARTITION BY name ORDER BY created_at, id) AS rn
      FROM oidc_providers
    ) ranked
    WHERE ranked.rn > 1
  LOOP
    suffix := left(replace(loser.org_id::text, '-', ''), 8);
    candidate := loser.name || '--' || suffix;
    n := 1;
    WHILE EXISTS (SELECT 1 FROM oidc_providers WHERE name = candidate) LOOP
      n := n + 1;
      candidate := loser.name || '--' || suffix || '-' || n;
    END LOOP;
    RAISE NOTICE 'oidc_providers: shadowed provider "%" (org %) renamed to "%"',
      loser.name, loser.org_id, candidate;
    UPDATE oidc_providers SET name = candidate, updated_at = now() WHERE id = loser.id;
  END LOOP;

  FOR loser IN
    SELECT id, org_id, name FROM (
      SELECT id, org_id, name,
             row_number() OVER (PARTITION BY name ORDER BY created_at, id) AS rn
      FROM edge_assertion_configs
    ) ranked
    WHERE ranked.rn > 1
  LOOP
    suffix := left(replace(loser.org_id::text, '-', ''), 8);
    candidate := loser.name || '--' || suffix;
    n := 1;
    WHILE EXISTS (SELECT 1 FROM edge_assertion_configs WHERE name = candidate) LOOP
      n := n + 1;
      candidate := loser.name || '--' || suffix || '-' || n;
    END LOOP;
    RAISE NOTICE 'edge_assertion_configs: shadowed source "%" (org %) renamed to "%"',
      loser.name, loser.org_id, candidate;
    UPDATE edge_assertion_configs SET name = candidate, updated_at = now() WHERE id = loser.id;
  END LOOP;
END $$;

-- Both constraint names are mapped to a typed 409 in core-api's
-- `map_unique_constraint`, so a second org claiming a name gets
-- `oidc_provider_name_taken` / `edge_source_name_taken` instead of the
-- sanitized generic conflict.
CREATE UNIQUE INDEX IF NOT EXISTS oidc_providers_global_name_key
  ON oidc_providers (name);

CREATE UNIQUE INDEX IF NOT EXISTS edge_assertion_configs_global_name_key
  ON edge_assertion_configs (name);

COMMENT ON INDEX oidc_providers_global_name_key IS
  'SSO names resolve before a tenant is known (app_resolve_oidc_provider), so a provider name is claimed deployment-wide.';

COMMENT ON INDEX edge_assertion_configs_global_name_key IS
  'Edge source names resolve before a tenant is known (app_resolve_edge_source), so a source name is claimed deployment-wide.';
