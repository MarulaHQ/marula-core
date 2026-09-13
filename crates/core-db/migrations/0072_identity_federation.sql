-- Wave 13 lane 7 (p13-identity-federation) — ID1–ID4.
--
-- Adds: (ID1) a `preset` column on oidc_providers naming the declarative
-- first-party preset (google | entra | okta) the provider was created
-- from — issuer/scopes/claim-mapping stay DATA in the preset registry,
-- never code forks; (ID2) edge-assertion auth sources (Cloudflare Access
-- pattern: signed `Cf-Access-Jwt-Assertion` validated against the team
-- JWKS with issuer/audience PINNED in config) and their conservative
-- link rows; (ID3) the federation link table shared by non-OIDC bundle
-- sources.
--
-- Non-purpose: no new session machinery. Federated logins resolve into
-- the SAME auth_sessions/JWT refresh-chain model as password logins
-- (issue_tokens); bundles only ever ADD an identity source.
--
-- Invariants encoded here:
--   * deny-by-default: edge sources are created DISABLED and only an
--     admin can enable them; the pre-auth resolver returns only
--     ENABLED sources, so an unknown or disabled source is uniformly
--     rejected.
--   * no JIT for edge-assertion sources: the edge proxy's signed JWT
--     carries no `email_verified` claim, so conservative linking has
--     nothing verified to JIT from — links are admin-created only.
--   * verified-email linking + org MFA policy are enforced in
--     core-api (shared decision function + login gate), not in SQL.
--
-- RLS doctrine: both new tables carry org_id + an isolation policy.
-- The SECURITY DEFINER bridge `app_resolve_edge_source` widens ONLY
-- the pre-authentication source resolution (name + enabled), mirroring
-- `app_resolve_oidc_provider`.

-- ---------------------------------------------------------------------------
-- ID1: first-party OIDC presets — declarative config marker.
-- ---------------------------------------------------------------------------
ALTER TABLE oidc_providers ADD COLUMN preset text;

-- ---------------------------------------------------------------------------
-- ID2: edge-assertion auth sources (Cloudflare Access pattern).
-- issuer/audience are PINNED values checked byte-for-byte against the
-- assertion; jwks_url is the team JWKS the assertion must verify
-- against. Raw header trust without signature verification is never
-- valid, so there is no "skip verification" mode — the table has no
-- column for one.
-- ---------------------------------------------------------------------------
CREATE TABLE edge_assertion_configs (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  display_name text NOT NULL,
  issuer text NOT NULL,
  audience text NOT NULL,
  jwks_url text NOT NULL,
  -- Deny-by-default: a new source is DISABLED until an admin
  -- explicitly enables it.
  enabled boolean NOT NULL DEFAULT false,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, name)
);

-- ---------------------------------------------------------------------------
-- ID3: conservative link rows for federation bundle sources that are
-- not OIDC providers (edge assertion today; future bundles reuse this
-- shape via the versioned bundle interface). OIDC keeps its
-- `oidc_accounts` rows; the (source_kind, source_id) pair namespaces
-- the rest. UNIQUE constraint name is consumed by the typed
-- reason-code registry: `federation_link_taken`.
-- ---------------------------------------------------------------------------
CREATE TABLE federation_links (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  source_kind text NOT NULL CHECK (source_kind IN ('edge_assertion')),
  source_id uuid NOT NULL,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  subject text NOT NULL,
  email text NOT NULL,
  linked_by uuid REFERENCES users(id) ON DELETE SET NULL,
  linked_at timestamptz NOT NULL DEFAULT now(),
  last_login_at timestamptz,
  UNIQUE (source_kind, source_id, subject)
);
CREATE INDEX idx_federation_links_user ON federation_links (user_id);

-- ---------------------------------------------------------------------------
-- Pre-auth bridges (SECURITY DEFINER; narrow columns only)
-- ---------------------------------------------------------------------------

-- Resolves an ENABLED edge source across orgs BEFORE RLS scope exists
-- (the exchange endpoint is unauthenticated). Disabled or unknown
-- names return no row — the caller answers with a uniform rejection
-- that does not distinguish the two (no source enumeration).
CREATE OR REPLACE FUNCTION app_resolve_edge_source(p_name text)
RETURNS TABLE (source_id uuid, org_id uuid, issuer text, audience text, jwks_url text)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
STABLE
AS $$
  SELECT e.id, e.org_id, e.issuer, e.audience, e.jwks_url
  FROM edge_assertion_configs e
  WHERE e.name = p_name AND e.enabled
  ORDER BY e.created_at
  LIMIT 1
$$;

REVOKE ALL ON FUNCTION app_resolve_edge_source(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION app_resolve_edge_source(text) TO meridian_app;

-- ---------------------------------------------------------------------------
-- Row level security
-- ---------------------------------------------------------------------------
ALTER TABLE edge_assertion_configs ENABLE ROW LEVEL SECURITY;
CREATE POLICY edge_assertion_configs_org ON edge_assertion_configs
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE federation_links ENABLE ROW LEVEL SECURITY;
CREATE POLICY federation_links_org ON federation_links
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- ---------------------------------------------------------------------------
-- App-role privileges (RLS still constrains rows)
-- ---------------------------------------------------------------------------
GRANT SELECT, INSERT, UPDATE, DELETE ON edge_assertion_configs,
  federation_links TO meridian_app;
