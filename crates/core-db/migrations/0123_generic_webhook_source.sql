-- Generic M2M webhook receiver (Wave 43 lane 1): extends the alert-source
-- posture (sealed HMAC secret, board-scoped) into a first-class ingest path
-- that emits domain-event envelopes onto the outbox stream — NOT alert rows.
--
-- The `alert_sources` table is the single source registry. A source with
-- `event_type IS NOT NULL` is a generic M2M receiver; `event_type IS NULL`
-- keeps it an alert source (Wave 9). No second source table, no fork.
--
-- Two new columns:
--   * event_type — the domain event type the source emits (e.g. "order.created").
--     Checked against the contract registry when the source is created.
--   * payload_schema_ref — pins the payload contract (defaults to the
--     same 'core-domain/v1' floor the outbox already uses).
--
-- Replay gate + rate limit share one ledger table: a UNIQUE (org, source,
-- dedup_key) makes a replayed delivery a no-op (the insert fails silently);
-- counting recent rows per source enforces a per-source rate ceiling.

ALTER TABLE alert_sources
    ADD COLUMN event_type text,
    ADD COLUMN payload_schema_ref text;

-- Extend the cross-org auth lookup so the generic ingest handler resolves
-- the declared event + schema in the same seam as the alert handler. The
-- two new columns are non-secret config — same posture as board_id/client_id.
DROP FUNCTION alert_source_auth_lookup(text);
CREATE FUNCTION alert_source_auth_lookup(p_source_ref text)
RETURNS TABLE (
    r_id uuid,
    r_org_id uuid,
    r_source_ref text,
    r_board_id uuid,
    r_client_id uuid,
    r_secret_encrypted bytea,
    r_correlation_window_secs integer,
    r_enabled boolean,
    r_event_type text,
    r_payload_schema_ref text
)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public AS $$
    SELECT id, org_id, source_ref, board_id, client_id, secret_encrypted,
           correlation_window_secs, enabled, event_type, payload_schema_ref
    FROM alert_sources
    WHERE source_ref = p_source_ref
$$;
GRANT EXECUTE ON FUNCTION alert_source_auth_lookup(text) TO meridian_app;

-- Replay gate + rate-limit ledger: one row per accepted ingest. The UNIQUE
-- constraint is the dedup fence; queries on occurred_at power the rate ceiling.
CREATE TABLE webhook_ingest_dedup (
    org_id uuid NOT NULL,
    source_id uuid NOT NULL REFERENCES alert_sources(id) ON DELETE CASCADE,
    dedup_key text NOT NULL,
    occurred_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (org_id, source_id, dedup_key)
);
CREATE INDEX idx_webhook_ingest_rate ON webhook_ingest_dedup (org_id, source_id, occurred_at DESC);

ALTER TABLE webhook_ingest_dedup ENABLE ROW LEVEL SECURITY;
CREATE POLICY webhook_ingest_dedup_org ON webhook_ingest_dedup
    USING (org_id = NULLIF(current_setting('app.org_id', true), '')::uuid);
