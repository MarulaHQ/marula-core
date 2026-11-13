-- 0184 — the ingest request log: the durable record behind
-- `marula.org.ingest_requests`.
--
-- Why. Wave 52 lane R12 put a counter behind both inbound doors
-- (`core_otel::metrics::org_ingest`, emitted once the HMAC signature has
-- proved the tenant, in `routes/alerts.rs` and `routes/webhook_receiver.rs`).
-- A counter answers "how much" and nothing else. The two questions an MSP
-- asks are "which source sent this, and when" and "which source went quiet",
-- and neither is a number: both need one row per request.
--
-- This table is that record. It is written at the point the counter is
-- incremented, over the same population, so the number and the log cannot
-- disagree about what "an ingest request" is.
--
-- What it deliberately is not:
--   * not the replay fence — `webhook_ingest_dedup` owns that;
--   * not the envelope — `outbox_events` owns that;
--   * not the alert ledger, and not an audit event.
-- Those three record what a delivery BECAME. A refused delivery becomes
-- nothing, and "we sent it and you refused it" is the row that is missing
-- when a connector owner calls.
--
-- `alert_source_id` is nullable and SET NULL on delete: the log outlives the
-- source it names. Deleting a source is not a reason to lose the record of the
-- traffic it sent, and `source_ref` — the value the caller actually used —
-- stays with the row.
--
-- Retention. Rows live for the window in the retention matrix, not for a
-- comment: `core_db::retention::MATRIX` (7 days compact, 30 standard, 90
-- extended, floor 7), and the worker's hourly `data_retention_sweep` deletes
-- them on the housekeeping role. No surface deletes a row, ever.
--
-- RLS doctrine: org-scoped user data, org-bound reads
-- (`assert_rls_coverage` requires the column and the policy to ship together).

CREATE TABLE ingest_requests (
  -- The correlation id (uuid v7, so it sorts by arrival). It is the primary
  -- key because it is also what an operator greps for: the handler logs it
  -- beside the row it wrote, and beside the failure when the write did not
  -- land.
  request_id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  alert_source_id uuid REFERENCES alert_sources(id) ON DELETE SET NULL,
  -- The routing id from the URL, kept verbatim: it is what the caller used.
  source_ref text NOT NULL,
  -- Which door the request came to. The same closed pair as the counter's
  -- `source` attribute (`core_otel::metrics::INGEST_SOURCES`), so a source
  -- that posts to the wrong door is visible as such instead of looking like
  -- traffic that vanished.
  endpoint text NOT NULL CHECK (endpoint IN ('alert', 'webhook')),
  -- When the endpoint received it. Deliberately not the envelope's
  -- `occurred_at`: a provider's clock is not evidence of arrival, and "went
  -- quiet" is a question about arrival.
  received_at timestamptz NOT NULL,
  -- The status the endpoint returned to the caller.
  http_status smallint NOT NULL CHECK (http_status BETWEEN 100 AND 599),
  -- What an accepted delivery did: `emitted`, `duplicate`, `collapsed`,
  -- `incident_created`. NULL on a refusal, whose answer is `refusal_reason`.
  outcome text,
  -- Why a delivery was refused: `rate_limited`, `validation_failed`,
  -- `conflict`, `not_found`, `forbidden`, `internal`. Taken from the status
  -- the caller received, so the log cannot report something the caller never
  -- saw.
  refusal_reason text,
  -- The request body's length. The body itself is never stored: it is the
  -- provider's data, and `outbox_events` or `alerts` already hold whatever
  -- was accepted.
  byte_count integer NOT NULL CHECK (byte_count >= 0),
  -- The provider's idempotency key and entity, when the document parsed far
  -- enough to carry them. NULL on a delivery that never parsed.
  dedup_key text,
  entity_id text,
  -- 2xx and 4xx are exclusive states and each owns exactly one of the two
  -- answer columns. An edit that fills both, or neither, fails here rather
  -- than in a reader that has to guess which one to believe.
  CONSTRAINT ingest_requests_outcome_matches_status
    CHECK ((http_status < 400) = (outcome IS NOT NULL)),
  CONSTRAINT ingest_requests_refusal_matches_status
    CHECK ((http_status < 400) = (refusal_reason IS NULL))
);

-- The reader's walk: one source's log, newest first, keyset on
-- (received_at, request_id). The same index answers "when did this source
-- last send anything" — the newest row is the first row.
CREATE INDEX ingest_requests_source_idx
  ON ingest_requests (org_id, alert_source_id, received_at DESC, request_id DESC);

-- The retention sweep's scan: rows past the window, oldest first. Leading on
-- time because the sweep is cross-org and the reader's index cannot serve it.
CREATE INDEX ingest_requests_received_idx
  ON ingest_requests (received_at);

ALTER TABLE ingest_requests ENABLE ROW LEVEL SECURITY;
CREATE POLICY ingest_requests_org ON ingest_requests
  USING (org_id = current_setting('app.org_id', true)::uuid);

COMMENT ON TABLE ingest_requests IS
  'One row per inbound ingest request the org_ingest counter counted: the '
  'source, the arrival time, the status the endpoint returned, and whether it '
  'was accepted or refused and why. Written after the request is handled — a '
  'failed write is logged and the ingest answer is unchanged — and deleted by '
  'the retention sweep, never by a surface.';
