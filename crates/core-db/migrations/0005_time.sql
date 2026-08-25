-- Multi-timer sessions with segments and time entries.
-- The partial unique index enforces the single-active-timer invariant at the
-- database level: at most one running session per user, ever.

CREATE TABLE timer_sessions (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  ticket_id uuid NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
  started_at timestamptz NOT NULL DEFAULT now(),
  stopped_at timestamptz,
  segments jsonb NOT NULL DEFAULT '[]'
);
CREATE UNIQUE INDEX idx_timer_one_active_per_user
  ON timer_sessions (user_id) WHERE stopped_at IS NULL;
CREATE INDEX idx_timer_user_recent ON timer_sessions (user_id, started_at DESC);

CREATE TABLE time_entries (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  ticket_id uuid NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  started_at timestamptz NOT NULL,
  ended_at timestamptz NOT NULL,
  minutes integer NOT NULL CHECK (minutes >= 0),
  billable boolean NOT NULL DEFAULT true,
  notes text,
  work_type text,
  timer_session_id uuid REFERENCES timer_sessions(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_entries_ticket ON time_entries (ticket_id);
CREATE INDEX idx_entries_user_time ON time_entries (user_id, started_at DESC);
CREATE INDEX idx_entries_org_time ON time_entries (org_id, started_at);

ALTER TABLE timer_sessions ENABLE ROW LEVEL SECURITY;
CREATE POLICY timer_sessions_org ON timer_sessions USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE time_entries ENABLE ROW LEVEL SECURITY;
CREATE POLICY time_entries_org ON time_entries USING (org_id = current_setting('app.org_id', true)::uuid);
