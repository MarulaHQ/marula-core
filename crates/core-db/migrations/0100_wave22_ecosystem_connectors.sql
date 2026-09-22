-- Wave 22 lane 1 `p22-ecosystem-stretch` — ecosystem connectors + the
-- distributor pricing feeds (legs over the existing
-- `connector_installations` seam, which is the consent store):
--
-- 1. `voip_call_logs` — the VoIP connector API's ingest read model.
--    Machine actors POST call records; every row keeps the
--    connector_installation provenance (the consented source), the
--    provider call id UNIQUE per org (idempotent ingest — a retried
--    call-log event is ONE row), and an optional linked ticket. No
--    ticket side effects unless the caller links one.
--
-- 2. `pricing_feed_batches` + `pricing_feed_entries` — distributor
--    pricing feeds land as immutable BATCHES (an upload is a batch of
--    SKU rows); applying a batch overlays `catalog_items.cost_minor` by
--    SKU in one transaction (typed report, never a silent partial).

-- ---------------------------------------------------------------------------
-- 1. VoIP call logs (machine ingest, one row per provider call id).
-- ---------------------------------------------------------------------------

CREATE TABLE voip_call_logs (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  installation_id uuid NOT NULL REFERENCES connector_installations(id) ON DELETE CASCADE,
  call_id text,
  direction text NOT NULL CHECK (direction IN ('in', 'out')),
  external_number text NOT NULL,
  duration_seconds integer NOT NULL CHECK (duration_seconds >= 0),
  summary text NOT NULL DEFAULT '',
  linked_ticket_id uuid REFERENCES tickets(id) ON DELETE SET NULL,
  recorded_at timestamptz NOT NULL DEFAULT now(),
  occurred_at timestamptz NOT NULL,
  -- Idempotent machine ingest: one row per provider call id per org.
  -- Registered in `map_unique_constraint` as `voip_call_replayed`.
  CONSTRAINT uq_voip_call_id UNIQUE (org_id, call_id)
);

CREATE INDEX idx_voip_call_logs_ticket
  ON voip_call_logs (org_id, linked_ticket_id);

ALTER TABLE voip_call_logs ENABLE ROW LEVEL SECURITY;
CREATE POLICY voip_call_logs_org
  ON voip_call_logs
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

-- ---------------------------------------------------------------------------
-- 2. Distributor pricing feeds (batches + line entries, apply = overlay).
-- ---------------------------------------------------------------------------

CREATE TABLE pricing_feed_batches (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  source text NOT NULL,
  note text NOT NULL DEFAULT '',
  uploaded_by uuid REFERENCES users(id) ON DELETE SET NULL,
  uploaded_at timestamptz NOT NULL DEFAULT now(),
  applied boolean NOT NULL DEFAULT false,
  applied_at timestamptz,
  applied_by uuid REFERENCES users(id) ON DELETE SET NULL
);

CREATE TABLE pricing_feed_entries (
  id uuid PRIMARY KEY,
  batch_id uuid NOT NULL REFERENCES pricing_feed_batches(id) ON DELETE CASCADE,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- The canonical join key is lower(sku) per org — the SAME key the
  -- catalog's own unique index reserves.
  sku text,
  currency text NOT NULL,
  unit_price_minor bigint NOT NULL CHECK (unit_price_minor >= 0),
  cost_minor bigint NOT NULL DEFAULT 0 CHECK (cost_minor >= 0)
);

-- A batch is optional SKU collections; entries join via batch.
CREATE INDEX idx_pricing_feed_batches_org
  ON pricing_feed_batches (org_id, uploaded_at DESC);
CREATE INDEX idx_pricing_feed_entries_batch
  ON pricing_feed_entries (batch_id);

ALTER TABLE pricing_feed_batches ENABLE ROW LEVEL SECURITY;
CREATE POLICY pricing_feed_batches_org
  ON pricing_feed_batches
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE pricing_feed_entries ENABLE ROW LEVEL SECURITY;
CREATE POLICY pricing_feed_entries_org
  ON pricing_feed_entries
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

-- Receipt read: applying one batch returns the overlay outcome per SKU
-- that had a matching active catalog row (typo'd / unknown SKUs leave a
-- typed unmatched record the ops UI surfaces).
CREATE TABLE pricing_feed_outcomes (
  id uuid PRIMARY KEY,
  batch_id uuid NOT NULL REFERENCES pricing_feed_batches(id) ON DELETE CASCADE,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  sku text,
  matched_catalog_item_id uuid,
  matched boolean NOT NULL,
  reason text NOT NULL DEFAULT ''
);
CREATE INDEX idx_pricing_feed_outcomes_batch ON pricing_feed_outcomes (batch_id);

ALTER TABLE pricing_feed_outcomes ENABLE ROW LEVEL SECURITY;
CREATE POLICY pricing_feed_outcomes_org
  ON pricing_feed_outcomes
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);
