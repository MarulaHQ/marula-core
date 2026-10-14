-- Client certificate authentication (Wave 47 lane A9, `p47-client-cert-auth`):
-- service accounts authenticate by a presented X.509 client certificate.
--
-- Purpose: v1 binds a service account to the SHA-256 fingerprint of the
-- enrolled client certificate. Every real deployment terminates TLS at the
-- edge proxy (V2 edge-TLS), which forwards the VERIFIED fingerprint in a
-- header; core-api stays HTTP-only and matches that fingerprint here. A
-- cert row selects exactly one service account in exactly one org, so a
-- cert enrolled for org A can never authenticate into org B — the row's
-- org_id is the org and its `scopes` are the granted permissions.
--
-- Non-purpose / documented follow-ups: no CA-bundle validation and no
-- SAN/custom-extension org-binding extraction in v1 (full CA-chain
-- validation is the next step, see design/qa-p47-client-cert-auth/LANDING.md);
-- no direct TLS termination at core-api (the edge proxy is the only
-- presentation path in v1). Grant strings are validated against the
-- EXISTING policy matrix at write time in the API layer, never here.

CREATE TABLE service_accounts (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- Admin-facing label (1..=120 chars, validated in the API layer).
  name text NOT NULL,
  -- Lowercase hex SHA-256 of the presented client certificate's DER. The
  -- v1 binding is fingerprint-PINNED: rotation enrolls a NEW fingerprint
  -- and revokes the old row. GLOBALLY UNIQUE so one cert resolves to
  -- exactly one service account in exactly one org — there is no ambiguity
  -- an attacker could exploit across orgs.
  cert_fingerprint text NOT NULL UNIQUE,
  -- Display-only subject DN carried from the enrolled certificate. NEVER
  -- used for matching (the fingerprint is), so a re-issued cert with the
  -- same fingerprint but a different subject cannot fork the binding.
  cert_subject text,
  -- Granted permission keys (["tickets:read", "special:manage_api_keys", ...]).
  -- Validated against the existing policy matrix at write time; the cert
  -- auth path resolves these strings to the principal's permission set.
  scopes jsonb NOT NULL DEFAULT '[]',
  -- Admin lifecycle: 'active' accepts auth; 'suspended' refuses it. A
  -- separate `revoked_at` records hard revocation (immediate + permanent).
  status text NOT NULL DEFAULT 'active',
  -- Optional expiry. NULL = never expires; otherwise cert auth is refused
  -- past this instant. The listing derives valid/expired/revoked state.
  expires_at timestamptz,
  revoked_at timestamptz,
  revocation_reason text,
  last_used_at timestamptz,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT service_accounts_status_chk CHECK (status IN ('active', 'suspended'))
);

-- Org-scoped admin listing walk (newest-first).
CREATE INDEX service_accounts_org_idx ON service_accounts (org_id, created_at DESC, id DESC);

-- RLS on the table (CI gate parses migrations): default-deny unless the
-- transaction is bound to the row's org via `app.org_id`. NULLIF keeps a
-- never-set session (NULL) and a defined-then-reset pooled session ('')
-- from raising 22P02 on the ::uuid cast — same shape as every org table.
ALTER TABLE service_accounts ENABLE ROW LEVEL SECURITY;
CREATE POLICY service_accounts_org ON service_accounts
  USING (org_id = NULLIF(current_setting('app.org_id', true), '')::uuid);

-- Narrow SECURITY DEFINER seams for the cert-auth path (same pattern as
-- api_keys in migration 0020): request authentication resolves the cert
-- fingerprint BEFORE any org scope could exist, so the constrained
-- `marula_app` role gets EXECUTE on two single-purpose functions and
-- nothing else crosses org boundaries. The lookup is STABLE and only
-- resolves ACTIVE, non-revoked, unexpired rows — revoked/expired/suspended
-- certificates are simply absent (deny-by-default).
CREATE FUNCTION service_account_auth_lookup(p_fingerprint text)
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
  WHERE cert_fingerprint = p_fingerprint
    AND revoked_at IS NULL
    AND status = 'active'
    AND (expires_at IS NULL OR expires_at > now())
$$;

-- Last-used tracking, throttled: only writes when the previous stamp is
-- absent or older than `p_min_interval_secs`, keeping the auth read cheap
-- on hot paths while still surfacing recent usage for the admin listing.
CREATE FUNCTION service_account_touch_last_used(p_id uuid, p_min_interval_secs integer)
RETURNS void
LANGUAGE sql SECURITY DEFINER SET search_path = public AS $$
  UPDATE service_accounts SET last_used_at = now()
  WHERE id = p_id
    AND (last_used_at IS NULL
         OR last_used_at < now() - make_interval(secs => p_min_interval_secs));
$$;

GRANT EXECUTE ON FUNCTION service_account_auth_lookup(text) TO marula_app;
GRANT EXECUTE ON FUNCTION service_account_touch_last_used(uuid, integer) TO marula_app;
