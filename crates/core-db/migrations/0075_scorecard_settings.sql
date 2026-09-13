-- Wave 14 lane 3 `p14-coaching-scorecards` — per-tech scorecard settings.
--
-- The scorecards themselves are pure aggregation over the versioned
-- read-model views (`ops.v1_sla_outcomes`, `ops.v1_first_response`,
-- `ops.v1_tickets`, `ops.v1_time_entries` — the event-trail-derived read
-- model from 0026, extended with the approval-aware columns by 0073); they
-- materialize NOTHING. The one piece of scorecard STATE is the per-user
-- coaching-settings row:
--
--   * `gamification_opt_in` — R5's opt-in gamification toggle. Streaks /
--     leaderboards are opt-in ONLY; the default (false) keeps scorecards
--     private coaching surfaces, never a public scoreboard.
--
-- Privacy model (R5 "private-by-default"): scorecard METRICS have no
-- visibility knob by design — a tech's scorecard is visible to the tech
-- themselves and to owner/admin (enforced in the API layer, not here);
-- the row below only stores the opt-in flag.
CREATE TABLE scorecard_settings (
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  gamification_opt_in boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT uq_scorecard_settings_user UNIQUE (org_id, user_id)
);

ALTER TABLE scorecard_settings ENABLE ROW LEVEL SECURITY;
CREATE POLICY scorecard_settings_org ON scorecard_settings
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

CREATE INDEX idx_scorecard_settings_user ON scorecard_settings (user_id);
