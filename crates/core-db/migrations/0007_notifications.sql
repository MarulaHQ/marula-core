-- Notification center: per-user notifications + per-kind channel preferences.

CREATE TABLE notifications (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  kind text NOT NULL CHECK (kind IN ('mention','assignment','sla_risk','sla_breach','escalation','missing_time','ticket_update')),
  ticket_id uuid REFERENCES tickets(id) ON DELETE CASCADE,
  title text NOT NULL,
  body text,
  read_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_notifications_user ON notifications (user_id, created_at DESC);

CREATE TABLE notification_preferences (
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  kind text NOT NULL CHECK (kind IN ('mention','assignment','sla_risk','sla_breach','escalation','missing_time','ticket_update')),
  in_app boolean NOT NULL DEFAULT true,
  email boolean NOT NULL DEFAULT false,
  quiet_from text,
  quiet_until text,
  PRIMARY KEY (user_id, kind)
);

ALTER TABLE notifications ENABLE ROW LEVEL SECURITY;
CREATE POLICY notifications_org ON notifications USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE notification_preferences ENABLE ROW LEVEL SECURITY;
CREATE POLICY notification_prefs_org ON notification_preferences
  USING (EXISTS (SELECT 1 FROM users u WHERE u.id = notification_preferences.user_id
                 AND u.org_id = current_setting('app.org_id', true)::uuid));
