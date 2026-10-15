-- 0132: pre-auth SECURITY DEFINER bridges for the calendar-sync surfaces.
--
-- WHY. Two calendar-sync surfaces resolve a `calendar_sync_connections` row
-- BEFORE any org is known, because the nonce (OAuth state) or the feed token IS
-- the capability that names the org:
--
--   * the delegated-OAuth callback (Google/Microsoft), which runs on the
--     vendor's browser redirect with no bearer and no session, and
--   * the public outbound iCal feed, whose token is in the URL path.
--
-- `calendar_sync_connections` is RLS-scoped (0074). A read on the bare
-- application pool is therefore default-deny, so both surfaces were silently
-- broken under RLS: the callback always answered "this authorization link is
-- unknown or has expired" (the nonce was written correctly by 42699760, but
-- nothing could consume it), and the feed returned 404 for a live token.
--
-- Neither read can ride an org-bound transaction, because the org is what the
-- read produces. So both resolve through the same narrow, owner-bypassing
-- SECURITY DEFINER seam every other pre-auth token surface uses
-- (app_resolve_login 0010, portal_resolve_login / portal_resolve_session 0021,
-- app_consume_portal_one_use_token 0122): identity only, never token material,
-- and every later read rides an org-bound tx bound to the returned org.

-- ---------------------------------------------------------------------------
-- Consume the OAuth state nonce, atomically. The UPDATE that clears the nonce
-- IS the claim, so a replayed callback finds nothing (doing it in the consuming
-- statement rather than a separate delete is what closes the race window). The
-- expiry predicate is part of the claim, so a stale nonce cannot complete a
-- consent the operator started a decision ago. Returns identity only.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION app_consume_calendar_oauth_state(
  p_provider text, p_nonce text
)
RETURNS TABLE (connection_id uuid, org_id uuid, user_id uuid)
SECURITY DEFINER
LANGUAGE plpgsql
SET search_path = public
AS $$
DECLARE
  v_row calendar_sync_connections%ROWTYPE;
BEGIN
  WITH claimed AS (
    UPDATE calendar_sync_connections c
    SET oauth_state = NULL,
        oauth_state_expires_at = NULL,
        updated_at = now()
    WHERE c.provider = p_provider
      AND c.oauth_state = p_nonce
      AND c.oauth_state_expires_at > now()
    RETURNING c.*
  )
  SELECT * INTO v_row FROM claimed;
  IF NOT FOUND THEN
    RETURN;
  END IF;
  RETURN QUERY SELECT v_row.id, v_row.org_id, v_row.user_id;
END;
$$;

REVOKE ALL ON FUNCTION app_consume_calendar_oauth_state(text, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION app_consume_calendar_oauth_state(text, text) TO marula_app;

-- ---------------------------------------------------------------------------
-- Resolve an outbound feed token to its connection. Read-only. The caller binds
-- app.org_id to the returned org before every subsequent read (the feed's
-- appointments, the connection itself), so this bridge only ever hands back the
-- pair the token already proves.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION app_resolve_calendar_feed_target(p_token_hash text)
RETURNS TABLE (org_id uuid, connection_id uuid)
SECURITY DEFINER
LANGUAGE sql
SET search_path = public
STABLE
AS $$
  SELECT c.org_id, c.id
  FROM calendar_sync_connections c
  WHERE c.feed_token_hash = p_token_hash
    AND c.status <> 'disabled'
  LIMIT 1
$$;

REVOKE ALL ON FUNCTION app_resolve_calendar_feed_target(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION app_resolve_calendar_feed_target(text) TO marula_app;
