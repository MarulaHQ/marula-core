-- Provenance read model (TASKLIST IF9, doctrine #9).
--
-- Purpose: "who did this" is a query, not a sift. The instant-first write
-- trail (0028) already carries who (actor), what (entity + seq), and the
-- before/after snapshots — but as WHOLE-ENTITY jsonb blobs, so per-field
-- questions ("who changed ticket.status", "what did the agent write under
-- this skill") would require raw-row scans with query-time JSON diffing.
-- This migration materializes that projection ONCE, at write time:
--
--   * entity_writes gains the provenance context columns the IF1 envelope
--     write path fills "when present": the agent session, the skill that
--     authored the write, the rule-set hash that governed it (IF8/IF9),
--     and the undo flag (IF7 reverts ride the same idempotent path).
--     Additive columns; existing rows default to plain human writes.
--
--   * provenance_events — one row per (write x changed field), projected
--     by an AFTER INSERT trigger on entity_writes. Same append-only
--     posture as its source: nothing updates or deletes rows. The
--     projection is derived state; the write trail stays the interaction
--     authority and audit_events stays the security authority — this
--     table is the QUERY authority (doctrine #9: read models, never raw
--     scans). Rebuildable from entity_writes by construction.
--
-- actor_kind reuses the exact entity_writes/audit_events vocabulary
-- (0001 + 0021 + 0027); no separate CHECK here — the trigger copies an
-- already-constrained column, and a duplicated constraint would only add
-- a second place to drift. Doctrine's "undo" is a flag (is_undo), not an
-- actor kind: a revert is a normal authorized write BY someone.

ALTER TABLE entity_writes
  ADD COLUMN session_id uuid,
  ADD COLUMN skill text,
  ADD COLUMN ruleset_hash text,
  ADD COLUMN is_undo boolean NOT NULL DEFAULT false;

CREATE TABLE provenance_events (
  write_id uuid NOT NULL,
  -- 1..n position of the changed field within its write, ordered by field
  -- name — disambiguates the keyset cursor across a multi-field write.
  field_seq integer NOT NULL,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  entity text NOT NULL,
  entity_id uuid NOT NULL,
  seq bigint NOT NULL,
  op text NOT NULL,
  actor_kind text NOT NULL,
  actor_id uuid,
  -- The changed field (top-level entity key) and its prior/applied values.
  -- before is NULL when the field is new (creates, added keys).
  field text NOT NULL,
  before jsonb,
  after jsonb,
  -- Agent provenance "when present" (IF8-approved writes): the session,
  -- skill, and rule-set hash that governed the write. NULL for humans.
  session_id uuid,
  skill text,
  ruleset_hash text,
  is_undo boolean NOT NULL DEFAULT false,
  idempotency_key text,
  occurred_at timestamptz NOT NULL,
  PRIMARY KEY (write_id, field_seq)
);

-- The per-field diff every consumer of this projection shares: the union
-- of top-level keys whose values differ (missing key = SQL NULL, so an
-- added field projects before=NULL and a removed one after=NULL).
CREATE FUNCTION provenance_diff(p_before jsonb, p_after jsonb)
RETURNS TABLE (field text, prior jsonb, applied jsonb)
LANGUAGE sql STABLE
AS $fn$
  SELECT changed.key,
         p_before -> changed.key,
         p_after -> changed.key
  FROM (
    SELECT key FROM jsonb_object_keys(COALESCE(p_before, '{}'::jsonb)) AS k(key)
    UNION
    SELECT key FROM jsonb_object_keys(COALESCE(p_after, '{}'::jsonb)) AS k(key)
  ) changed
  WHERE (p_before -> changed.key) IS DISTINCT FROM (p_after -> changed.key)
  ORDER BY changed.key;
$fn$;

-- Write-time projection: every entity_writes insert fans out into its
-- per-field provenance rows inside the SAME transaction (so a committed
-- write is always queryable, and a rolled-back one never projects).
CREATE FUNCTION provenance_project_write() RETURNS trigger
LANGUAGE plpgsql
AS $fn$
BEGIN
  INSERT INTO provenance_events (write_id, field_seq, org_id, entity, entity_id,
                                 seq, op, actor_kind, actor_id, field, before, after,
                                 session_id, skill, ruleset_hash, is_undo,
                                 idempotency_key, occurred_at)
  SELECT NEW.id,
         fields.field_seq,
         NEW.org_id, NEW.entity, NEW.entity_id,
         NEW.seq, NEW.op, NEW.actor_kind, NEW.actor_id,
         fields.field, fields.prior, fields.applied,
         NEW.session_id, NEW.skill, NEW.ruleset_hash, NEW.is_undo,
         NEW.idempotency_key, NEW.created_at
  FROM (
    SELECT row_number() OVER (ORDER BY d.field) AS field_seq,
           d.field, d.prior, d.applied
    FROM provenance_diff(NEW.before, NEW.after) AS d
  ) fields;
  RETURN NEW;
END;
$fn$;

CREATE TRIGGER provenance_project
AFTER INSERT ON entity_writes
FOR EACH ROW EXECUTE FUNCTION provenance_project_write();

-- Backfill: project the writes that predate this migration. Same shared
-- diff, field_seq from field-name order — identical to what the trigger
-- would have written (the trigger only fires on FUTURE inserts, so the
-- two never overlap).
INSERT INTO provenance_events (write_id, field_seq, org_id, entity, entity_id,
                               seq, op, actor_kind, actor_id, field, before, after,
                               session_id, skill, ruleset_hash, is_undo,
                               idempotency_key, occurred_at)
SELECT w.id,
       fields.field_seq,
       w.org_id, w.entity, w.entity_id,
       w.seq, w.op, w.actor_kind, w.actor_id,
       fields.field, fields.prior, fields.applied,
       w.session_id, w.skill, w.ruleset_hash, w.is_undo,
       w.idempotency_key, w.created_at
FROM entity_writes w
CROSS JOIN LATERAL (
  SELECT row_number() OVER (ORDER BY d.field) AS field_seq,
         d.field, d.prior, d.applied
  FROM provenance_diff(w.before, w.after) AS d
) fields;

-- Doctrine #9 questions, one covering index each:
--   * "what is the history of this record / this field"
--   * "who did this" (actor lookups; the self-scoped RBAC path too)
--   * "what did this agent session / skill / rule-set write" (when present;
--     partial — the overwhelmingly common human rows are excluded)
CREATE INDEX idx_provenance_record
  ON provenance_events (org_id, entity, entity_id, field, occurred_at DESC, write_id DESC);
CREATE INDEX idx_provenance_actor
  ON provenance_events (org_id, actor_id, occurred_at DESC, write_id DESC);
CREATE INDEX idx_provenance_session
  ON provenance_events (org_id, session_id, occurred_at DESC) WHERE session_id IS NOT NULL;
CREATE INDEX idx_provenance_skill
  ON provenance_events (org_id, skill, occurred_at DESC) WHERE skill IS NOT NULL;
CREATE INDEX idx_provenance_ruleset
  ON provenance_events (org_id, ruleset_hash, occurred_at DESC) WHERE ruleset_hash IS NOT NULL;

ALTER TABLE provenance_events ENABLE ROW LEVEL SECURITY;
CREATE POLICY provenance_events_org ON provenance_events
  USING (org_id = current_setting('app.org_id', true)::uuid);
