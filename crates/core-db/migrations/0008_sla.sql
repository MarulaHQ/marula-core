-- SLA policies (with embedded business calendars) and per-ticket SLA state.

CREATE TABLE sla_policies (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  is_default boolean NOT NULL DEFAULT false,
  targets jsonb NOT NULL,
  calendar jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, name)
);

CREATE TABLE ticket_sla_state (
  ticket_id uuid PRIMARY KEY REFERENCES tickets(id) ON DELETE CASCADE,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  policy_id uuid NOT NULL REFERENCES sla_policies(id) ON DELETE RESTRICT,
  first_response_due timestamptz,
  resolution_due timestamptz,
  first_responded_at timestamptz,
  paused_business_secs bigint NOT NULL DEFAULT 0,
  paused_since timestamptz,
  breached boolean NOT NULL DEFAULT false,
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_sla_state_due ON ticket_sla_state (org_id, resolution_due) WHERE breached = false;

ALTER TABLE sla_policies ENABLE ROW LEVEL SECURITY;
CREATE POLICY sla_policies_org ON sla_policies USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE ticket_sla_state ENABLE ROW LEVEL SECURITY;
CREATE POLICY ticket_sla_state_org ON ticket_sla_state USING (org_id = current_setting('app.org_id', true)::uuid);
