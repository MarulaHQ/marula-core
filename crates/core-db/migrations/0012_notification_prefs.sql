-- Stage 2 / item 2.2 — notification preferences completion.
--
-- Extends (never rewrites) the 0007 notification tables:
--   * per-preference quiet-hours timezone (windows are wall-clock in this tz)
--   * org-level defaults rows consulted BEFORE the built-in defaults
--     (defaults-first doctrine §0.5): user row → org default → built-in
--   * unread-state tracking support: partial index for unread scans
--     (`read_at` remains the single source of read state)

ALTER TABLE notification_preferences
  ADD COLUMN timezone text NOT NULL DEFAULT 'UTC';

-- Org defaults: one row per (org, kind). Absent row ⇒ built-in default
-- (in_app = true, email = false, no quiet hours).
CREATE TABLE notification_org_defaults (
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  kind text NOT NULL CHECK (kind IN ('mention','assignment','sla_risk','sla_breach','escalation','missing_time','ticket_update')),
  in_app boolean NOT NULL DEFAULT true,
  email boolean NOT NULL DEFAULT false,
  quiet_from text CHECK (quiet_from IS NULL OR quiet_from ~ '^\d{2}:\d{2}$'),
  quiet_until text CHECK (quiet_until IS NULL OR quiet_until ~ '^\d{2}:\d{2}$'),
  timezone text NOT NULL DEFAULT 'UTC',
  PRIMARY KEY (org_id, kind)
);

ALTER TABLE notification_org_defaults ENABLE ROW LEVEL SECURITY;
CREATE POLICY notification_org_defaults_org ON notification_org_defaults
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- Quiet-hours windows must come as a pair (both bounds or neither) so a
-- half-configured window can never silently suppress a whole day.
ALTER TABLE notification_preferences
  ADD CONSTRAINT notification_prefs_quiet_pair
  CHECK ((quiet_from IS NULL) = (quiet_until IS NULL));
ALTER TABLE notification_org_defaults
  ADD CONSTRAINT notification_org_defaults_quiet_pair
  CHECK ((quiet_from IS NULL) = (quiet_until IS NULL));

-- Unread-badge scans filter `read_at IS NULL`; this partial index keeps the
-- count and mark-all paths off the full per-user history index.
CREATE INDEX idx_notifications_unread
  ON notifications (org_id, user_id) WHERE read_at IS NULL;

UPDATE notification_preferences SET timezone = 'UTC' WHERE timezone = '';
