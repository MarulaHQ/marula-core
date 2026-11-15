-- Two-step codes on a credential, and the document flags the portal needs
-- (surface-pack lane, migration 0218).
--
-- Purpose, part one: Hudu and IT Glue both let a credential carry an
-- authenticator seed, and the pack draws that card (K08) plus its setup
-- dialog (K16). `knowledge_secret_totp` stores the seed sealed with the same
-- key and the same coordinate-bound AAD as a password field
-- (`secret_fields`), so the seed is never in a list, a result or a log — only
-- the audited code response ever produces digits. One seed per record.
--
-- Purpose, part two: two columns on `documents` that the client-first build
-- needs and the existing docs platform never carried.
--
--   * `client_visible` — a document the client portal may show. Off by
--     default, because visible-to-client is a decision, not an inheritance.
--   * `expires_at`      — the review-by date IT Glue calls document
--     expiration, so a policy that goes stale shows up in the same expiry
--     rollup as a certificate.
--
-- Purpose, part three: an index for rotation age. `secret_records` already
-- carries `last_rotated_at` (migration 0096), and the hub reads THAT column
-- rather than `updated_at` — a metadata edit is not a rotation. No second
-- column is added; a parallel rotation stamp would be a second truth.
--
-- RLS on the new table; the document columns inherit their table's policy.

CREATE TABLE knowledge_secret_totp (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  record_id uuid NOT NULL REFERENCES secret_records(id) ON DELETE CASCADE,
  -- Sealed seed. Never selected by a metadata query; the route unseals it
  -- only inside the audited code response.
  seed_enc bytea NOT NULL,
  digits smallint NOT NULL DEFAULT 6 CHECK (digits IN (6, 8)),
  period_seconds smallint NOT NULL DEFAULT 30 CHECK (period_seconds BETWEEN 15 AND 120),
  algorithm text NOT NULL DEFAULT 'SHA1' CHECK (algorithm IN ('SHA1', 'SHA256', 'SHA512')),
  issuer text,
  account text,
  confirmed boolean NOT NULL DEFAULT true,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (record_id)
);

CREATE INDEX idx_knowledge_secret_totp_org
  ON knowledge_secret_totp (org_id, record_id);

ALTER TABLE knowledge_secret_totp ENABLE ROW LEVEL SECURITY;
CREATE POLICY knowledge_secret_totp_org ON knowledge_secret_totp
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE documents
  ADD COLUMN client_visible boolean NOT NULL DEFAULT false,
  ADD COLUMN expires_at timestamptz;

CREATE INDEX idx_documents_client_visible
  ON documents (org_id, client_id)
  WHERE client_visible;

CREATE INDEX IF NOT EXISTS idx_secret_records_last_rotated
  ON secret_records (org_id, client_id, last_rotated_at);
