-- Org user management (Wave 11 lane 2; migration 0047).
--
-- Purpose: staff-user directory + lifecycle management endpoints
--   * invite/create: a user row is created INACTIVE until the invitee sets
--     a first password through a one-time setup token (argon2 hash reuse;
--     token stored hashed, single-use, expiring).
--   * activate/deactivate: deactivation revokes live sessions and blocks
--     login (users.is_active is already the login gate). Users are never
--     deleted.
--
-- The invite redemption happens on the RAW pool before any RLS scope is
-- bound (the invitee has no session yet), so resolution of the invite by
-- token hash goes through a narrow SECURITY DEFINER bridge, mirroring
-- 0010's `app_resolve_login` / 0046's `app_create_session`. The bridge
-- returns only org/user ids — never the token hash, never credentials.

CREATE TABLE user_invites (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  -- sha-256 of the one-time setup token; the plaintext token lives only in
  -- the invite API response the admin delivers out-of-band.
  token_hash text NOT NULL UNIQUE,
  created_by uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  expires_at timestamptz NOT NULL,
  used_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX idx_user_invites_org ON user_invites (org_id);
-- Redemption lookup by token goes through the definer bridge below; the
-- index keeps that probe cheap.
CREATE INDEX idx_user_invites_user ON user_invites (user_id);

ALTER TABLE user_invites ENABLE ROW LEVEL SECURITY;
CREATE POLICY user_invites_org ON user_invites
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

CREATE OR REPLACE FUNCTION app_resolve_user_invite(p_token_hash text)
RETURNS TABLE (org_id uuid, user_id uuid)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
AS $$
  SELECT org_id, user_id FROM user_invites
  WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now()
$$;

REVOKE ALL ON FUNCTION app_resolve_user_invite(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION app_resolve_user_invite(text) TO meridian_app;
