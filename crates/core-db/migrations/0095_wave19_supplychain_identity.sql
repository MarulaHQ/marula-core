-- Wave 19 lane 2 `p19-supplychain-identity` — ADR 0009 full + ADR 0006 remainder.
--
-- Purpose: the supply-chain trust model gets a rotation-capable signing-key
-- registry. Migration 0079 (Wave 16 lane 7) verified distribution artifacts
-- against ONE operator key carried in process config
-- (`MERIDIAN_PLUGIN_SIGNING_KEY`); a single immutable key made rotation a
-- config edit that stranded every previously-signed artifact the moment the
-- key changed. This migration adds `plugin_signing_keys` — the GLOBAL
-- key-trust store (server-wide like `plugin_catalog`, no org_id, no RLS —
-- the same global-registry doctrine 0076 documents) — with:
--
--   * multiple trusted keys at once: rotation = add the new key, let
--     operators re-sign or dual-sign, RETIRE the old key (retired keys
--     still VERIFY artifacts already in the chain — a retired key is past
--     trust, not distrust), REVOKE only on compromise (revoked keys verify
--     nothing, retroactively poisoning accepts is the point);
--   * stable key IDs: the signature block names `key_id` explicitly, so a
--     verifier resolves EXACTLY one key instead of guessing; an artifact's
--     key_id is a trust identity, never secret material;
--   * the legacy single env key stays honored: at runtime the fallback hook
--     unions the registry rows with the legacy env key (implicit derived
--     key_id), so an operator upgrading past 0095 keeps a green pipeline
--     until they complete the runbook migration (docs/reference/
--     plugin-signing-rotation-runbook.md).
--
-- No other Wave-19-lane-2 schema work lands here: junior-role restricted-
-- chain defaults are seeded GRANT ROWS (secret_category_grants, migration
-- 0078 — the seeds are data by construction, the per-role posture is code
-- doctrine), the connector governance profile is a modeled FIELD inside the
-- already-JSONB manifest documents, and `vault:ingress` is a closed-
-- vocabulary capability key (code, not schema). Audit trails for those ride
-- the existing append-only audit_events + provenance chains.

CREATE TYPE plugin_signing_key_status AS ENUM ('active', 'retired', 'revoked');

CREATE TABLE plugin_signing_keys (
  id uuid PRIMARY KEY,
  -- The stable trust identity named INSIDE the signature block. Defaults
  -- to the first 16 hex chars of the public key unless the operator names
  -- it; rotation runbook pins the convention.
  key_id text NOT NULL,
  -- The ed25519 PUBLIC key (64 hex chars) — verification material only.
  -- Private keys NEVER live here or in the vault (the operator keeps them
  -- in their signing environment; the CLI surface imports a public key).
  public_key text NOT NULL,
  status plugin_signing_key_status NOT NULL DEFAULT 'active',
  label text NOT NULL DEFAULT '',
  created_at timestamptz NOT NULL DEFAULT now(),
  retired_at timestamptz,
  revoked_at timestamptz,
  revoked_reason text,
  CHECK (public_key ~ '^[0-9a-f]{64}$'),
  CHECK (key_id ~ '^[a-z0-9][a-z0-9._\-]{0,63}$'),
  CHECK (revoked_at IS NULL OR revoked_reason IS NOT NULL),
  CHECK (retired_at IS NULL OR status = 'retired'),
  CHECK (revoked_at IS NULL OR status = 'revoked')
);

-- The constraint name is consumed by the typed reason-code registry:
-- `signing_key_id_taken`.
CREATE UNIQUE INDEX plugin_signing_keys_key_id_key
  ON plugin_signing_keys (key_id);

-- Guard-text note for operators reading the table directly (no code path):
-- a key_id row that becomes 'revoked' turns every artifact citing it away,
-- which is the semantic compromise response — the runbook warns about it.
COMMENT ON TABLE plugin_signing_keys IS
  'GLOBAL operator key-trust store for plugin/skill distribution artifact
signatures (Wave 19 lane 2, ADR 0009 rotation semantics). Multiple keys,
stable key_ids in the signature block, retired-overlap rotation. Server-wide
registry data like plugin_catalog — no org_id, no RLS.';
