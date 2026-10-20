-- Instant-first write-path contract (the private build tracker IF1–IF3).
--
-- Purpose: the durable substrate under idempotent autosave writes and the
-- read-your-write response. Three org-scoped tables, all RLS-bound to the
-- caller's org exactly like every other tenant table (0001 convention):
--
--   * entity_write_seqs — the per-entity monotonic counter. One row per
--     (org, entity, entity_id); the upsert-increment takes the row lock, so
--     concurrent writers to the same entity serialize their seq values
--     without contending across entities. The seq doubles as the entity
--     `version` clients echo back for If-Match-style semantics.
--
--   * entity_writes — the append-only write trail. One row per applied
--     write carrying the seq, the operation, the actor, and the before and
--     after entity snapshots (jsonb). This is the undo substrate the
--     instant-first contract names: the response's prior value and new
--     value come from here, and per-field/per-record undo history (IF7)
--     reads it rather than re-deriving from audit. Append-only at the app
--     layer: no code path updates or deletes rows (same posture as
--     audit_events); the audit trail stays the security authority and this
--     trail stays the interaction authority — they cooperate, never alias.
--
--   * idempotency_keys — the replay gate. UNIQUE (org_id, key) is the whole
--     contract: a create/update carrying an idempotency key claims its row
--     transactionally, stores the request fingerprint (SHA-256 over the
--     canonical request body), and persists the exact response so a
--     debounce/retry replay returns the SAME result byte-for-byte and can
--     never double-apply. Same key with a different payload is a typed 409,
--     not a silent second write.
--
-- Non-purpose: not an event bus replacement (outbox events keep flowing for
-- the WS relay), not a security audit (audit_events keeps that role), and
-- not a read model (no ops.v1 view yet — undo history queries are IF7).
--
-- Provenance: entity_writes.actor_kind reuses the exact audit_events
-- vocabulary (0001 + 0021 + 0027) so trail rows join audit rows on actor
-- semantics; the write path currently emits 'human' only, agent-authored
-- writes arrive through the approval pipeline (IF8) as normal audited
-- writes.

CREATE TABLE entity_write_seqs (
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  entity text NOT NULL,
  entity_id uuid NOT NULL,
  seq bigint NOT NULL DEFAULT 0,
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (org_id, entity, entity_id)
);

CREATE TABLE entity_writes (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  entity text NOT NULL,
  entity_id uuid NOT NULL,
  seq bigint NOT NULL,
  op text NOT NULL,
  actor_id uuid NOT NULL,
  actor_kind text NOT NULL DEFAULT 'human'
    CHECK (actor_kind IN ('human','agent','plugin','connector','system',
                          'portal_contact','portal_approver','portal_it_manager',
                          'import')),
  idempotency_key text,
  -- Prior entity snapshot; NULL for creates (there is no prior value).
  before jsonb,
  after jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, entity, entity_id, seq)
);
-- Undo-history reads walk one entity's trail newest-first.
CREATE INDEX idx_entity_writes_trail
  ON entity_writes (org_id, entity, entity_id, created_at DESC);
-- "Who changed this record" provenance lookups (management intelligence).
CREATE INDEX idx_entity_writes_actor
  ON entity_writes (org_id, actor_id, created_at DESC);

CREATE TABLE idempotency_keys (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  key text NOT NULL,
  entity text NOT NULL,
  entity_id uuid,
  -- SHA-256 hex over the canonical request body; same key + different
  -- payload is rejected instead of replayed or re-applied.
  request_hash text NOT NULL,
  -- The exact serialized mutation response, stored at commit and replayed
  -- verbatim; NULL only between the claim and the commit of the first
  -- in-flight request with this key.
  response jsonb,
  actor_id uuid NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, key)
);

ALTER TABLE entity_write_seqs ENABLE ROW LEVEL SECURITY;
CREATE POLICY entity_write_seqs_org ON entity_write_seqs
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE entity_writes ENABLE ROW LEVEL SECURITY;
CREATE POLICY entity_writes_org ON entity_writes
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE idempotency_keys ENABLE ROW LEVEL SECURITY;
CREATE POLICY idempotency_keys_org ON idempotency_keys
  USING (org_id = current_setting('app.org_id', true)::uuid);
