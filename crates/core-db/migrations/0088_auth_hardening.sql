-- Wave 19 lane 5 — p19-auth-hardening (docs/reference/auth-threat-model.md §A).
-- Lane-exclusive migration pair 0088 (PG port 54391 is the branch-local
-- test port). ADDITIVE ONLY: no existing column, table, index, or policy
-- changes shape; RLS posture unchanged (new tables carry one-policy-per-org
-- shape exactly like 0072). U9-customization cross-check: none of the
-- constructs referenced by customization rows are touched.
--
-- Artifacts:
--
-- (1) Durable rate limiting + account lockout (POST /api/auth/login,
--     /api/auth/mfa/verify, /api/auth/mfa/webauthn/begin + /finish,
--     /api/auth/refresh).
--     `auth_attempt_log` records every THROTTLED-surface failed attempt
--     (durable evidence + per-IP/per-actor counting). `auth_lockouts`
--     carries the admitted-attempt counters with exponential backoff —
--     `locked_until = now + backoff(attempts)` grows as failures
--     accumulate, auto-unlocks when the deadline passes (time-driven, no
--     cron), and is cleared on success. Keys are (kind, lock_key) where
--     lock_key is either `email:<lowered>` (per-actor) or `ip:<hash>`
--     (per-IP; sha256 of client IP + server secret — not enumerable).
--     org_id is NULL for pre-auth rows written before the org resolves;
--     the RLS policy admits them both ways. All access from the
--     unauthenticated path rides SECURITY DEFINER bridges (§4) — the
--     application role NEVER reads these tables pre-auth.
--
-- (2) Password reset tokens: single-use, HASHED (only sha256 stored —
--     parity with refresh sessions), 30-minute expiry. Redemption is
--     the atomic bridge `app_consume_lockout_email_token` (used_at
--     guard ⇒ a replayed link is dead). Reset revokes ALL of the
--     user's refresh sessions ('<user>-all' convention in core-api).
--
-- (3) Email verification: `users.email_verified` + `email_verified_at`
--     (NULL default preserves every existing row's semantics; the deny
--     gate is OPT-IN per org via `orgs.require_email_verification`,
--     also deny-by-default=false). Verification tokens share the
--     single-use hashed-token table (kind discriminates).
--
-- (4) Magic-link sign-in: tokens share the same single-use hashed-token
--     table; org opt-in via `orgs.magic_link_sign_in` (deny-by-default
--     false). Redemption bridge returns the same narrow (user, org) as
--     `app_resolve_login`; issuance and sessions stay in core (§C).
--
-- (5) `auth_one_use_tokens` — ONE table for password-reset + email-
--     verification + magic-link tokens (kind discriminator): identical
--     semantics (hash-only storage, expiring, single-use, audited) would
--     be three copies of the same rows otherwise.
--
-- Rollback-safety: token rows are transient security work product;
-- lockouts/attempts are counters (dropping them only resets brute-force
-- throttles); the org/user columns are NULL/false for all existing rows.

-- ---------------------------------------------------------------------------
-- (1)+(2)+(5) auth lockouts + attempts + one-use tokens
-- ---------------------------------------------------------------------------

CREATE TABLE auth_attempt_log (
  id uuid PRIMARY KEY,
  -- NULL for pre-auth rows (org not yet resolved).
  org_id uuid REFERENCES orgs(id) ON DELETE CASCADE,
  -- Throttled surface: 'password_login' | 'mfa_verify' | 'webauthn_login'
  -- | 'webauthn_begin' | 'refresh' | 'password_reset' | 'email_verify'
  -- | 'magic_link'.
  kind text NOT NULL,
  -- Per-actor key: 'email:<lowered email>' when known.
  subject_key text,
  -- Per-IP key: 'ip:<sha256 hex>' (server-salt).
  ip_key text,
  occurred_at timestamptz NOT NULL DEFAULT now()
);
-- Throttle counting directions (per-actor and per-IP).
CREATE INDEX idx_auth_attempts_subject
  ON auth_attempt_log (kind, subject_key, occurred_at);
CREATE INDEX idx_auth_attempts_ip
  ON auth_attempt_log (kind, ip_key, occurred_at);
-- Retention: housekeeping purges >1d rows opportunistically.
CREATE INDEX idx_auth_attempts_retention
  ON auth_attempt_log (occurred_at);

CREATE TABLE auth_lockouts (
  -- 'password_login' | 'mfa_verify' | 'webauthn_login' | 'webauthn_begin'
  -- | 'refresh' | 'password_reset' | 'email_verify' | 'magic_link'
  kind text NOT NULL,
  -- lock_key: 'email:<lowered>' or 'ip:<hash>' (per-actor / per-IP).
  lock_key text NOT NULL,
  org_id uuid REFERENCES orgs(id) ON DELETE CASCADE,
  attempts int NOT NULL DEFAULT 0,
  locked_until timestamptz NOT NULL DEFAULT now(),
  last_failure_at timestamptz NOT NULL DEFAULT now(),
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (kind, lock_key)
);
-- Operator triage surface (which accounts/ips are locked out right now).
-- Plain-column index (time predicates would need an IMMUTABLE func).
CREATE INDEX idx_auth_lockouts_active ON auth_lockouts (locked_until);

CREATE TABLE auth_one_use_tokens (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  -- 'password_reset' | 'email_verification' | 'magic_link'
  kind text NOT NULL,
  -- sha256 hex — the raw token NEVER persists (refresh-session parity).
  token_hash text NOT NULL,
  expires_at timestamptz NOT NULL,
  used_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  created_by_bridge boolean NOT NULL DEFAULT false,
  UNIQUE (token_hash)
);
CREATE INDEX idx_auth_one_use_user
  ON auth_one_use_tokens (user_id, kind);
-- Retention: consumed/expired rows purge opportunistically in the bridge.
CREATE INDEX idx_auth_one_use_retention
  ON auth_one_use_tokens (expires_at);

ALTER TABLE auth_attempt_log ENABLE ROW LEVEL SECURITY;
CREATE POLICY auth_attempt_log_org
  ON auth_attempt_log
  USING (
    org_id IS NULL
    OR org_id = NULLIF(current_setting('app.org_id', true), '')::uuid
  );
ALTER TABLE auth_lockouts ENABLE ROW LEVEL SECURITY;
CREATE POLICY auth_lockouts_org
  ON auth_lockouts
  USING (
    org_id IS NULL
    OR org_id = NULLIF(current_setting('app.org_id', true), '')::uuid
  );
ALTER TABLE auth_one_use_tokens ENABLE ROW LEVEL SECURITY;
CREATE POLICY auth_one_use_tokens_org
  ON auth_one_use_tokens
  USING (org_id = NULLIF(current_setting('app.org_id', true), '')::uuid);

-- ---------------------------------------------------------------------------
-- (2)(3)(4) org/user opt-in columns (deny-by-default: both default FALSE)
-- ---------------------------------------------------------------------------

-- Magic-link sign-in: per-org opt-in. FALSE for every existing org ⇒
-- the flow is OFF until an admin turns it on.
ALTER TABLE orgs ADD COLUMN magic_link_sign_in boolean NOT NULL DEFAULT false;
COMMENT ON COLUMN orgs.magic_link_sign_in IS
  'Wave 19 lane 5: magic-link email sign-in allowed for this org (deny-by-default).';

-- Require verified email before login: per-org opt-in. FALSE for every
-- existing org ⇒ nothing is gated until an admin turns it on; when ON,
-- unverified accounts are denied at login (documented in the threat
-- model; admin override available via /api/org/users).
ALTER TABLE orgs ADD COLUMN require_email_verification boolean NOT NULL DEFAULT false;
COMMENT ON COLUMN orgs.require_email_verification IS
  'Wave 19 lane 5: login denied until users.email_verified is true (deny-by-default).';

ALTER TABLE users ADD COLUMN email_verified boolean NOT NULL DEFAULT false;
ALTER TABLE users ADD COLUMN email_verified_at timestamptz;
COMMENT ON COLUMN users.email_verified IS
  'Wave 19 lane 5: proven email control (verified link, OIDC verified-email claim, or admin override).';

-- ---------------------------------------------------------------------------
-- (6) Pre-auth SECURITY DEFINER bridges (narrow; owner-bypasses RLS)
-- ---------------------------------------------------------------------------

-- Exponential backoff lockout clock (pure; deterministic so tests pin it):
-- 30s after the 1st failure (brute-force window before any lock), 60s
-- after the 2nd, doubling per attempt, capped at 1h.
CREATE OR REPLACE FUNCTION app_auth_backoff_secs(p_attempts int)
RETURNS bigint
LANGUAGE sql
IMMUTABLE
AS $$
  SELECT LEAST(30::bigint * (1::bigint << LEAST(GREATEST(p_attempts - 1, 0), 7)), 3600)
$$;

-- Returns seconds the caller must wait (0 = allowed, lock expired or
-- never engaged). Auto-unlock: a locked_until in the past satisfies
-- every future check (time-driven) without any cleanup step.
CREATE OR REPLACE FUNCTION app_auth_gate_check(p_kind text, p_lock_keys text[])
RETURNS bigint
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
STABLE
AS $$
  SELECT COALESCE(
    MAX(EXTRACT(EPOCH FROM (locked_until - clock_timestamp()))::bigint),
    0
  )
  FROM auth_lockouts
  WHERE kind = p_kind
    AND lock_key = ANY (p_lock_keys)
    AND locked_until > clock_timestamp()
$$;

-- Records one failure: bumps the admitted-attempt counters for EVERY key
-- in the list (per-actor AND per-IP lock independently of each other),
-- writes the durable attempt-log row, and returns the new lockout
-- remaining seconds (0 while merely counting, >0 once locked). Row-less
-- org: org_id NULL (pre-auth). The late window purge keeps the log bounded.
CREATE OR REPLACE FUNCTION app_auth_gate_fail(
  p_kind text, p_lock_keys text[], p_org_id uuid, p_subject text, p_ip text
)
RETURNS bigint
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public
AS $$
DECLARE
  v_now timestamptz := clock_timestamp();
BEGIN
  -- Durable evidence row (subject_key/ip_key pre-shaped by the caller).
  INSERT INTO auth_attempt_log (id, org_id, kind, subject_key, ip_key)
  VALUES (
    gen_random_uuid(), p_org_id, p_kind,
    p_subject, p_ip
  );

  -- Opportunistic retention purge (>1 day), same posture as note_drafts.
  DELETE FROM auth_attempt_log WHERE occurred_at < v_now - interval '1 day';

  -- Bump EVERY key (per-actor AND per-IP lock independently of each
  -- other). One statement: insert-new or increment-existing, atomically.
  INSERT INTO auth_lockouts (kind, lock_key, org_id, attempts, locked_until, last_failure_at)
  SELECT p_kind, k, p_org_id, 1, v_now, v_now FROM unnest(p_lock_keys) AS k
  ON CONFLICT (kind, lock_key) DO UPDATE SET
    attempts = auth_lockouts.attempts + 1,
    last_failure_at = v_now,
    org_id = COALESCE(auth_lockouts.org_id, EXCLUDED.org_id);

  -- Recompute locked_until from the NEW counter so backoff doubles; the
  -- last_failure_at = v_now predicate limits the update to THIS call's
  -- keys so a concurrent failure isn't clobbered mid-window.
  UPDATE auth_lockouts
    SET locked_until = v_now + make_interval(secs => app_auth_backoff_secs(attempts))
    WHERE kind = p_kind AND lock_key = ANY (p_lock_keys) AND last_failure_at = v_now;

  RETURN app_auth_gate_check(p_kind, p_lock_keys);
END;
$$;

-- Cleared on success.
CREATE OR REPLACE FUNCTION app_auth_gate_clear(p_kind text, p_lock_keys text[])
RETURNS void
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
AS $$
  DELETE FROM auth_lockouts
  WHERE kind = p_kind AND lock_key = ANY (p_lock_keys)
$$;

-- Narrow email→(user, org) resolution for the PRE-AUTH flows that must
-- learn whether a mailbox exists WITHOUT any org scope (uniform
-- success-shaped answers outside stay the caller's discipline).
CREATE OR REPLACE FUNCTION app_resolve_auth_email(p_email text, p_kind text)
RETURNS TABLE (user_id uuid, org_id uuid, email_verified boolean)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
STABLE
AS $$
  SELECT u.id, u.org_id, u.email_verified
  FROM users u
  JOIN orgs o ON o.id = u.org_id
  WHERE lower(u.email) = lower(p_email)
    AND u.is_active
    -- 'password_reset' needs any active account; 'magic_link' also
    -- requires the org-level opt-in (deny-by-default in SQL).
    AND (p_kind <> 'magic_link' OR o.magic_link_sign_in)
  LIMIT 1
$$;

-- Consume a one-use token: the used_at guard IS the single-use
-- enforcement — two redemptions of one token give exactly one RETURNING
-- row; the loser sees nothing.
CREATE OR REPLACE FUNCTION app_consume_one_use_token(
  p_kind text, p_token_hash text
)
RETURNS TABLE (user_id uuid, org_id uuid)
SECURITY DEFINER
LANGUAGE plpgsql
SET search_path = public
AS $$
DECLARE
  v_row auth_one_use_tokens%ROWTYPE;
BEGIN
  WITH claimed AS (
    UPDATE auth_one_use_tokens
    SET used_at = clock_timestamp()
    WHERE kind = p_kind
      AND token_hash = p_token_hash
      AND used_at IS NULL
      AND expires_at > clock_timestamp()
    RETURNING *
  )
  SELECT * INTO v_row FROM claimed;
  IF NOT FOUND THEN
    RETURN;
  END IF;
  -- Opportunistic retention purge (consumed >30d / expired >7d).
  DELETE FROM auth_one_use_tokens
   WHERE (used_at IS NOT NULL AND used_at < clock_timestamp() - interval '30 days')
      OR (expires_at < clock_timestamp() - interval '7 days');
  RETURN QUERY SELECT v_row.user_id, v_row.org_id;
END;
$$;

REVOKE ALL ON FUNCTION app_auth_gate_check(text, text[]) FROM PUBLIC;
REVOKE ALL ON FUNCTION app_auth_gate_fail(text, text[], uuid, text, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION app_auth_gate_clear(text, text[]) FROM PUBLIC;
REVOKE ALL ON FUNCTION app_resolve_auth_email(text, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION app_consume_one_use_token(text, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION
  app_auth_gate_check(text, text[]),
  app_auth_gate_fail(text, text[], uuid, text, text),
  app_auth_gate_clear(text, text[]),
  app_resolve_auth_email(text, text),
  app_consume_one_use_token(text, text)
TO meridian_app;
