-- Core-side entitlement artifact adoption (RFC 0023).
--
-- Purpose: the store behind `PUT /api/v1/orgs/{org_id}/entitlements`, which
-- adopts a SIGNED, revision-stamped entitlement artifact and reports the
-- revision it holds. One row per org: the artifact is a fact about what the
-- tenant bought elsewhere, so this table keeps the current fact, not a log of
-- every artifact ever offered (the audit row and the domain event carry the
-- history).
--
-- Why a signed artifact and not a push endpoint: a live push would make Core a
-- second source of truth for commercial facts and would leave an endpoint that
-- no self-hosted install has a caller for. An artifact verifies offline, so an
-- air-gapped install works and HQ keeps its own truth.
--
-- `payload` is deliberately opaque JSONB. Core verifies, stores and reports;
-- it does not interpret flags, seats or allowances, and nothing in core gates
-- a feature on this row (the entitlement-gating law). Interpreting the payload
-- belongs to whoever bought it.
--
-- `payload_sha256` and `artifact_sha256` are the digests the signature covered:
-- the payload digest lets a caller compare what it was sold against what was
-- stored, and the artifact digest is the exact signed envelope, kept so a
-- refusal can be explained later without re-deriving it.
--
-- Revision monotonicity is enforced by the write statement (the candidate must
-- exceed the stored revision), not by a trigger: the same guard also answers
-- "is this a replay or a conflict", which a trigger could not.
--
-- Non-purpose: no entitlement interpretation, no feature gating, no pricing or
-- commerce columns, and no history table — a superseded artifact is not a fact
-- worth keeping, and a second source of truth is what RFC 0023 rejected.

CREATE TABLE org_entitlements (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL UNIQUE REFERENCES orgs(id) ON DELETE CASCADE,
  -- The issuing side's monotonic revision. Strictly increasing per org.
  revision bigint NOT NULL,
  -- The signed payload, stored verbatim and never interpreted here.
  payload jsonb NOT NULL,
  payload_sha256 text NOT NULL,
  -- The exact signed envelope (artifact object, without the signature block).
  artifact jsonb NOT NULL,
  artifact_sha256 text NOT NULL,
  -- Which trust identity signed it, as verified. Audit material, not a gate:
  -- verification runs against the whole trust set, never against this column.
  key_id text,
  -- When the artifact says it was issued; null when it omitted the field.
  issued_at timestamptz,
  adopted_by uuid REFERENCES users(id) ON DELETE SET NULL,
  adopted_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT org_entitlements_revision_positive CHECK (revision > 0),
  CONSTRAINT org_entitlements_payload_sha256_hex
    CHECK (payload_sha256 ~ '^[0-9a-f]{64}$'),
  CONSTRAINT org_entitlements_artifact_sha256_hex
    CHECK (artifact_sha256 ~ '^[0-9a-f]{64}$')
);

ALTER TABLE org_entitlements ENABLE ROW LEVEL SECURITY;
CREATE POLICY org_entitlements_org ON org_entitlements
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

COMMENT ON TABLE org_entitlements IS
  'The signed entitlement artifact each org currently holds (RFC 0023): verified, stored, reported — never interpreted.';
