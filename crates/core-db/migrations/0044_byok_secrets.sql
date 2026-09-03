-- BYOK Secrets & AI Settings (TASKLIST Phase 3, lane `p9-adapters-export`; migration 0044).
--
-- Purpose: per-org sealed secret storage for Bring-Your-Own-Key (BYOK) AI provider
-- configurations (`CompletionProvider` seam).
--
-- Pattern follows `oidc_providers` (0013) and `alert_sources` (0037):
--   * Secrets are encrypted at rest with AES-256-GCM (app-level master secret key,
--     12-byte nonce prepended to ciphertext: nonce || ct).
--   * Secret bytes are never selected back to client APIs, never logged, and never
--     surfaced beyond backend provider execution / test probes.
--   * Tenancy is strictly enforced with RLS (`app.org_id`).
--
-- Non-purpose: no model catalog management (model ids are validated strings),
-- no persistent prompt/token logs (auditing records provenance metadata, not secret payloads).

CREATE TABLE byok_secrets (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- Provider identifier: 'openai', 'anthropic', 'custom', etc.
  provider text NOT NULL,
  -- AES-256-GCM ciphertext (nonce || ct) of the API key / bearer token.
  secret_encrypted bytea NOT NULL,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, provider)
);

CREATE INDEX idx_byok_secrets_org ON byok_secrets (org_id);

ALTER TABLE byok_secrets ENABLE ROW LEVEL SECURITY;
CREATE POLICY byok_secrets_org ON byok_secrets
  USING (org_id = current_setting('app.org_id', true)::uuid);
