-- Login happens before the org is known, but `users` is RLS-scoped by org.
-- These SECURITY DEFINER functions are the two narrow bridges that resolve
-- (user_id, org_id) BEFORE a transaction can be bound to an org:
--   * app_resolve_login     — email → (user_id, org_id) for the login form
--   * app_resolve_session   — session id + token hash → (user_id, org_id)
--                             for refresh flows
-- Neither exposes password hashes or tokens. After resolution the caller
-- binds the transaction via `app.org_id` and all further queries run under
-- normal RLS.

CREATE OR REPLACE FUNCTION app_resolve_login(p_email text)
RETURNS TABLE (user_id uuid, org_id uuid)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
STABLE
AS $$
  SELECT u.id, u.org_id
  FROM users u
  WHERE lower(u.email) = lower(p_email) AND u.is_active
  LIMIT 1
$$;

CREATE OR REPLACE FUNCTION app_resolve_session(p_session_id uuid, p_token_hash text)
RETURNS TABLE (user_id uuid, org_id uuid)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
STABLE
AS $$
  SELECT s.user_id, u.org_id
  FROM auth_sessions s
  JOIN users u ON u.id = s.user_id
  WHERE s.id = p_session_id
    AND s.refresh_token_hash = p_token_hash
    AND s.revoked_at IS NULL
    AND s.expires_at > now()
    AND u.is_active
  LIMIT 1
$$;

REVOKE ALL ON FUNCTION app_resolve_login(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION app_resolve_session(uuid, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION app_resolve_login(text) TO meridian_app;
GRANT EXECUTE ON FUNCTION app_resolve_session(uuid, text) TO meridian_app;
