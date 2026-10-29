-- 0175 — `ops.dead_work` tells the truth about what can be replayed.
--
-- Why. The view was written before R4's reconciler shipped and hard-coded
-- `false AS replayable` for the event leg, on the reasoning that re-publishing a
-- stream entry needed the worker's writer and was not this surface's property.
-- The reconciler now does exactly that (`core_db::reconcile::replayable_dead_events`
-- → `mark_replayed`), so the list told an operator a dead event could not be
-- recovered at the moment the worker was busy recovering it (Wave 62 lane
-- `p62-data-truth`, ruling 2026-10-27).
--
-- The expression mirrors the reconciler's row-level rule: unreplayed, with an
-- event identity and a payload to publish, and not cancelled. The reconciler's
-- remaining bound — deaths per (group, event_id) — is a pass parameter, not a
-- property of the row, so it stays out of the view.
--
-- The email leg also joins the cancelled set: 0174 gave a cancellation its own
-- status, and a row an operator cancelled leaves the dead set without leaving
-- the ledger. Without this the "include cancelled" filter would show cancelled
-- jobs but silently drop cancelled mail.

CREATE OR REPLACE VIEW ops.dead_work WITH (security_invoker = true) AS
WITH dead_jobs AS (
  SELECT j.id,
         j.org_id,
         j.kind,
         j.status,
         j.payload,
         j.attempts,
         j.last_error,
         j.updated_at,
         -- Text extraction first, cast second, and only after the shape is
         -- proven: `(payload->>'webhook_id')::uuid` on a non-webhook job
         -- payload would raise, and a view that raises on one row hides all
         -- of them.
         CASE
           WHEN j.kind = 'webhook.deliver'
            AND j.payload->>'webhook_id' ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
           THEN (j.payload->>'webhook_id')::uuid
         END AS webhook_id
    FROM jobs j
   -- `failed` rides along because cancellation puts it there: the row has left
   -- the dead set (which is the point) but a caller asking for cancelled rows
   -- should still be shown what was abandoned and why.
   WHERE j.status IN ('dead', 'failed')
)
SELECT 'job'::text AS source,
       d.id,
       d.org_id,
       d.kind || COALESCE(' / ' || (d.payload->>'ticket_id'), '') AS subject,
       d.attempts,
       d.last_error,
       d.updated_at AS dead_since,
       d.status = 'dead' AS replayable,
       c.cancelled_at
  FROM dead_jobs d
  LEFT JOIN dead_work_cancellations c
         ON c.source = 'job' AND c.org_id = d.org_id AND c.id = d.id
 WHERE d.kind <> 'webhook.deliver'
   AND (d.status = 'dead' OR c.cancelled_at IS NOT NULL)
UNION ALL
SELECT 'webhook'::text AS source,
       d.id,
       d.org_id,
       COALESCE(w.url, '(deleted subscription)')
         || ' <- ' || COALESCE(d.payload->'envelope'->>'type', 'event') AS subject,
       d.attempts,
       d.last_error,
       d.updated_at AS dead_since,
       d.status = 'dead' AS replayable,
       c.cancelled_at
  FROM dead_jobs d
  LEFT JOIN webhooks w ON w.id = d.webhook_id AND w.org_id = d.org_id
  LEFT JOIN dead_work_cancellations c
         ON c.source = 'webhook' AND c.org_id = d.org_id AND c.id = d.id
 WHERE d.kind = 'webhook.deliver'
   AND (d.status = 'dead' OR c.cancelled_at IS NOT NULL)
UNION ALL
SELECT 'email'::text AS source,
       q.id,
       q.org_id,
       COALESCE(m.subject, '(no subject)')
         || ' -> ' || array_to_string(m.to_addresses, ', ') AS subject,
       q.attempts,
       q.last_error,
       q.updated_at AS dead_since,
       q.status = 'dead' AS replayable,
       c.cancelled_at
  FROM email_outbound q
  JOIN email_messages m ON m.id = q.email_message_id
  LEFT JOIN dead_work_cancellations c
         ON c.source = 'email' AND c.org_id = q.org_id AND c.id = q.id
 WHERE q.status IN ('dead', 'cancelled')
UNION ALL
SELECT 'event'::text AS source,
       e.id,
       e.org_id,
       e.consumer_group || ' / ' || COALESCE(e.event_type, '(unparsed entry)') AS subject,
       e.attempts,
       e.last_error,
       e.dead_at AS dead_since,
       -- The reconciler's row-level rule; see the header.
       (e.replayed_at IS NULL
        AND e.event_id IS NOT NULL
        AND e.payload IS NOT NULL
        AND c.cancelled_at IS NULL) AS replayable,
       c.cancelled_at
  FROM dead_events e
  LEFT JOIN dead_work_cancellations c
         ON c.source = 'event'
        AND c.org_id IS NOT DISTINCT FROM e.org_id
        AND c.id = e.id;

GRANT SELECT ON ops.dead_work TO marula_app;
