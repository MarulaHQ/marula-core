-- Dead work, with a list, two verbs, and a pause (Wave 52 lane R3,
-- `p52-ledger-redrive`).
--
-- Why this exists: three different ledgers could each abandon work, and each
-- one did it silently. `jobs` flips a row to `dead` after its attempt budget
-- and only a counter (`core_db::queue::stats`) noticed. `email_outbound` does
-- the same on its own attempt counter. Webhook deliveries dead-letter as a
-- `webhook.deliver` job, so they were in `jobs` all along but indistinguishable
-- from a workflow wake without reading the payload. R2 added `dead_events` for
-- stream entries whose handler exhausted its budget, and that list has no
-- reader either. An operator with a receiver that started returning 500 could
-- see "12 dead" and had no way to ask what they were, no way to retry one, and
-- no way to stop the next twelve.
--
-- What this migration adds, in order:
--   1. A pause on a webhook destination: the counters that decide it, the
--      instant it took effect, and why. A paused destination is not matched for
--      new events, so the dead set stops growing while an operator fixes or
--      clears it.
--   2. A record of a cancellation decision, kept when the work item is moved
--      out of the dead set or (for email and dead events) when it cannot be.
--   3. `ops.dead_work`: one row shape over the four sources.
--   4. A dead-job index the view and the operational list both read.
--   5. `apply_email_send_result` learns a permanence flag, so a provider that
--      refuses a message on its merits stops retrying it eight times.
--
-- Non-purpose: no retention (R6 owns pruning these tables), no automatic
-- redrive (an operator decides; R4 owns reconciliation that finds gaps), and no
-- replay of a dead stream event — see the `replayable` column note below.

-- ---------------------------------------------------------------------------
-- 1. A webhook destination can be paused
-- ---------------------------------------------------------------------------
--
-- `consecutive_failures` counts DELIVERIES that failed in a retryable way since
-- the last delivery this destination accepted (or last refused on its merits,
-- which still proves the endpoint is answering). It is not the job's attempt
-- counter: a single event's six attempts are six failures, and six is also the
-- point at which one event has exhausted its whole budget without a single
-- success. Both readings mean the same thing to an operator — this endpoint is
-- not working — which is why the threshold is the policy's own attempt budget
-- rather than a second number that would drift from it.
--
-- Pause is set by the delivery path and cleared by an operator (or by changing
-- the URL, which is a different destination). It is deliberately not cleared on
-- the next success: a destination that has to be fixed will not deliver
-- anything while paused, so a "clear on success" rule would never fire.

ALTER TABLE webhooks ADD COLUMN IF NOT EXISTS consecutive_failures integer NOT NULL DEFAULT 0;
ALTER TABLE webhooks ADD COLUMN IF NOT EXISTS paused_at timestamptz;
ALTER TABLE webhooks ADD COLUMN IF NOT EXISTS paused_reason text;

-- Only paused rows are ever scanned by the admin list; the unpaused majority
-- pays nothing.
CREATE INDEX IF NOT EXISTS idx_webhooks_paused
  ON webhooks (org_id, paused_at DESC)
  WHERE paused_at IS NOT NULL;

-- ---------------------------------------------------------------------------
-- 2. A cancellation is a decision, and decisions are kept
-- ---------------------------------------------------------------------------
--
-- Why a table and not a column on each source: `jobs` has a spare terminal
-- status (`failed`, never written by the queue) so a cancelled job simply
-- leaves the dead set; `email_outbound` has no spare status and its CHECK
-- constraint is not worth a breaking migration to widen, and `dead_events` is
-- R2's table whose shape a later redrive lane will want unchanged. One table
-- covers all four sources with the same idempotent insert, and it keeps the
-- reason and the actor next to the id, which is what makes the list reviewable
-- a month later.
--
-- A cancellation never deletes the record of the failure: the email ledger row
-- stays `dead` (nobody will retry it, which is exactly what "dead" means) and
-- the dead-work list stops showing it because this table says an operator
-- looked at it. The audit row is the durable statement of who decided.

CREATE TABLE dead_work_cancellations (
  -- 'job' | 'email' | 'webhook' | 'event' — the same vocabulary as the view.
  source text NOT NULL CHECK (source IN ('job', 'email', 'webhook', 'event')),
  -- The underlying row id in that source's ledger. No foreign key on purpose:
  -- three of the four parents are pruned by retention (R6) and a cascade would
  -- take the decision with the row.
  id uuid NOT NULL,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- Required, because "why did we give up on this" is the only question the
  -- row will be asked later.
  reason text NOT NULL,
  cancelled_by uuid REFERENCES users(id) ON DELETE SET NULL,
  cancelled_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (source, id)
);

CREATE INDEX idx_dead_work_cancellations_org
  ON dead_work_cancellations (org_id, cancelled_at DESC);

ALTER TABLE dead_work_cancellations ENABLE ROW LEVEL SECURITY;
CREATE POLICY dead_work_cancellations_org ON dead_work_cancellations
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- Explicit rather than relying on the schema's ALTER DEFAULT PRIVILEGES: the
-- grant is one line, and a lane that later discovers the default was set by a
-- different role would be debugging a permission error on an insert that looks
-- correct in every other respect.
GRANT SELECT, INSERT, UPDATE, DELETE ON dead_work_cancellations TO marula_app;

-- ---------------------------------------------------------------------------
-- 3. ops.dead_work — one shape over four ledgers
-- ---------------------------------------------------------------------------
--
-- Columns, all four sources agreeing on them:
--   source      text         'job' | 'email' | 'webhook' | 'event'
--   id          uuid         the row id in that source (the verb's argument)
--   org_id      uuid         owning org; NULL for a dead event whose envelope
--                            never parsed, which is not attributable
--   subject     text         what a human would call this: the job kind and its
--                            ticket, the email subject and recipients, the
--                            webhook URL and event type, the consumer group and
--                            event type
--   attempts    integer      attempts made before it was abandoned
--   last_error  text         the abandonment reason as the ledger recorded it
--   dead_since  timestamptz  when it was abandoned (not when it was enqueued)
--   replayable  boolean      whether the redrive verb can requeue it
--   cancelled_at timestamptz the cancellation decision, when one exists
--
-- `replayable` is a column rather than a rule in the verb because the operator
-- list has to show it BEFORE the click. Two of the four sources requeue in
-- place: the queue rewinds the row (attempts to zero, due now) and the dedupe
-- key stays, so a replay can neither duplicate a delivery nor fork its history.
-- A row an operator cancelled reports false, because a cancel is terminal. A
-- dead event reports false: its payload is the raw stream body, and
-- re-publishing needs the relay's stream writer, which lives in the worker and
-- is not this surface's property. R4's gap detector owns that path. The verb
-- refuses both with a typed conflict instead of pretending.
--
-- security_invoker so the four base tables' policies answer for the caller;
-- a view owner's rights here would read every org's dead work into an operator
-- list scoped to one.

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
       true AS replayable,
       c.cancelled_at
  FROM email_outbound q
  JOIN email_messages m ON m.id = q.email_message_id
  LEFT JOIN dead_work_cancellations c
         ON c.source = 'email' AND c.org_id = q.org_id AND c.id = q.id
 WHERE q.status = 'dead'
UNION ALL
SELECT 'event'::text AS source,
       e.id,
       e.org_id,
       e.consumer_group || ' / ' || COALESCE(e.event_type, '(unparsed entry)') AS subject,
       e.attempts,
       e.last_error,
       e.dead_at AS dead_since,
       false AS replayable,
       c.cancelled_at
  FROM dead_events e
  LEFT JOIN dead_work_cancellations c
         ON c.source = 'event'
        AND c.org_id IS NOT DISTINCT FROM e.org_id
        AND c.id = e.id;

GRANT SELECT ON ops.dead_work TO marula_app;

-- The job leg of the view filters `status = 'dead'`, and the admin list sorts
-- by abandonment time. `idx_jobs_claim` is partial on `pending` and cannot
-- serve either, so the dead set — the only part of `jobs` that accumulates
-- until someone acts — gets its own.
CREATE INDEX IF NOT EXISTS idx_jobs_dead
  ON jobs (org_id, updated_at DESC)
  WHERE status = 'dead';

-- ---------------------------------------------------------------------------
-- 4. A permanent refusal stops retrying immediately
-- ---------------------------------------------------------------------------
--
-- The ledger's dead decision was `attempts >= max_attempts`, so a message the
-- provider refused on its merits — a bad address, an inactive recipient, a
-- malformed sender — burned the whole budget proving the same thing, and landed
-- in the dead set eight backoffs later where an operator found it late. The
-- permanence flag is the transport's classification (core-mail
-- `TransportError::permanent`), not a status list here: a 4xx-shaped refusal
-- from one provider, a 5xx-shaped one from another, and an SMTP reply code
-- from a third all arrive as the same two classes at this boundary.
--
-- The six-argument form is new; the original five-argument form stays and
-- delegates with `false`, so a core-api binary from before this migration
-- keeps calling a function that exists during a rolling deploy.

CREATE OR REPLACE FUNCTION apply_email_send_result(
  p_id uuid, p_ok boolean, p_provider_id text, p_error text, p_delay interval,
  p_permanent boolean
)
RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public AS $fn$
BEGIN
  IF p_ok THEN
    UPDATE email_outbound
       SET status = 'sent', provider_message_id = p_provider_id,
           last_error = NULL, claimed_at = NULL, updated_at = now()
     WHERE id = p_id AND status = 'sending';
  ELSE
    UPDATE email_outbound
       SET status = CASE WHEN p_permanent OR attempts >= max_attempts
                         THEN 'dead' ELSE 'queued' END,
           next_attempt_at = CASE WHEN p_permanent OR attempts >= max_attempts
                                  THEN next_attempt_at
                                  ELSE now() + p_delay END,
           last_error = p_error, provider_message_id = NULL,
           claimed_at = NULL, updated_at = now()
     WHERE id = p_id AND status = 'sending';
  END IF;
  RETURN FOUND;
END
$fn$;

CREATE OR REPLACE FUNCTION apply_email_send_result(
  p_id uuid, p_ok boolean, p_provider_id text, p_error text, p_delay interval
)
RETURNS boolean
LANGUAGE sql SECURITY DEFINER SET search_path = public AS $fn$
  SELECT apply_email_send_result(p_id, p_ok, p_provider_id, p_error, p_delay, false);
$fn$;

GRANT EXECUTE ON FUNCTION apply_email_send_result(uuid, boolean, text, text, interval, boolean)
  TO marula_app;
GRANT EXECUTE ON FUNCTION apply_email_send_result(uuid, boolean, text, text, interval)
  TO marula_app;
