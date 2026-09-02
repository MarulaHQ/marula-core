-- Alert ingestor (TASKLIST Phase 3): HMAC webhook ingest →
-- dedup/correlation → incident tickets (a storm collapses to ONE ticket).
--
-- Three org-scoped tables:
--
--   * alert_sources — per-org ingest configuration. `source_ref` is the
--     public routing identifier embedded in the ingest URL and is GLOBALLY
--     unique, because it resolves which org (and secret) a request belongs
--     to before any authentication exists — the same cross-org posture as
--     boards.inbound_address in 0014, served by a narrow SECURITY DEFINER
--     lookup instead of a role grant (0014's rationale applies verbatim).
--     The per-source HMAC secret is sealed at rest (AES-256-GCM, app
--     level — same scheme as webhooks in 0020) and never returned after
--     creation.
--
--   * alerts — the normalized alert ledger. UNIQUE (org_id, source_id,
--     dedup_key) is the replay gate: a provider re-delivering the same
--     alert (same dedup_key) can never double-apply — the original row is
--     returned and nothing else happens.
--
--   * alert_incidents — the storm-collapse groups. One row per
--     (source, correlation_key) incident window anchored to exactly ONE
--     incident ticket. A partial unique index enforces at most one OPEN
--     group per key, so a burst of related alerts folds into the ticket
--     the anchor alert created; the group re-opens a fresh incident only
--     after the source's correlation window has elapsed.
--
-- Non-purpose: no RMM provider payload dialects (adapters normalize at
-- the edge into the canonical ingest document), no queue/worker
-- involvement (correlation runs synchronously inside the ingest route),
-- no alert-driven escalation ladders (SLA policy on the created ticket
-- already covers paging).
-- Customization constructs (boards/statuses/fields) are only REFERENCED,
-- never altered — U9 cross-check gate not triggered.

CREATE TABLE alert_sources (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  -- Public ingest routing id (path segment). Globally unique: it is how
  -- an unauthenticated webhook resolves the org before it can know it.
  source_ref text NOT NULL,
  -- AES-256-GCM ciphertext (nonce || ct) of the HMAC signing secret.
  -- Never selected back except by the verification path; never logged,
  -- never returned after create.
  secret_encrypted bytea NOT NULL,
  -- Board the incident tickets land on.
  board_id uuid NOT NULL REFERENCES boards(id) ON DELETE RESTRICT,
  -- Optional client context stamped onto created incident tickets.
  client_id uuid REFERENCES clients(id) ON DELETE SET NULL,
  -- Storm window: a related alert (same correlation_key) within this
  -- many seconds of the group's last alert collapses into the open
  -- incident ticket instead of creating a new one.
  correlation_window_secs integer NOT NULL DEFAULT 900
    CHECK (correlation_window_secs BETWEEN 10 AND 86400),
  enabled boolean NOT NULL DEFAULT true,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX idx_alert_sources_ref ON alert_sources (source_ref);
CREATE INDEX alert_sources_org_idx ON alert_sources (org_id);

CREATE TABLE alerts (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  source_id uuid NOT NULL REFERENCES alert_sources(id) ON DELETE CASCADE,
  -- Provider's per-event identity; unique per source so a replayed
  -- webhook delivery is inert (the original row answers).
  dedup_key text NOT NULL,
  -- Storm grouping key (device/monitor/check identity). Empty string is
  -- never stored: the ingest path defaults it to the dedup_key.
  correlation_key text NOT NULL,
  title text NOT NULL,
  description text,
  severity text NOT NULL CHECK (severity IN ('info','warning','critical')),
  -- When the provider says the condition started (defaults to receipt).
  occurred_at timestamptz NOT NULL,
  attributes jsonb NOT NULL DEFAULT '{}',
  -- The incident ticket this alert folded into (set on ingest when the
  -- anchor ticket was created; never rewritten afterwards).
  incident_ticket_id uuid REFERENCES tickets(id) ON DELETE SET NULL,
  received_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, source_id, dedup_key)
);

-- Newest-first browse + per-ticket rollups.
CREATE INDEX idx_alerts_received ON alerts (org_id, received_at DESC);
CREATE INDEX idx_alerts_ticket ON alerts (org_id, incident_ticket_id)
  WHERE incident_ticket_id IS NOT NULL;

CREATE TABLE alert_incidents (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  source_id uuid NOT NULL REFERENCES alert_sources(id) ON DELETE CASCADE,
  correlation_key text NOT NULL,
  -- The ONE ticket the storm collapsed into.
  ticket_id uuid NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
  alert_count integer NOT NULL DEFAULT 1 CHECK (alert_count >= 1),
  first_alert_at timestamptz NOT NULL,
  -- Rolling window anchor: every collapsed alert pushes this forward.
  last_alert_at timestamptz NOT NULL DEFAULT now(),
  closed_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now()
);

-- At most one OPEN group per (source, correlation_key): the storm rule
-- stated as a constraint. Expired groups get closed_at stamped before a
-- replacement opens.
CREATE UNIQUE INDEX idx_alert_incidents_open
  ON alert_incidents (org_id, source_id, correlation_key)
  WHERE closed_at IS NULL;
CREATE INDEX idx_alert_incidents_ticket ON alert_incidents (ticket_id);

ALTER TABLE alert_sources ENABLE ROW LEVEL SECURITY;
CREATE POLICY alert_sources_org ON alert_sources
  USING (org_id = NULLIF(current_setting('app.org_id', true), '')::uuid);
ALTER TABLE alerts ENABLE ROW LEVEL SECURITY;
CREATE POLICY alerts_org ON alerts
  USING (org_id = NULLIF(current_setting('app.org_id', true), '')::uuid);
ALTER TABLE alert_incidents ENABLE ROW LEVEL SECURITY;
CREATE POLICY alert_incidents_org ON alert_incidents
  USING (org_id = NULLIF(current_setting('app.org_id', true), '')::uuid);

-- Cross-org seam for the ingest path (0014 pattern): the request cannot
-- know its org before the source_ref resolves, so the constrained
-- `meridian_app` role gets EXECUTE on one single-purpose lookup and
-- NOTHING else crosses org boundaries. The ref column is UNIQUE, so
-- lookups are exact-match by construction.
CREATE FUNCTION alert_source_auth_lookup(p_source_ref text)
RETURNS TABLE (
  r_id uuid,
  r_org_id uuid,
  r_source_ref text,
  r_board_id uuid,
  r_client_id uuid,
  r_secret_encrypted bytea,
  r_correlation_window_secs integer,
  r_enabled boolean
)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public AS $$
  SELECT id, org_id, source_ref, board_id, client_id, secret_encrypted,
         correlation_window_secs, enabled
  FROM alert_sources
  WHERE source_ref = p_source_ref
$$;

GRANT EXECUTE ON FUNCTION alert_source_auth_lookup(text) TO meridian_app;
