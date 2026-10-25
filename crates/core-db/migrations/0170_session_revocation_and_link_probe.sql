-- The p57 sweep's three rowed findings, all in pre-auth or access-token auth.
--
-- 1. A revoked session kept its outstanding access token. `require_auth`
--    re-read the user and roles on every cache miss but never looked at
--    `auth_sessions.revoked_at`, so logout, an admin factor reset and reuse
--    detection killed the refresh chain while the token already issued kept
--    working for its full TTL (15 minutes).
--
--    A blanket `revoked_at IS NULL` check would break rotation: a refresh
--    retires the parent row and mints a child, and the parent's access token
--    is still in the client's hands until it expires. Refusing it would turn
--    every rotation into a burst of 401s. So the row now records WHY it was
--    retired, and only a deliberate end (logout, admin or reset revocation,
--    refresh-token reuse) refuses the token. Rotation does not.
--
-- 2. `app_peek_one_use_token` reads a single-use token WITHOUT consuming it.
--    The magic-link confirm needs the (user, org) pair to answer the MFA
--    gate, and consuming first meant an enrolled account spent a link to
--    learn it must use a password. The consume function stays the only
--    writer; the peek cannot redeem anything.
--
-- 3. `app_org_slug_taken` answers the registration suggestion probe. The
--    previous probe was a plain pool read of `orgs`, which is RLS-scoped, so
--    an unscoped pre-auth read saw no row and the suffix suggestion could
--    never fire. Slugs are public (they are in URLs), so a boolean probe is
--    not a disclosure.
--
-- Non-purpose: no route, response shape or privilege changes. The bridges are
-- narrow, STABLE, and return a boolean or the same (user_id, org_id) pair the
-- consume bridge already returns.

-- ---------------------------------------------------------------------------
-- 1. Why a session was retired
-- ---------------------------------------------------------------------------
ALTER TABLE auth_sessions ADD COLUMN IF NOT EXISTS revoked_reason text;

COMMENT ON COLUMN auth_sessions.revoked_reason IS
  'Why revoked_at was set: rotated, reuse, or revoked. NULL on an active row, '
  'and on rows retired before this column existed — read as deliberate, the '
  'conservative answer. The access-token check refuses every reason except '
  'rotated, because a rotated parent''s access token is still in flight.';

CREATE OR REPLACE FUNCTION app_session_active(p_session_id uuid)
RETURNS boolean
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
STABLE
AS $$
  SELECT EXISTS (
    SELECT 1
    FROM auth_sessions s
    WHERE s.id = p_session_id
      AND s.expires_at > now()
      AND (s.revoked_at IS NULL OR s.revoked_reason = 'rotated')
  )
$$;

REVOKE ALL ON FUNCTION app_session_active(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION app_session_active(uuid) TO marula_app;

DO $$
BEGIN
  IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'meridian_app') THEN
    GRANT EXECUTE ON FUNCTION app_session_active(uuid) TO meridian_app;
  END IF;
END
$$;

-- ---------------------------------------------------------------------------
-- 2. Read a single-use token without redeeming it
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION app_peek_one_use_token(p_kind text, p_token_hash text)
RETURNS TABLE (user_id uuid, org_id uuid)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
STABLE
AS $$
  SELECT t.user_id, t.org_id
  FROM auth_one_use_tokens t
  WHERE t.kind = p_kind
    AND t.token_hash = p_token_hash
    AND t.used_at IS NULL
    AND t.expires_at > now()
  LIMIT 1
$$;

REVOKE ALL ON FUNCTION app_peek_one_use_token(text, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION app_peek_one_use_token(text, text) TO marula_app;

DO $$
BEGIN
  IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'meridian_app') THEN
    GRANT EXECUTE ON FUNCTION app_peek_one_use_token(text, text) TO meridian_app;
  END IF;
END
$$;

-- ---------------------------------------------------------------------------
-- 3. Is this slug held, seen from outside any tenant's scope
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION app_org_slug_taken(p_slug text)
RETURNS boolean
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
STABLE
AS $$
  SELECT EXISTS (SELECT 1 FROM orgs o WHERE o.slug = p_slug)
$$;

REVOKE ALL ON FUNCTION app_org_slug_taken(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION app_org_slug_taken(text) TO marula_app;

DO $$
BEGIN
  IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'meridian_app') THEN
    GRANT EXECUTE ON FUNCTION app_org_slug_taken(text) TO meridian_app;
  END IF;
END
$$;
