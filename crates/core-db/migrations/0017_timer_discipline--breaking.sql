-- Timer discipline (TASKLIST 2.9/2.10): single-active enforcement fix +
-- missing-time gap audit.
--
-- BREAKING flag rationale (see scripts/check-migrations.sh): the 0005
-- partial unique index treated every session with `stopped_at IS NULL` as
-- active — including PAUSED sessions — so the single-active switch flow
-- (auto-pause old + start new) hit a unique violation on the new insert.
-- This migration drops that index and replaces it with one keyed on a
-- stored `is_running` generated column (running last segment). No data is
-- dropped or rewritten; the change tightens enforcement semantics to what
-- the docked working set requires: many paused sessions, at most one
-- running per user.
--
-- Also adds what the missing-time audit needs:
--   * `time_gap_audits` — one row per (org, user, local day) recording the
--     activity-vs-entries comparison and whether an EOD nudge was sent.
--     The (user, day) unique key makes nudges idempotent: the nudge
--     decision is the `nudge_sent_at IS NULL → set` transition inside the
--     upsert transaction.
--   * org-level knobs: nudge enablement, the local-hour trigger, and the
--     minimum logged minutes below which an active day is a gap.
--
-- Non-purpose: no per-user nudge overrides (notification preferences and
-- quiet hours already govern delivery per kind), no approval workflows.

-- 1. Single-active invariant: "running" means stopped_at IS NULL AND the
--    last segment has no ended_at. Generated (stored) so the partial
--    unique index and query predicates share one immutable definition.
ALTER TABLE timer_sessions ADD COLUMN is_running boolean GENERATED ALWAYS AS (
  stopped_at IS NULL
  AND jsonb_array_length(segments) > 0
  AND (segments -> (jsonb_array_length(segments) - 1) ->> 'ended_at') IS NULL
) STORED;

DROP INDEX idx_timer_one_active_per_user;
CREATE UNIQUE INDEX idx_timer_one_running_per_user
  ON timer_sessions (user_id) WHERE is_running;
CREATE INDEX idx_timer_user_paused ON timer_sessions (user_id, started_at DESC)
  WHERE stopped_at IS NULL AND NOT is_running;

-- 2. Missing-time gap audit ledger (idempotent EOD nudges).
CREATE TABLE time_gap_audits (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  day date NOT NULL,
  activity boolean NOT NULL,
  logged_minutes integer NOT NULL CHECK (logged_minutes >= 0),
  expected_minutes integer NOT NULL CHECK (expected_minutes >= 0),
  has_gap boolean NOT NULL,
  nudge_sent_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, user_id, day)
);
CREATE INDEX idx_gap_audits_org_day ON time_gap_audits (org_id, day DESC);

ALTER TABLE time_gap_audits ENABLE ROW LEVEL SECURITY;
CREATE POLICY time_gap_audits_org ON time_gap_audits
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- 3. Org time-discipline settings. `eod_nudge_hour` is interpreted in each
--    member's own timezone (users.timezone): the nudge fires when local
--    time passes the hour. Defaults-first: nudges on at 18:00 local, a day
--    with ticket activity but under an hour logged is a gap.
ALTER TABLE orgs ADD COLUMN eod_nudges_enabled boolean NOT NULL DEFAULT true;
ALTER TABLE orgs ADD COLUMN eod_nudge_hour smallint NOT NULL DEFAULT 18
  CHECK (eod_nudge_hour >= 0 AND eod_nudge_hour <= 23);
ALTER TABLE orgs ADD COLUMN time_gap_min_minutes integer NOT NULL DEFAULT 60
  CHECK (time_gap_min_minutes >= 0 AND time_gap_min_minutes <= 1440);
