-- Scheduled report delivery (the private build tracker R4; ADR 0001 §Decision).
--
-- Purpose: two worker-facing tables for the delivery lane.
--
-- `report_schedules` — one row per delivery schedule: a reference to an
-- R3 `report_definitions` row, the recipient staff users, a typed
-- cadence (daily | weekly | monthly at a wall-clock time in an IANA
-- timezone), and the trailing-day window each run covers. The worker
-- owns the CLOCK here: `next_run_at` is materialized so the listing can
-- preview it and the claim sweep can `FOR UPDATE SKIP LOCKED` due rows
-- without recomputing cadence math per poll.
--
-- Cadence shape is pinned by CHECKs, not runtime validation alone: a
-- weekly schedule MUST carry `day_of_week` (0 = Monday) and MUST NOT
-- carry `day_of_month`; monthly the mirror image (1–28 — short months
-- simply have every such day); daily carries neither. `time_of_day` is
-- a naive time read in `timezone` (validated as an IANA name in Rust on
-- every write — the DB cannot enumerate zones without extra extensions).
--
-- Recipients are STAFF users (`uuid[]` FK-checked one-by-one at write
-- time in Rust; array FKs are not enforceable in Postgres). Delivery is
-- always governed per-recipient by the notification preference chain
-- (kind `report_delivery`) incl. quiet hours — a schedule names WHO is
-- offered the report; preferences decide whether each recipient's email
-- leg actually fires.
--
-- `report_deliveries` — the append-only run ledger the worker writes:
-- one row per claimed schedule slot with the per-recipient outcomes
-- (delivered / disabled / quiet_deferred / failed / skipped-none) in
-- `detail`. Delivery is at-least-once in worker-crash terms; the ledger
-- makes that visible, never silent.
--
-- Non-purpose: no report EXECUTION state (spec results are never
-- persisted — each run compiles on demand), no cron strings (cadence is
-- a typed enum), no per-recipient scheduling offsets (one slot fires
-- one fan-out), no events from the worker (only core-api's CRUD surface
-- emits; the ledger is the observability record).

CREATE TABLE report_schedules (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  report_definition_id uuid NOT NULL REFERENCES report_definitions(id) ON DELETE CASCADE,
  name text NOT NULL,
  -- Recipient staff users of THIS org; validated (existence + org
  -- membership + at least one) in Rust on every write.
  recipients uuid[] NOT NULL CHECK (array_length(recipients, 1) >= 1),
  -- Typed cadence. Weekly/monthly consistency enforced below.
  cadence text NOT NULL CHECK (cadence IN ('daily','weekly','monthly')),
  -- Local wall-clock fire time in `timezone`.
  time_of_day time NOT NULL,
  -- 0 = Monday .. 6 = Sunday; weekly only.
  day_of_week smallint CHECK (day_of_week BETWEEN 0 AND 6),
  -- 1–28 so every month has the day; monthly only.
  day_of_month smallint CHECK (day_of_month BETWEEN 1 AND 28),
  -- IANA zone the wall clock reads in (Rust-validated; UTC by default).
  timezone text NOT NULL DEFAULT 'UTC',
  -- Trailing-day window each run covers (the spec itself is windowless).
  window_days integer NOT NULL DEFAULT 30 CHECK (window_days BETWEEN 1 AND 366),
  enabled boolean NOT NULL DEFAULT true,
  -- Materialized claim state: due when `enabled AND next_run_at <= now`.
  next_run_at timestamptz NOT NULL,
  last_run_at timestamptz,
  last_error text,
  created_by uuid NOT NULL REFERENCES users(id),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, name)
);

-- Cadence shape as data: a weekly schedule MUST carry `day_of_week` and
-- MUST NOT carry `day_of_month`; monthly the mirror image; daily carries
-- neither. One constraint spells the whole truth so a half-shaped row can
-- never land regardless of column order or hand-written inserts.
ALTER TABLE report_schedules
  ADD CONSTRAINT report_schedules_cadence_shape CHECK (
    (cadence = 'daily'    AND day_of_week IS NULL     AND day_of_month IS NULL)
    OR (cadence = 'weekly'  AND day_of_week IS NOT NULL AND day_of_month IS NULL)
    OR (cadence = 'monthly' AND day_of_week IS NULL     AND day_of_month IS NOT NULL)
  );

CREATE INDEX idx_report_schedules_org ON report_schedules (org_id, name);
CREATE INDEX idx_report_schedules_due ON report_schedules (next_run_at) WHERE enabled;

ALTER TABLE report_schedules ENABLE ROW LEVEL SECURITY;
CREATE POLICY report_schedules_org ON report_schedules
  USING (org_id = current_setting('app.org_id', true)::uuid);

CREATE TABLE report_deliveries (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  schedule_id uuid NOT NULL REFERENCES report_schedules(id) ON DELETE CASCADE,
  -- The claimed slot this run discharged (== the schedule's next_run_at
  -- at claim time) — a run's window is [slot - window_days, slot).
  slot_at timestamptz NOT NULL,
  -- delivered | partial | failed | skipped (none-mode log-skip).
  status text NOT NULL CHECK (status IN ('delivered','partial','failed','skipped')),
  -- Per-recipient outcomes keyed by user id:
  -- {"delivered": n, "disabled": n, "quiet_deferred": n, "failed": [..]}.
  detail jsonb NOT NULL CHECK (jsonb_typeof(detail) = 'object'),
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX idx_report_deliveries_schedule ON report_deliveries (schedule_id, slot_at DESC);

ALTER TABLE report_deliveries ENABLE ROW LEVEL SECURITY;
CREATE POLICY report_deliveries_org ON report_deliveries
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- --- report_delivery notification kind ---------------------------------------
--
-- Scheduled delivery rides the SAME preference chain as every other email
-- leg: per-user rows, org defaults, then built-ins (email=false). Widening
-- the three kind CHECKs is additive — existing rows are untouched and the
-- slug is reserved by this migration from here on.

ALTER TABLE notification_preferences DROP CONSTRAINT notification_preferences_kind_check;
ALTER TABLE notification_preferences ADD CONSTRAINT notification_preferences_kind_check
  CHECK (kind IN ('mention','assignment','sla_risk','sla_breach','escalation','missing_time','ticket_update','report_delivery'));

ALTER TABLE notification_org_defaults DROP CONSTRAINT notification_org_defaults_kind_check;
ALTER TABLE notification_org_defaults ADD CONSTRAINT notification_org_defaults_kind_check
  CHECK (kind IN ('mention','assignment','sla_risk','sla_breach','escalation','missing_time','ticket_update','report_delivery'));

ALTER TABLE notifications DROP CONSTRAINT notifications_kind_check;
ALTER TABLE notifications ADD CONSTRAINT notifications_kind_check
  CHECK (kind IN ('mention','assignment','sla_risk','sla_breach','escalation','missing_time','ticket_update','report_delivery'));
