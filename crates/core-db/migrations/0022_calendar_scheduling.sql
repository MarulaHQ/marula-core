-- Scheduling/calendars/availability foundation (TASKLIST 2.12).
--
-- Purpose: the data-model substrate for the Stage-2 dispatch board:
-- (1) `appointments` — a user's booked time, optionally bound to a ticket
--     and client/site, carrying travel-minute placeholder fields so
--     RoutingProvider estimates can land later without reshaping;
-- (2) `availability_windows` — recurring weekly working windows per user
--     in LOCAL wall-clock time with an IANA timezone and an optional
--     effective date range;
-- (3) `on_call_blocks` — absolute UTC on-call rotations per user;
-- (4) `calendar_exclusions` — absolute UTC time-off that punches holes in
--     availability.
--
-- Overlap doctrine (documented choice): hard double-booking prevention is
-- enforced by GiST EXCLUDE constraints on the ABSOLUTE-time tables where a
-- single person cannot be two places at once — appointments and on-call
-- blocks get `EXCLUDE (user_id WITH =, tstzrange(...) WITH &&)`; cancelled
-- appointments free their slot via the constraint's WHERE predicate while
-- done ones still occupy it (rewriting history into an overlap is still a
-- double-booking). Availability windows are weekly TEMPLATES, not
-- occurrences, so they take no exclusion constraint; containment/coverage
-- math lives in core-domain::scheduling. Exclusions may legally stack.
--
-- Non-purpose: no reminders/recurrence expansion (worker lane, deferred),
-- no iCal feeds, no external calendar sync (Stage 4), no queue tables
-- (owned by the SLA lane).
--
-- RLS: every new user-data table carries an org-scoped policy — the CI
-- gate (`assert_rls_coverage`) fails otherwise.

CREATE TYPE appointment_kind AS ENUM ('on_site', 'remote', 'internal', 'personal');
CREATE TYPE appointment_status AS ENUM ('scheduled', 'in_progress', 'done', 'cancelled');

CREATE EXTENSION IF NOT EXISTS btree_gist;

CREATE TABLE appointments (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- The assignee whose calendar holds the slot (the resource being booked).
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  ticket_id uuid REFERENCES tickets(id) ON DELETE SET NULL,
  client_id uuid REFERENCES clients(id) ON DELETE SET NULL,
  site_id uuid REFERENCES sites(id) ON DELETE SET NULL,
  title text NOT NULL,
  starts_at timestamptz NOT NULL,
  ends_at timestamptz NOT NULL,
  kind appointment_kind NOT NULL DEFAULT 'on_site',
  status appointment_status NOT NULL DEFAULT 'scheduled',
  -- Travel placeholders for dispatch-board v2 / RoutingProvider estimates;
  -- never interpreted by scheduling math in this stage.
  travel_minutes integer NOT NULL DEFAULT 0 CHECK (travel_minutes >= 0),
  notes text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CHECK (ends_at > starts_at)
);

-- No two live bookings of the same person may overlap. btree_gist provides
-- the uuid equality operator class; the partial predicate exempts
-- cancelled rows so cancelling frees the slot immediately.
ALTER TABLE appointments ADD CONSTRAINT appointments_no_double_booking
  EXCLUDE USING gist (
    user_id WITH =,
    tstzrange(starts_at, ends_at, '[)') WITH &&
  ) WHERE (status <> 'cancelled');

CREATE INDEX idx_appointments_org_time ON appointments (org_id, starts_at);
CREATE INDEX idx_appointments_user_time ON appointments (user_id, starts_at);
CREATE INDEX idx_appointments_ticket ON appointments (ticket_id)
  WHERE ticket_id IS NOT NULL;

ALTER TABLE appointments ENABLE ROW LEVEL SECURITY;
CREATE POLICY appointments_org ON appointments
  USING (org_id = current_setting('app.org_id', true)::uuid);

CREATE TABLE availability_windows (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  -- 0 = Monday … 6 = Sunday (chrono num_days_from_monday convention).
  weekday smallint NOT NULL CHECK (weekday BETWEEN 0 AND 6),
  start_local time NOT NULL,
  end_local time NOT NULL,
  timezone text NOT NULL,
  effective_from date,
  effective_until date,
  created_at timestamptz NOT NULL DEFAULT now(),
  CHECK (start_local < end_local),
  CHECK (effective_until IS NULL OR effective_from IS NULL OR effective_until >= effective_from)
);

-- Exact-duplicate templates are rejected (including across tz/effective
-- ranges); overlapping-but-distinct templates are legal and resolved by
-- coverage union in core-domain::scheduling.
CREATE UNIQUE INDEX idx_availability_no_dupes
  ON availability_windows (
    org_id, user_id, weekday, start_local, end_local, timezone,
    COALESCE(effective_from, '0001-01-01'::date),
    COALESCE(effective_until, '0001-01-01'::date)
  );
CREATE INDEX idx_availability_user ON availability_windows (org_id, user_id);

ALTER TABLE availability_windows ENABLE ROW LEVEL SECURITY;
CREATE POLICY availability_windows_org ON availability_windows
  USING (org_id = current_setting('app.org_id', true)::uuid);

CREATE TABLE on_call_blocks (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  starts_at timestamptz NOT NULL,
  ends_at timestamptz NOT NULL,
  -- Free-form rotation label ('primary', 'secondary', 'escalation', …);
  -- semantics stay org-level until dispatch-board needs structure.
  category text NOT NULL DEFAULT 'primary',
  notes text,
  created_at timestamptz NOT NULL DEFAULT now(),
  CHECK (ends_at > starts_at)
);

-- One rotation slot per person at a time: stacked on-call for one user is
-- always a configuration mistake in this model.
ALTER TABLE on_call_blocks ADD CONSTRAINT on_call_no_overlap
  EXCLUDE USING gist (
    user_id WITH =,
    tstzrange(starts_at, ends_at, '[)') WITH &&
  );

CREATE INDEX idx_on_call_org_time ON on_call_blocks (org_id, starts_at);
CREATE INDEX idx_on_call_user_time ON on_call_blocks (user_id, starts_at);

ALTER TABLE on_call_blocks ENABLE ROW LEVEL SECURITY;
CREATE POLICY on_call_blocks_org ON on_call_blocks
  USING (org_id = current_setting('app.org_id', true)::uuid);

CREATE TABLE calendar_exclusions (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  starts_at timestamptz NOT NULL,
  ends_at timestamptz NOT NULL,
  reason text,
  created_at timestamptz NOT NULL DEFAULT now(),
  CHECK (ends_at > starts_at)
);
CREATE INDEX idx_calendar_exclusions_user_time
  ON calendar_exclusions (org_id, user_id, starts_at);

ALTER TABLE calendar_exclusions ENABLE ROW LEVEL SECURITY;
CREATE POLICY calendar_exclusions_org ON calendar_exclusions
  USING (org_id = current_setting('app.org_id', true)::uuid);
