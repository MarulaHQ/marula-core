-- MFA step-up records and the admin factor reset (Wave 55, p55-mfa-step-up).
--
-- Why this exists. `totp_enroll_begin` used to overwrite a CONFIRMED TOTP
-- secret with a pending one (`INSERT ... ON CONFLICT DO UPDATE SET
-- confirmed = false`), so anyone holding a session could replace the account's
-- second factor with their own. `webauthn_credential_remove` asked for the
-- password and nothing else, and the audit row did not say who authorized the
-- removal. Both are the same defect: a destructive factor change that proves
-- only "this browser is signed in".
--
-- The rule (recorded in `qa-p54-security-decisions/DECISIONS.md`, records 1
-- and 2, and accepted 2026-10-25) is that a destructive factor change needs a
-- FRESH step-up: a factor verification on THIS sign-in lineage, minutes ago.
-- Two things are needed to make that a fact rather than a hope:
--
--   1. `mfa_step_ups` — the durable record. A row means "this refresh chain
--      proved a live factor at this instant". It is keyed by `chain_id`, not
--      by session row, so a refresh rotation (which inserts a new session row
--      and keeps the chain) does not silently drop the proof; and a new login
--      starts a new chain, so one session's step-up can never authorize
--      another's act. `expires_at` is written at insert time by the handler
--      that applied the policy's window, so the freshness rule lives in one
--      place and the row stays inspectable afterwards.
--
--   2. `users.mfa_reenroll_required` — the durable consequence of an admin
--      reset. Clearing every factor would otherwise hand the account back as
--      a password-only login with no record that a factor is owed. The flag
--      is what the MFA surface refuses against, and the enrolment that clears
--      it is the same one that satisfies the reset.
--
-- RLS doctrine: `mfa_step_ups` carries org_id and the same policy shape as its
-- neighbours (0155's `USING (org_id = current_setting('app.org_id', true)::uuid)`),
-- so an unscoped read sees nothing. The table also cascades on both the user
-- and the session, which is what makes "the session is gone" mean "the proof
-- is gone".

CREATE TABLE mfa_step_ups (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  -- The session that presented the factor. Kept (not just the chain) so the
  -- audit and a later investigation can name the request that proved it.
  session_id uuid NOT NULL REFERENCES auth_sessions(id) ON DELETE CASCADE,
  -- The refresh chain the session belongs to. This is the lookup key: a
  -- rotated session carries the same chain, so the proof survives rotation
  -- without the rotation path needing to know about it.
  chain_id uuid NOT NULL,
  verified_at timestamptz NOT NULL DEFAULT now(),
  expires_at timestamptz NOT NULL,
  -- Which method proved the factor. `factor_ref` names the specific
  -- credential for `webauthn` (the credential id) and the consumed code row
  -- for `recovery_code`, so a rule can refuse "authorize removing this key
  -- with this same key".
  method text NOT NULL CHECK (method IN ('totp', 'recovery_code', 'webauthn')),
  factor_ref text,
  CONSTRAINT mfa_step_ups_window_positive CHECK (expires_at > verified_at)
);

-- The freshness lookup: newest live proof for one (user, session) resolved
-- through the chain. Descending on the window is what lets the read stop at
-- the first row sqlx returns.
CREATE INDEX idx_mfa_step_ups_chain
  ON mfa_step_ups (user_id, chain_id, expires_at DESC);

ALTER TABLE mfa_step_ups ENABLE ROW LEVEL SECURITY;
CREATE POLICY mfa_step_ups_org ON mfa_step_ups
  USING (org_id = current_setting('app.org_id', true)::uuid);

COMMENT ON TABLE mfa_step_ups IS
  'One row per successful factor verification on a signed-in refresh chain. '
  'A destructive MFA change (TOTP re-enrolment, passkey removal) requires a '
  'row for the caller''s own chain that has not passed expires_at. Inserted '
  'on MFA login completion and by the step-up endpoint; cleared by the admin '
  'factor reset, which revokes the chain anyway.';

-- The admin reset leaves the account with no factor. This flag is the record
-- that a factor is owed: the MFA surface refuses destructive factor changes
-- while it is set, permits enrolment, and clears it when a factor is
-- confirmed. Without it, a reset would be indistinguishable from an account
-- that never enrolled.
ALTER TABLE users
  ADD COLUMN IF NOT EXISTS mfa_reenroll_required boolean NOT NULL DEFAULT false;

COMMENT ON COLUMN users.mfa_reenroll_required IS
  'Set by an admin factor reset (audited as auth.mfa_factors_reset). While '
  'true the account has no working factor by design: enrolment is permitted, '
  'every other factor change is refused, and confirming a new factor clears '
  'the flag.';
