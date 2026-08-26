-- SLA engine v1 + automations v1 (TASKLIST 2.7).
--
-- Purpose: org-level business-hours calendars (tz, weekday windows,
-- holidays, maintenance windows) that policies attach to; per-policy
-- escalation ladders (threshold steps → notification-center kinds); a
-- fire-once ledger so repeated evaluations can never double-notify;
-- event-triggered automation rules; and the general worker job queue
-- (claim with SKIP LOCKED, retries with backoff, dead-letter state).
--
-- Semantics contract:
--   * `sla_policies.calendar_id` is OPTIONAL — when NULL the policy keeps
--     governing itself via its embedded `calendar` jsonb (the seeded
--     default policy predates calendars and must keep working unchanged).
--   * Escalation step `trigger_kind` binds to notification kinds that
--     already exist in 0007/0012 (`sla_risk` / `sla_breach` /
--     `escalation`) so preferences and quiet hours apply untouched.
--   * `sla_fired_escalations` is the idempotency floor: the PK makes a
--     (ticket, step, target) fire exactly once even under racing workers.
--   * `jobs` is worker-owned infrastructure but carries org_id for
--     uniform tenancy; like every user-data table it gets an RLS policy.
--
-- Non-purpose: no changes to ticket lifecycle tables (0011 owns those),
-- no notification schema changes (0007/0012 own those).

CREATE TABLE sla_calendars (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  timezone text NOT NULL DEFAULT 'UTC',
  -- [Mon..Sun] entries of ["HH:MM","HH:MM"] or null (non-business day),
  -- local wall-clock in `timezone`.
  windows jsonb NOT NULL,
  -- Local dates as "YYYY-MM-DD" strings.
  holidays jsonb NOT NULL DEFAULT '[]',
  -- Planned downtime v1: [{name, starts_at, ends_at}] RFC3339 UTC instants
  -- during which SLA clocks do not consume budget.
  maintenance jsonb NOT NULL DEFAULT '[]',
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, name)
);

ALTER TABLE sla_calendars ENABLE ROW LEVEL SECURITY;
CREATE POLICY sla_calendars_org ON sla_calendars
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- Policy ↔ calendar attachment. NULL keeps the embedded-calendar behavior;
-- deleting a calendar detaches instead of destroying policies.
ALTER TABLE sla_policies ADD COLUMN calendar_id uuid REFERENCES sla_calendars(id) ON DELETE SET NULL;

-- Escalation ladder steps, ordered per policy. `at_pct` is percent of the
-- target's business-time budget consumed when the step fires.
CREATE TABLE sla_escalation_steps (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  policy_id uuid NOT NULL REFERENCES sla_policies(id) ON DELETE CASCADE,
  step_index integer NOT NULL CHECK (step_index >= 0),
  trigger_kind text NOT NULL CHECK (trigger_kind IN ('risk','breach','escalate')),
  at_pct integer NOT NULL CHECK (at_pct BETWEEN 1 AND 500),
  notify_user_ids jsonb NOT NULL DEFAULT '[]',
  enabled boolean NOT NULL DEFAULT true,
  UNIQUE (policy_id, step_index)
);
CREATE INDEX idx_sla_steps_policy ON sla_escalation_steps (policy_id) WHERE enabled;

ALTER TABLE sla_escalation_steps ENABLE ROW LEVEL SECURITY;
CREATE POLICY sla_escalation_steps_org ON sla_escalation_steps
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- Fire-once ledger: one row per fired (ticket, step, clock target).
CREATE TABLE sla_fired_escalations (
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  ticket_id uuid NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
  step_id uuid NOT NULL REFERENCES sla_escalation_steps(id) ON DELETE CASCADE,
  target text NOT NULL CHECK (target IN ('first_response','resolution')),
  fired_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (ticket_id, step_id, target)
);

ALTER TABLE sla_fired_escalations ENABLE ROW LEVEL SECURITY;
CREATE POLICY sla_fired_escalations_org ON sla_fired_escalations
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- Event-triggered automation rules. `board_id` NULL means every board in
-- the org; conditions/actions are JSONB documents validated by core-domain
-- on read (malformed rules are skipped, never crash evaluation).
CREATE TABLE automation_rules (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  board_id uuid REFERENCES boards(id) ON DELETE CASCADE,
  name text NOT NULL,
  -- Domain event type from the envelope, e.g. 'ticket.created'.
  trigger_event text NOT NULL,
  conditions jsonb NOT NULL DEFAULT '[]',
  actions jsonb NOT NULL,
  enabled boolean NOT NULL DEFAULT true,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, name)
);
CREATE INDEX idx_automation_rules_trigger
  ON automation_rules (org_id, trigger_event) WHERE enabled;

ALTER TABLE automation_rules ENABLE ROW LEVEL SECURITY;
CREATE POLICY automation_rules_org ON automation_rules
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- General worker job queue. Claims use FOR UPDATE SKIP LOCKED; retries get
-- exponential backoff via run_after; exhausted attempts land in 'dead'.
CREATE TABLE jobs (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  kind text NOT NULL,
  payload jsonb NOT NULL,
  status text NOT NULL DEFAULT 'pending'
    CHECK (status IN ('pending','running','done','failed','dead')),
  run_after timestamptz NOT NULL DEFAULT now(),
  attempts integer NOT NULL DEFAULT 0,
  max_attempts integer NOT NULL DEFAULT 5,
  last_error text,
  claimed_at timestamptz,
  claimed_by text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_jobs_claim ON jobs (status, run_after) WHERE status = 'pending';
CREATE INDEX idx_jobs_stale ON jobs (claimed_at) WHERE status = 'running';

ALTER TABLE jobs ENABLE ROW LEVEL SECURITY;
CREATE POLICY jobs_org ON jobs
  USING (org_id = current_setting('app.org_id', true)::uuid);
