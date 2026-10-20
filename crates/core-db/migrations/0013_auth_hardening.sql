-- Stage 2 auth hardening (the private build tracker 2.3).
--
-- Adds: TOTP secrets (encrypted at rest), WebAuthn credentials + ceremony
-- state, single-use recovery codes (hashed), org-level generic OIDC
-- providers (secret encrypted at rest, JIT provisioning default OFF) with
-- conservative account linking, and refresh-token CHAIN tracking so a
-- replayed (already-rotated) refresh token revokes the whole lineage.
--
-- Non-purpose: no changes to existing tables beyond additive columns on
-- auth_sessions; users.mfa_totp_secret is legacy and stays untouched.
--
-- RLS doctrine: every new user-data table carries org_id + an isolation
-- policy. The SECURITY DEFINER bridge `app_resolve_refresh` widens the
-- session resolver to return revoked rows too — reuse detection must be
-- able to SEE a rotated token before org scope exists.

-- ---------------------------------------------------------------------------
-- TOTP (RFC 6238). Secret is AES-256-GCM ciphertext (12-byte nonce prefix),
-- key from app config; never returned after enrollment confirms.
-- ---------------------------------------------------------------------------
CREATE TABLE totp_secrets (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id uuid NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
  secret_enc bytea NOT NULL,
  confirmed boolean NOT NULL DEFAULT false,
  -- Highest consumed RFC 6238 timestep; codes at or below are rejected
  -- (replay protection within and across windows).
  last_used_step bigint NOT NULL DEFAULT -1,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

-- ---------------------------------------------------------------------------
-- WebAuthn / passkeys. `credential` holds the serialized webauthn-rs
-- Passkey (public key + counter); the private side never exists server-side.
-- ---------------------------------------------------------------------------
CREATE TABLE webauthn_credentials (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  cred_id text NOT NULL UNIQUE,
  credential jsonb NOT NULL,
  label text NOT NULL DEFAULT '',
  created_at timestamptz NOT NULL DEFAULT now(),
  last_used_at timestamptz
);
CREATE INDEX idx_webauthn_creds_user ON webauthn_credentials (user_id);

-- Short-lived ceremony state (registration/authentication challenges and
-- single-use SSO token-exchange codes), deleted on use and swept lazily
-- by expiry.
CREATE TABLE mfa_challenges (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  kind text NOT NULL CHECK (kind IN ('registration', 'authentication', 'sso_code')),
  state jsonb NOT NULL,
  expires_at timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_mfa_challenges_expiry ON mfa_challenges (expires_at);

-- ---------------------------------------------------------------------------
-- Recovery codes: generated once per enable/regenerate, stored hashed
-- (argon2id), single-use (used_at set atomically on consume).
-- ---------------------------------------------------------------------------
CREATE TABLE recovery_codes (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  code_hash text NOT NULL,
  used_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_recovery_codes_user_unused ON recovery_codes (user_id)
  WHERE used_at IS NULL;

-- ---------------------------------------------------------------------------
-- Generic OIDC SSO (org-level). Client secret encrypted at rest like all
-- integration credentials; never returned after write.
-- ---------------------------------------------------------------------------
CREATE TABLE oidc_providers (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  display_name text NOT NULL,
  issuer text NOT NULL,
  discovery_url text NOT NULL,
  client_id text NOT NULL,
  client_secret_enc bytea NOT NULL,
  jit_provisioning boolean NOT NULL DEFAULT false,
  enabled boolean NOT NULL DEFAULT true,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, name)
);

-- Conservative linking: a login succeeds only for an EXPLICITLY linked
-- (provider_id, subject) pair, unless JIT provisioning is enabled — and
-- even then only for a VERIFIED email claim.
CREATE TABLE oidc_accounts (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  provider_id uuid NOT NULL REFERENCES oidc_providers(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  subject text NOT NULL,
  email text NOT NULL,
  linked_by uuid REFERENCES users(id) ON DELETE SET NULL,
  linked_at timestamptz NOT NULL DEFAULT now(),
  last_login_at timestamptz,
  UNIQUE (provider_id, subject)
);
CREATE INDEX idx_oidc_accounts_user ON oidc_accounts (user_id);

-- ---------------------------------------------------------------------------
-- Refresh-token chains. Existing rows become their own chain root;
-- rotation inserts a child row (new secret) instead of mutating the old
-- one, so every presented token maps to exactly one immutable row.
-- ---------------------------------------------------------------------------
ALTER TABLE auth_sessions ADD COLUMN chain_id uuid;
UPDATE auth_sessions SET chain_id = id;
ALTER TABLE auth_sessions ALTER COLUMN chain_id SET NOT NULL;
ALTER TABLE auth_sessions ADD COLUMN rotated_from uuid REFERENCES auth_sessions(id) ON DELETE SET NULL;
CREATE INDEX idx_auth_sessions_chain ON auth_sessions (chain_id);

-- Widened bridge for the refresh endpoint: resolves the session's org even
-- when the row was already rotated/revoked (that exact case IS reuse), and
-- reports whether it is currently active.
CREATE OR REPLACE FUNCTION app_resolve_refresh(p_session_id uuid, p_token_hash text)
RETURNS TABLE (user_id uuid, org_id uuid, chain_id uuid, active boolean)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
STABLE
AS $$
  SELECT s.user_id, u.org_id, s.chain_id, (s.revoked_at IS NULL AND s.expires_at > now())
  FROM auth_sessions s
  JOIN users u ON u.id = s.user_id
  WHERE s.id = p_session_id
    AND s.refresh_token_hash = p_token_hash
    AND u.is_active
  LIMIT 1
$$;

REVOKE ALL ON FUNCTION app_resolve_refresh(uuid, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION app_resolve_refresh(uuid, text) TO meridian_app;

-- Public (unauthenticated) SSO discovery for the login screen: lists
-- enabled provider names without leaking any configuration detail.
CREATE OR REPLACE FUNCTION app_list_enabled_oidc_providers()
RETURNS TABLE (org_id uuid, name text, display_name text)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
STABLE
AS $$
  SELECT o.org_id, o.name, o.display_name
  FROM oidc_providers o
  WHERE o.enabled
  ORDER BY o.created_at
  LIMIT 20
$$;

REVOKE ALL ON FUNCTION app_list_enabled_oidc_providers() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION app_list_enabled_oidc_providers() TO meridian_app;

-- Resolves an enabled provider across orgs BEFORE RLS scope exists (the
-- SSO entry point is unauthenticated). Self-host deployments have one org;
-- ties resolve deterministically by creation order.
CREATE OR REPLACE FUNCTION app_resolve_oidc_provider(p_name text)
RETURNS TABLE (provider_id uuid, org_id uuid, issuer text, discovery_url text,
               client_id text, jit_provisioning boolean)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
STABLE
AS $$
  SELECT o.id, o.org_id, o.issuer, o.discovery_url, o.client_id, o.jit_provisioning
  FROM oidc_providers o
  WHERE o.name = p_name AND o.enabled
  ORDER BY o.created_at
  LIMIT 1
$$;

REVOKE ALL ON FUNCTION app_resolve_oidc_provider(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION app_resolve_oidc_provider(text) TO meridian_app;

-- SSO one-time handoff codes live in mfa_challenges (kind 'sso_code').
-- This bridge resolves the owning org BEFORE RLS scope exists so the
-- exchange endpoint can bind its transaction; expiry is enforced here.
CREATE OR REPLACE FUNCTION app_resolve_sso_code(p_code uuid)
RETURNS uuid
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
STABLE
AS $$
  SELECT c.org_id
  FROM mfa_challenges c
  WHERE c.id = p_code AND c.kind = 'sso_code' AND c.expires_at > now()
  LIMIT 1
$$;

REVOKE ALL ON FUNCTION app_resolve_sso_code(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION app_resolve_sso_code(uuid) TO meridian_app;

-- ---------------------------------------------------------------------------
-- Row level security
-- ---------------------------------------------------------------------------
ALTER TABLE totp_secrets ENABLE ROW LEVEL SECURITY;
CREATE POLICY totp_secrets_org ON totp_secrets
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE webauthn_credentials ENABLE ROW LEVEL SECURITY;
CREATE POLICY webauthn_credentials_org ON webauthn_credentials
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE mfa_challenges ENABLE ROW LEVEL SECURITY;
CREATE POLICY mfa_challenges_org ON mfa_challenges
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE recovery_codes ENABLE ROW LEVEL SECURITY;
CREATE POLICY recovery_codes_org ON recovery_codes
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE oidc_providers ENABLE ROW LEVEL SECURITY;
CREATE POLICY oidc_providers_org ON oidc_providers
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE oidc_accounts ENABLE ROW LEVEL SECURITY;
CREATE POLICY oidc_accounts_org ON oidc_accounts
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- ---------------------------------------------------------------------------
-- App-role privileges (RLS still constrains rows)
-- ---------------------------------------------------------------------------
GRANT SELECT, INSERT, UPDATE, DELETE ON totp_secrets,
  webauthn_credentials, mfa_challenges, recovery_codes,
  oidc_providers, oidc_accounts TO meridian_app;
