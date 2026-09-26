-- CSAT surveys on resolution (Wave 34 lane 2, `p34-csat-surveys`; migration
-- 0108 is this lane's exclusive window; head was 0107).
--
-- Purpose: the Stage 3/4 service-desk extra the scorecards wave stubbed.
-- One table carries the whole lifecycle — survey INTENT (queued on resolve),
-- delivery state (sent via MailTransport with a tokenized response link),
-- and the response itself (single-use token, score 1-5, optional comment).
--
--   * Delivery is deferred, not dropped, through quiet hours: `scheduled_for`
--     moves to the end of the org's quiet window and the worker sweep sends
--     when due. The quiet-hours MODEL is reused from
--     core-domain::notifications — no fork.
--
--   * The response token is stored as a SHA-256 hash; the raw token lives
--     only in the emailed link. `responded_at` marks single-use consumption.
--
--   * No mail transport configured ⇒ the worker inserts nothing and records
--     the typed not-configured state (ADR 0010 posture); scorecards keep
--     `csat.available: false` honestly instead of faking an empty dataset.
--
-- Additive only: one new table. Nothing existing is rewritten or dropped.

CREATE TABLE csat_surveys (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  ticket_id uuid NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
  client_id uuid REFERENCES clients(id) ON DELETE SET NULL,
  contact_email text NOT NULL CHECK (contact_email ~* '^[^@[:space:]]+@[^@[:space:]]+\.[^@[:space:]]+$'),
  -- SHA-256 of the single-use response token; raw token never stored.
  token_hash text NOT NULL UNIQUE CHECK (char_length(token_hash) = 64),
  token_expires_at timestamptz NOT NULL,
  -- pending = queued (quiet hours or transport); sent = link delivered;
  -- responded = consumed; skipped = policy said no (interval/none-mode).
  status text NOT NULL DEFAULT 'pending'
    CHECK (status IN ('pending', 'sent', 'responded', 'skipped')),
  skip_reason text CHECK (skip_reason IN ('min_interval', 'transport_not_configured')),
  scheduled_for timestamptz NOT NULL DEFAULT now(),
  sent_at timestamptz,
  score smallint CHECK (score BETWEEN 1 AND 5),
  comment text CHECK (comment IS NULL OR char_length(comment) <= 2000),
  responded_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  -- One survey per resolved ticket, ever.
  UNIQUE (org_id, ticket_id)
);
CREATE INDEX idx_csat_surveys_due
  ON csat_surveys (org_id, scheduled_for) WHERE status = 'pending';
CREATE INDEX idx_csat_surveys_client
  ON csat_surveys (org_id, client_id, scheduled_for) WHERE client_id IS NOT NULL;
CREATE INDEX idx_csat_surveys_ticket
  ON csat_surveys (org_id, ticket_id);

ALTER TABLE csat_surveys ENABLE ROW LEVEL SECURITY;
CREATE POLICY csat_surveys_org ON csat_surveys
  USING (org_id = current_setting('app.org_id', true)::uuid);
