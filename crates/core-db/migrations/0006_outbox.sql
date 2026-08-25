-- Transactional outbox (§4.2): every mutation inserts its event here in the
-- SAME transaction as the data write. The worker relays unpublished rows to
-- the Redis stream and marks them published.

CREATE TABLE outbox_events (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  event_type text NOT NULL,
  entity text NOT NULL,
  entity_id text,
  payload jsonb NOT NULL,
  payload_schema_ref text NOT NULL DEFAULT 'core-domain/v1',
  occurred_at timestamptz NOT NULL DEFAULT now(),
  published_at timestamptz
);
CREATE INDEX idx_outbox_unpublished ON outbox_events (occurred_at) WHERE published_at IS NULL;

ALTER TABLE outbox_events ENABLE ROW LEVEL SECURITY;
CREATE POLICY outbox_events_org ON outbox_events USING (org_id = current_setting('app.org_id', true)::uuid);
