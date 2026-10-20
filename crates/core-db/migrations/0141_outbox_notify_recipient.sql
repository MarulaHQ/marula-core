-- The per-user recipient on the event NOTIFY (Wave 52 lane P2b).
--
-- Why this exists: core-api's SSE hub is org-scoped, so before this
-- migration a notification or timer event reached every member of the org
-- and the frame carried nothing that said who it was for. The deleted
-- socket leg routed those families on `user:<id>` channels instead. The
-- recipient is already in the envelope payload (`NotificationCreated.user_id`,
-- `TimerStarted.user_id`, ...), so the ONLY thing missing is getting it to
-- the listener.
--
-- Why `payload->'user_id'` and not the whole payload: the NOTIFY payload
-- stays the event IDENTITY. Shipping the body would send every event's
-- contents to every replica over the channel and can exceed the 8000-byte
-- NOTIFY limit; the id is a scalar and costs nothing. Shipping the body
-- would also push the classification decision into SQL, where it would
-- have to be kept in step with `realtime::recipient_from` by hand.
--
-- Why the type allow-list is NOT in SQL: this trigger copies `user_id`
-- whenever the payload has one, and `recipient_from` in core-api decides
-- which event families are per-user. One place to teach about a new
-- per-user family, and no chance of the two disagreeing.
--
-- Exact body of migration 0114's trigger with one key added; the channel
-- name, SECURITY DEFINER and seq join are unchanged.

DROP TRIGGER IF EXISTS trg_outbox_notify_marula_events ON outbox_events;
DROP FUNCTION IF EXISTS outbox_notify_marula_events();

CREATE FUNCTION outbox_notify_marula_events() RETURNS trigger
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
    'marula_events',
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
      'at',        NEW.occurred_at,
      -- The recipient, when the payload names one. `->` keeps it a JSON
      -- string (or null), which is what the listener deserializes; a
      -- payload with no `user_id` yields null and the frame stays
      -- org-scoped.
      'user_id',   NEW.payload->'user_id'
    )::text
  );
  RETURN NEW;
END;
$$;

CREATE TRIGGER trg_outbox_notify_marula_events
AFTER INSERT ON outbox_events
FOR EACH ROW EXECUTE FUNCTION outbox_notify_marula_events();
