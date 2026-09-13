-- LISTEN/NOTIFY delivery seam (Wave 12 lane 12, instant-first support
-- contract). The transactional outbox stays the single authoritative event
-- trail; this trigger adds a same-transaction NOTIFY so a process-local
-- fan-out (core-api's SSE hub) can push contract events to connected
-- clients the moment their transaction commits — no polling, no second
-- pipeline. pg_notify is transactional: notifications fire only when the
-- enclosing mutation commits, and roll back with it, so the trail and the
-- push channel can never disagree.
--
-- Payload is the event IDENTITY only (never the payload body): exactly the
-- SSE wire frame {id, org_id, type, entity, entity_id, seq, at} the web
-- instant-store reconciles on. `seq` is the per-entity monotonic ledger
-- value (entity_write_seqs) joined at trigger time — i.e. INSIDE the same
-- transaction, so the attribution is exact even when several writes to one
-- entity commit back-to-back. Events written outside the instant-path
-- (no entity_write_seqs row) carry seq: null; the ledger treats a null seq
-- as always-apply (no dedupe), and the client's snapshot hydration covers
-- ordering. The identity payload is far below the 8000-byte NOTIFY limit.
--
-- SECURITY DEFINER so seq enrichment works from any write context (the
-- application role with app.org_id GUC, migrations, internal jobs): the
-- subquery is strictly keyed on NEW.org_id, so definer status cannot widen
-- what is read beyond the inserting row's own org (narrow-function
-- precedent: cross-org email routing, 0064).

CREATE FUNCTION outbox_notify_meridian_events() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER AS $$
DECLARE
  v_entity_id uuid;
BEGIN
  -- outbox_events.entity_id is text; only uuid-keyed entities have a
  -- ledger counter to join against. A non-uuid id degrades to seq: null.
  IF NEW.entity_id IS NOT NULL THEN
    BEGIN
      v_entity_id := NEW.entity_id::uuid;
    EXCEPTION WHEN invalid_text_representation THEN
      v_entity_id := NULL;
    END;
  END IF;

  PERFORM pg_notify(
    'meridian_events',
    json_build_object(
      'id',        NEW.id,
      'org_id',    NEW.org_id,
      'type',      NEW.event_type,
      'entity',    NEW.entity,
      'entity_id', NEW.entity_id,
      'seq',       (SELECT seq FROM entity_write_seqs
                     WHERE org_id = NEW.org_id
                       AND entity = NEW.entity
                       AND entity_id = v_entity_id),
      'at',        NEW.occurred_at
    )::text
  );
  RETURN NEW;
END;
$$;

CREATE TRIGGER trg_outbox_notify_meridian_events
AFTER INSERT ON outbox_events
FOR EACH ROW EXECUTE FUNCTION outbox_notify_meridian_events();
