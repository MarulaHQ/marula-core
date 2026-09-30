-- 0119: portal magic-link sign-in (Wave 41 lane 13).
--
-- WHY THIS TABLE EXISTS AT ALL, AND WHY IT IS NOT A COLUMN ON THE STAFF ONE.
-- `auth_one_use_tokens.user_id` is `NOT NULL REFERENCES users(id) ON DELETE
-- CASCADE` (migration 0088). A portal identity lives in `portal_users`, so a
-- portal_user_id cannot be written to the staff token table — the foreign key
-- rejects it, and loosening that FK to accommodate a second identity family
-- would give up the referential guarantee the staff rows depend on. So the
-- portal gets its own table with its own foreign key, mirroring 0088 exactly.
--
-- WHAT THE LINK IS. A single-use, expiring credential that lets a client
-- contact sign in to the PORTAL without a password. It resolves to a portal
-- identity and nothing else: consuming it issues portal tokens through the
-- same `app_create_portal_session` path the password login uses, so a portal
-- link can never mint a staff session, and a staff magic link can never mint a
-- portal session. The two families stay as separate as their guards.
--
-- DENY-BY-DEFAULT. The request endpoint answers `{ok:true}` for every input and
-- mints nothing unless the address resolves to a portal user in good standing,
-- so this table is only ever written for a real portal identity. There is no
-- org-level opt-in column on purpose: the staff flow is a LOGIN METHOD for
-- staff and is therefore gated per org (0088 `orgs.magic_link_sign_in`), while
-- a portal link is only useful to a provisioned contact, and a disabled
-- contact is refused at consume time (see below).

CREATE TABLE portal_auth_one_use_tokens (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- The PORTAL identity, not a staff user. This FK is the reason the table is
  -- separate (see header).
  portal_user_id uuid NOT NULL REFERENCES portal_users(id) ON DELETE CASCADE,
  -- 'portal_magic_link' today; the column is open like its staff twin's.
  kind text NOT NULL,
  -- sha256 hex. The raw token NEVER persists (refresh-session parity).
  token_hash text NOT NULL,
  expires_at timestamptz NOT NULL,
  used_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (token_hash)
);

CREATE INDEX idx_portal_auth_one_use_user
  ON portal_auth_one_use_tokens (portal_user_id, kind);
-- Retention sweep (see the consume bridge) walks expiry.
CREATE INDEX idx_portal_auth_one_use_retention
  ON portal_auth_one_use_tokens (expires_at);

ALTER TABLE portal_auth_one_use_tokens ENABLE ROW LEVEL SECURITY;
CREATE POLICY portal_auth_one_use_tokens_org
  ON portal_auth_one_use_tokens
  USING (org_id = NULLIF(current_setting('app.org_id', true), '')::uuid);

-- Explicit grants. 0114 already set ALTER DEFAULT PRIVILEGES for the
-- rebrand's `marula_app` role, but a table that carries credentials says so
-- out loud rather than relying on a default that a future migration could
-- change.
GRANT SELECT, INSERT, UPDATE, DELETE ON portal_auth_one_use_tokens TO marula_app;

-- ---------------------------------------------------------------------------
-- Pre-auth SECURITY DEFINER bridge (narrow; owner-bypasses RLS).
--
-- Claim-and-return in ONE statement, so a replayed link finds nothing: the
-- UPDATE that stamps `used_at` IS the claim, which is what makes the
-- single-use guarantee race-proof. The claim is also the status check — a
-- contact disabled AFTER the link was mailed cannot sign in with it, because
-- the UPDATE matches nothing and the link is simply invalid. `client_id` rides
-- out with the ids because every portal token needs it, and reading it
-- separately would be a second lookup against a table the caller has no org
-- scope for yet.
--
-- The request side needs no bridge of its own: `portal_resolve_login` (0021)
-- already resolves an address to (portal_user_id, org_id, client_id) and
-- already restricts itself to `status IN ('active','invited')`, which is
-- precisely the population a portal link may be minted for.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION app_consume_portal_one_use_token(
  p_kind text, p_token_hash text
)
RETURNS TABLE (portal_user_id uuid, org_id uuid, client_id uuid)
SECURITY DEFINER
LANGUAGE plpgsql
SET search_path = public
AS $$
DECLARE
  v_row portal_auth_one_use_tokens%ROWTYPE;
BEGIN
  WITH claimed AS (
    UPDATE portal_auth_one_use_tokens t
    SET used_at = clock_timestamp()
    FROM portal_users pu
    WHERE t.kind = p_kind
      AND t.token_hash = p_token_hash
      AND t.used_at IS NULL
      AND t.expires_at > clock_timestamp()
      -- The claim carries the standing check, so there is no window between
      -- "is this identity allowed?" and "spend the token".
      AND pu.id = t.portal_user_id
      AND pu.org_id = t.org_id
      AND pu.status <> 'disabled'
    RETURNING t.*
  )
  SELECT * INTO v_row FROM claimed;
  IF NOT FOUND THEN
    RETURN;
  END IF;
  -- Opportunistic retention purge, same windows as the staff bridge
  -- (consumed >30d / expired >7d).
  DELETE FROM portal_auth_one_use_tokens
   WHERE (used_at IS NOT NULL AND used_at < clock_timestamp() - interval '30 days')
      OR (expires_at < clock_timestamp() - interval '7 days');
  RETURN QUERY
    SELECT v_row.portal_user_id, v_row.org_id, pu.client_id
      FROM portal_users pu
     WHERE pu.id = v_row.portal_user_id
       AND pu.org_id = v_row.org_id;
END;
$$;

REVOKE ALL ON FUNCTION app_consume_portal_one_use_token(text, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION app_consume_portal_one_use_token(text, text)
TO marula_app;
