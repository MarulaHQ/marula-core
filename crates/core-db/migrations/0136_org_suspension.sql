-- Tenant suspension notice flag (TASKLIST Wave 47 lane A7,
-- `p47-org-suspension-notice`).
--
-- NUMBERING: this is 0136, the next free slot at lane start (main's head was
-- 0135_org_custom_domains.sql). The board claim is a reservation at claim
-- time, not a guarantee at merge time: any lane claiming a migration number
-- must re-check it against main's head immediately before merge, and the
-- loser renumbers (see the 0131 header for the precedent).
--
-- (A) THREE nullable columns on `orgs`, and nothing else:
--   * `suspended_at` — when the org was flagged. NULL means the org is not
--     flagged, which is the state every org ships in.
--   * `suspension_reason` — why, from a closed set: `billing_lapsed`,
--     `abuse`, or `other`. The reason is operator-facing text a human reads
--     before acting; a bare enum value with no home in the schema would
--     leave that human guessing.
--   * `suspension_notify_after` — the interval after `suspended_at` at which
--     the in-app notice starts showing. NULL means the notice is due
--     immediately; the hosted tier can also set it so a customer sees the
--     flag only after a grace window.
--
-- (C) THE POSTURE — NOTICE, NEVER A BLOCK (binding on this lane):
--   Nothing in this schema and nothing that reads it stops a request. There
--   is no trigger, no RLS predicate, no middleware branch, and no feature
--   gate keyed on these columns. A flagged org keeps every endpoint, every
--   plugin, and every write it had the day before it was flagged. The flag
--   exists so a surface can TELL the customer what is happening and offer
--   the way to take their data with them; it is not an enforcement
--   mechanism, and a later lane that adds one to this column is changing
--   this line, not extending it.
--
-- The coherence constraint: a flag with no reason is a dead end for the
-- operator who has to explain it, so the two move together or not at all.
-- `suspension_notify_after` is deliberately outside the constraint — it is
-- an independent knob about WHEN the notice appears, not whether the org is
-- flagged.
--
-- RLS: the three columns live on `orgs`, whose `org_self` policy (0001)
-- already scopes every SELECT/UPDATE to `app.org_id` (omitted WITH CHECK
-- falls back to USING, so it constrains the new row too). No new policy is
-- needed — the API layer owns the admin gate on the write, applied on top of
-- the row-level scope, exactly as org settings do.

ALTER TABLE orgs ADD COLUMN suspended_at timestamptz;
COMMENT ON COLUMN orgs.suspended_at IS
  'When the org was flagged for the hosted-tier suspension NOTICE. NULL = not flagged. Informational only: nothing in the core gates a request on this.';

ALTER TABLE orgs ADD COLUMN suspension_reason text
  CHECK (suspension_reason IS NULL OR
         suspension_reason IN ('billing_lapsed', 'abuse', 'other'));
COMMENT ON COLUMN orgs.suspension_reason IS
  'Why the org was flagged: billing_lapsed | abuse | other. NULL exactly when suspended_at is NULL (coherence constraint below).';

ALTER TABLE orgs ADD COLUMN suspension_notify_after interval
  CHECK (suspension_notify_after IS NULL OR suspension_notify_after >= interval '0');
COMMENT ON COLUMN orgs.suspension_notify_after IS
  'Interval after suspended_at at which the customer-facing notice starts showing. NULL = due immediately.';

-- The flag and its reason are one fact, so the schema keeps them one fact.
-- Every write path sets both or clears both; this makes a half-set flag
-- impossible even from a psql session that never read this file.
ALTER TABLE orgs ADD CONSTRAINT orgs_suspension_flag_coherent
  CHECK ((suspended_at IS NULL) = (suspension_reason IS NULL));
