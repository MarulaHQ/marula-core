-- Wave 19 lane 3 `p19-secrets-product` — the deliberate lane-7 deferrals:
-- one-time share links + rotation automation.
--
-- 1. `secret_share_links` — one-time, scoped, expiring, audited share
--    links for vault material. The SERVER stores only a SHA-256 hex
--    token_hash: the raw token exists ONCE, inside the create response
--    (the secret-handling posture of routes/secrets.rs is binding —
--    plaintext secrets never sit in a table, log, or event). A link is
--    single-use by construction (a transactional FOR UPDATE claim means
--    exactly one claimer wins), org-scoped (the claimer must belong to
--    the sharing org, and the same category-grant / role gate the
--    reveal endpoints enforce applies at claim time), expires by
--    `expires_at`, and revocable by the creator side at any time.
--    Every lifecycle step (created / claimed / revoked / refused) rides
--    the append-only `audit_events` trail in code — this table is
--    state, not the audit trail.
--
-- 2. Rotation state, additive columns on BOTH vault subjects:
--    `secret_records` (the native ADR-0006 records) and `passwords`
--    (the legacy byte-compatible entries). `rotate_at` is the
--    operator-scheduled rotation deadline; `last_rotated_at` is
--    stamped by the same update that moves `rotate_at` forward;
--    `rotation_reminded_at` is the worker sweep's durable
--    at-most-once reminder marker (the sweep flips it in the same
--    UPDATE that claims the row, so a crash either reminds once or
--    never double-sends). Reminders fire as the `secret.rotation_due`
--    domain event (automation-rule/webhook hook: org subscribers see
--    the event on the single event stream) + an in-app notification —
--    never with any secret material in the payload.

-- ---------------------------------------------------------------------------
-- 1. One-time share links (org-scoped, expiring, revocable).
-- ---------------------------------------------------------------------------

CREATE TABLE secret_share_links (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  subject_kind text NOT NULL CHECK (subject_kind IN ('secret_record', 'password')),
  subject_id uuid NOT NULL,
  -- SHA-256 hex of the random token (64 lowercase hex chars); UNIQUE so
  -- a claimed link can be re-resolved exactly once by hash. The typed
  -- reason-code registry consumes this constraint name:
  -- `share_link_token_taken`.
  token_hash text NOT NULL UNIQUE,
  label text NOT NULL DEFAULT '',
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  expires_at timestamptz NOT NULL,
  revoked_at timestamptz,
  consumed_at timestamptz,
  consumed_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  -- A granted claim row can never also be a revoked/expired one.
  CHECK (revoked_at IS NULL OR consumed_at IS NULL)
);

CREATE INDEX idx_secret_share_links_subject
  ON secret_share_links (org_id, subject_kind, subject_id);
CREATE INDEX idx_secret_share_links_expiry
  ON secret_share_links (expires_at);

-- Expiry/revocation/claim can all be enforced in the API layer BEFORE a
-- claim is even attempted; the sweep-side garbage collection of dead
-- links (already consumed or revoked rows persist as ledger rows) is a
-- deliberate non-goal: the rows are the audit story the ops UI reads.
COMMENT ON TABLE secret_share_links IS
  'One-time scoped expiring share links for vault material (Wave 19 lane
3). Token HASH only — the raw token is returned exactly once in the
create response and never stored. Claim is a transactional FOR UPDATE
consume behind the org + role/category gates; lifecycle audited via
audit_events.';

ALTER TABLE secret_share_links ENABLE ROW LEVEL SECURITY;
CREATE POLICY secret_share_links_org ON secret_share_links
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

-- ---------------------------------------------------------------------------
-- 2. Rotation state — additive columns on both vault subjects.
-- ---------------------------------------------------------------------------

ALTER TABLE secret_records
  ADD COLUMN rotate_at timestamptz,
  ADD COLUMN last_rotated_at timestamptz,
  ADD COLUMN rotation_reminded_at timestamptz;

ALTER TABLE passwords
  ADD COLUMN rotate_at timestamptz,
  ADD COLUMN last_rotated_at timestamptz,
  ADD COLUMN rotation_reminded_at timestamptz;

-- The worker rotation sweep polls exactly this shape; the partial index
-- keeps the claim scan free of all never-scheduled rows.
CREATE INDEX idx_secret_records_rotate_due
  ON secret_records (rotate_at) WHERE rotate_at IS NOT NULL;
CREATE INDEX idx_passwords_rotate_due
  ON passwords (rotate_at) WHERE rotate_at IS NOT NULL;

-- last_rotated_at can never run ahead of the schedule it came from the
-- same write (operators stamp both together through the API).
COMMENT ON COLUMN secret_records.rotation_reminded_at IS
  'At-most-once marker flipped by the worker rotation sweep (Wave 19
lane 3); the sweep emits `secret.rotation_due` once per schedule until
the row is rotated again.';

-- ---------------------------------------------------------------------------
-- 3. Notification kind `secret_rotation` + the cross-org token bridge.
-- ---------------------------------------------------------------------------

-- The rotation reminder lands in the SAME in-app notification inbox; the
-- kind widening follows the 0038/0081 additive recipe (existing rows
-- untouched; the slug is reserved from here on). In-app delivery is the
-- reminder surface; the domain event (`secret.rotation_due`) is the
-- automation/webhook hook orgs wire into their own runbooks.
ALTER TABLE notifications DROP CONSTRAINT notifications_kind_check;
ALTER TABLE notifications ADD CONSTRAINT notifications_kind_check
  CHECK (kind IN ('mention','assignment','sla_risk','sla_breach','escalation','missing_time','ticket_update','report_delivery','ai_cap_warn','secret_rotation'));

ALTER TABLE notification_preferences DROP CONSTRAINT notification_preferences_kind_check;
ALTER TABLE notification_preferences ADD CONSTRAINT notification_preferences_kind_check
  CHECK (kind IN ('mention','assignment','sla_risk','sla_breach','escalation','missing_time','ticket_update','report_delivery','ai_cap_warn','secret_rotation'));

ALTER TABLE notification_org_defaults DROP CONSTRAINT notification_org_defaults_kind_check;
ALTER TABLE notification_org_defaults ADD CONSTRAINT notification_org_defaults_kind_check
  CHECK (kind IN ('mention','assignment','sla_risk','sla_breach','escalation','missing_time','ticket_update','report_delivery','ai_cap_warn','secret_rotation'));

-- The claim-time token bridge (`resolve_secret_share_link`): the
-- SAME narrow SECURITY DEFINER doctrine as `resolve_inbound_email_route`
-- (0014) — the claim flow must first learn which org a token's hash
-- belongs to, and RLS has no home org yet at that instant (the claimer's
-- membership is not a scope grant — it is verified explicitly AFTER the
-- resolve, matching the link's home org). Hash-only input; the function
-- NEVER exposes raw tokens (it cannot — it receives a hash).
CREATE FUNCTION resolve_secret_share_link(p_token_hash text)
RETURNS TABLE (
  r_id uuid, r_org_id uuid, r_subject_kind text, r_subject_id uuid,
  r_expires_at timestamptz, r_revoked_at timestamptz, r_consumed_at timestamptz
)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public AS $fn$
  SELECT id, org_id, subject_kind, subject_id, expires_at, revoked_at, consumed_at
  FROM secret_share_links WHERE token_hash = p_token_hash
$fn$;

REVOKE ALL ON FUNCTION resolve_secret_share_link(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION resolve_secret_share_link(text) TO meridian_app;
