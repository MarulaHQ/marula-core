-- Agent credentials: an API key may be bound to a service account.
--
-- Why: an MCP (or any header-based) machine caller needs an identity with
-- explicit permissions. Service accounts already carry that vocabulary
-- (`tickets:view`), a status, an expiry and a revocation reason, but they
-- authenticate by client certificate. Binding a key to one gives the same
-- grants a header-based credential, with the account as the actor.
--
-- Additive and reversible: every existing key keeps service_account_id NULL
-- and resolves exactly as it does today.

ALTER TABLE api_keys
  ADD COLUMN service_account_id uuid REFERENCES service_accounts(id) ON DELETE CASCADE;

CREATE INDEX idx_api_keys_service_account
  ON api_keys (service_account_id)
  WHERE service_account_id IS NOT NULL;

-- The auth lookup returns the new column. The return type changes, so the
-- function is replaced rather than redeclared.
DROP FUNCTION api_key_auth_lookup(text);

CREATE FUNCTION api_key_auth_lookup(p_key_hash text)
RETURNS TABLE (
  id uuid,
  org_id uuid,
  name text,
  key_hash text,
  prefix text,
  scopes jsonb,
  service_account_id uuid,
  created_by uuid,
  last_used_at timestamptz,
  revoked_at timestamptz,
  created_at timestamptz
)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public AS $$
  SELECT id, org_id, name, key_hash, prefix, scopes, service_account_id,
         created_by, last_used_at, revoked_at, created_at
  FROM api_keys
  WHERE key_hash = p_key_hash AND revoked_at IS NULL
$$;

-- One active service account by id, for the key-bound auth path. Runs before
-- any org is bound, so it is SECURITY DEFINER with the same active/expiry
-- filter the certificate path uses. Returns no row for unknown, suspended,
-- revoked or expired accounts (deny-by-default).
CREATE FUNCTION service_account_key_lookup(p_id uuid)
RETURNS TABLE (
  id uuid,
  org_id uuid,
  name text,
  cert_fingerprint text,
  cert_subject text,
  scopes jsonb,
  status text,
  expires_at timestamptz,
  revoked_at timestamptz,
  revocation_reason text,
  last_used_at timestamptz,
  created_by uuid,
  created_at timestamptz
)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public AS $$
  SELECT id, org_id, name, cert_fingerprint, cert_subject, scopes, status,
         expires_at, revoked_at, revocation_reason, last_used_at, created_by, created_at
  FROM service_accounts
  WHERE id = p_id
    AND revoked_at IS NULL
    AND status = 'active'
    AND (expires_at IS NULL OR expires_at > now())
$$;

GRANT EXECUTE ON FUNCTION api_key_auth_lookup(text) TO marula_app;
GRANT EXECUTE ON FUNCTION service_account_key_lookup(uuid) TO marula_app;

-- Down (by hand, never run by the migrator): drop the two functions, restore
-- the pre-0179 api_key_auth_lookup without service_account_id, drop the index
-- and the column.
