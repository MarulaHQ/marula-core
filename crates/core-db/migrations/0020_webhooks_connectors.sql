-- Stage 2 / item 2.15 — plugin runtime v1 storage: signed webhooks OUT
-- and connector installations.
--
-- Purpose: org-scoped tables completing the extension surface (§4.6).
-- `webhooks` holds subscription endpoints with their HMAC secret sealed
-- at rest (AES-256-GCM, app level); `webhook_deliveries` is the per-ATTEMPT
-- delivery ledger the in-app logs render (unique on webhook+event+attempt,
-- so at-least-once workers can never double-log);
-- `connector_installations` records which first-party connectors an org
-- consented to, with config sealed and capability scopes captured.
--
-- `api_keys` is NOT touched here: it shipped in 0001 (RLS policy included)
-- with exactly this item's required columns — name, key_hash, scopes,
-- created_by, last_used_at, revoked_at — plus a display `prefix`. Item
-- 2.15 only wires the hashing/scoping/last-used semantics through the API.
--
-- Non-purpose: no plugin/WASM registry (Stage 4), no delivery payloads
-- (the event stream is the payload source of truth — deliveries reference
-- the envelope id), no OAuth connection records (Stage 3 adapters).

CREATE TABLE webhooks (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  url text NOT NULL,
  -- AES-256-GCM ciphertext: nonce || ct. Never selected back except by
  -- the signing path; never logged, never returned after create.
  secret_encrypted bytea NOT NULL,
  -- Empty array = wildcard (all event types). Otherwise exact match set.
  event_types text[] NOT NULL DEFAULT '{}',
  enabled boolean NOT NULL DEFAULT true,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX webhooks_org_idx ON webhooks (org_id);

CREATE TABLE webhook_deliveries (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  webhook_id uuid NOT NULL REFERENCES webhooks(id) ON DELETE CASCADE,
  -- The outbox/event envelope id this attempt delivered.
  event_id uuid NOT NULL,
  event_type text NOT NULL,
  -- 1-based attempt counter; unique per (webhook, event) so a retried
  -- worker can never log the same attempt twice.
  attempt integer NOT NULL CHECK (attempt >= 1),
  -- HTTP status when a response arrived; NULL for transport failures.
  response_status integer,
  next_retry_at timestamptz,
  delivered_at timestamptz,
  last_error text,
  created_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT webhook_deliveries_attempt_unique UNIQUE (webhook_id, event_id, attempt)
);

-- Newest-first walk for the in-app delivery log.
CREATE INDEX webhook_deliveries_page_idx
  ON webhook_deliveries (webhook_id, created_at DESC, id DESC);

CREATE TABLE connector_installations (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- Connector type slug ("ninjaone", "hudu"); one installation per type.
  connector_type text NOT NULL,
  -- AES-256-GCM ciphertext of the connector's credential JSON.
  config_encrypted bytea NOT NULL,
  -- Capability scopes the org consented to at install time.
  capability_scopes text[] NOT NULL DEFAULT '{}',
  consented_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT connector_installations_type_unique UNIQUE (org_id, connector_type)
);

-- RLS on every new table (CI gate parses migrations): default-deny unless
-- the transaction is bound to the row's org via `app.org_id`. Both GUCs go
-- through NULLIF so never-set sessions yield NULL and defined-then-reset
-- pooled sessions yield '' without raising 22P02.

ALTER TABLE webhooks ENABLE ROW LEVEL SECURITY;
CREATE POLICY webhooks_org ON webhooks
  USING (org_id = NULLIF(current_setting('app.org_id', true), '')::uuid);

ALTER TABLE webhook_deliveries ENABLE ROW LEVEL SECURITY;
CREATE POLICY webhook_deliveries_org ON webhook_deliveries
  USING (org_id = NULLIF(current_setting('app.org_id', true), '')::uuid);

ALTER TABLE connector_installations ENABLE ROW LEVEL SECURITY;
CREATE POLICY connector_installations_org ON connector_installations
  USING (org_id = NULLIF(current_setting('app.org_id', true), '')::uuid);

-- Narrow SECURITY DEFINER seams for the x-api-key auth path (same pattern
-- as the email lane): request authentication cannot know the org before
-- the key resolves, so the constrained `meridian_app` role gets EXECUTE on
-- two single-purpose functions and NOTHING else crosses org boundaries.
-- The hash column is UNIQUE, so lookups are exact-match by construction.
CREATE FUNCTION api_key_auth_lookup(p_key_hash text)
RETURNS TABLE (
  id uuid,
  org_id uuid,
  name text,
  key_hash text,
  prefix text,
  scopes jsonb,
  created_by uuid,
  last_used_at timestamptz,
  revoked_at timestamptz,
  created_at timestamptz
)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public AS $$
  SELECT id, org_id, name, key_hash, prefix, scopes, created_by,
         last_used_at, revoked_at, created_at
  FROM api_keys
  WHERE key_hash = p_key_hash AND revoked_at IS NULL
$$;

CREATE FUNCTION api_key_touch_last_used(p_key_id uuid, p_min_interval_secs integer)
RETURNS void
LANGUAGE sql SECURITY DEFINER SET search_path = public AS $$
  UPDATE api_keys SET last_used_at = now()
  WHERE id = p_key_id
    AND (last_used_at IS NULL
         OR last_used_at < now() - make_interval(secs => p_min_interval_secs));
$$;

GRANT EXECUTE ON FUNCTION api_key_auth_lookup(text) TO meridian_app;
GRANT EXECUTE ON FUNCTION api_key_touch_last_used(uuid, integer) TO meridian_app;
