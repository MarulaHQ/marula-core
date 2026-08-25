-- Login happens before the org is known, but `users` is RLS-scoped by org.
-- This SECURITY DEFINER function is the single narrow bridge: it resolves
-- (user_id, org_id) for an email without exposing the password hash. The
-- caller then binds the transaction to that org via `app.org_id` and all
-- subsequent queries run under RLS as normal.

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

REVOKE ALL ON FUNCTION app_resolve_login(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION app_resolve_login(text) TO meridian_app;
