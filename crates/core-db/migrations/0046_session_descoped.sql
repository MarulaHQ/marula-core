-- Session creation happens on the raw pool AFTER the auth transaction
-- commits, when no RLS scope (`app.org_id`) is bound. The 0001
-- `auth_sessions_org` policy uses `current_setting('app.org_id', true)::uuid`
-- as its WITH CHECK, so an unscoped INSERT casts "" to uuid and dies with
-- 22P02 (`invalid input syntax for type uuid: ""`). Register + login have
-- failed 500 on a fresh RLS database for that reason. These two narrow
-- bridges (mirroring 0010's `app_resolve_login`) insert session rows as the
-- definer (owner) so no RLS scope is required at the call site; the caller
-- still passes the real `org_id`, and the session is chain-rooted per 0013.
-- Neither function accepts or returns credentials.

CREATE OR REPLACE FUNCTION app_create_session(
  p_session_id uuid, p_user_id uuid, p_refresh_hash text,
  p_user_agent text, p_expires_at timestamptz, p_chain_id uuid
)
RETURNS TABLE (id uuid, user_id uuid, refresh_token_hash text, user_agent text,
               expires_at timestamptz, chain_id uuid, created_at timestamptz, revoked_at timestamptz)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
AS $$
  INSERT INTO auth_sessions
    (id, user_id, refresh_token_hash, user_agent, expires_at, chain_id)
  VALUES
    ($1, $2, $3, $4, $5, $6)
  RETURNING id, user_id, refresh_token_hash, user_agent, expires_at, chain_id,
            created_at, revoked_at
$$;

CREATE OR REPLACE FUNCTION app_create_portal_session(
  p_session_id uuid, p_org_id uuid, p_portal_user_id uuid,
  p_refresh_hash text, p_user_agent text, p_expires_at timestamptz
)
RETURNS void
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
AS $$
  INSERT INTO portal_sessions
    (id, org_id, portal_user_id, refresh_token_hash, user_agent, expires_at)
  VALUES
    ($1, $2, $3, $4, $5, $6)
$$;

REVOKE ALL ON FUNCTION app_create_session(uuid, uuid, text, text, timestamptz, uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION app_create_portal_session(uuid, uuid, uuid, text, text, timestamptz) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION app_create_session(uuid, uuid, text, text, timestamptz, uuid) TO meridian_app;
GRANT EXECUTE ON FUNCTION app_create_portal_session(uuid, uuid, uuid, text, text, timestamptz) TO meridian_app;