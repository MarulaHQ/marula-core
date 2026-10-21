-- Stream-consumer delivery bookkeeping, and the stream dead-letter queue.
--
-- Why this exists: every stream consumer used to recover one page of its
-- pending list at startup and retry nothing afterwards. A handler that failed
-- left its entry unacknowledged in the group's pending list until the process
-- restarted, and if the same entry failed the same way every time, nothing
-- said so: the group's PEL depth grew, the entry was retried once per restart,
-- and an operator had no list of what was stuck or why.
--
-- `consumer_deliveries` gives each (group, entry) one row: how many times it
-- has failed, when it is next due, the last error, and whether it settled or
-- was dead-lettered. It is the per-entry attempt counter, and it is what lets a
-- consumer retry a failed entry live — after the policy delay, in the same
-- process — instead of waiting for a restart.
--
-- `dead_events` is where an entry lands once its attempts are exhausted. The
-- entry leaves the PEL at that point (a poison message must not hold the tail
-- back), so this table is the only record that it existed: event identity,
-- group, consumer, attempt count, last error, and the raw payload so the work
-- can be requeued later without the stream entry.
--
-- Non-purpose: no retention policy (pruning is a separate operational
-- decision), no requeue verb, no org-facing surface. Rows are written by the
-- worker's consumer harness and read by operations tooling.

-- One row per (consumer group, stream entry). The entry id is a Valkey stream
-- id (`<ms>-<seq>`) and is text for that reason. A stream entry belongs to at
-- most one consumer at a time, but the same entry can be delivered to several
-- groups, so the group is part of the key.
CREATE TABLE consumer_deliveries (
  consumer_group text NOT NULL,
  entry_id text NOT NULL,
  -- Which stream the entry came from. One value today; a column because a
  -- second stream must not silently share the entry-id namespace.
  stream text NOT NULL DEFAULT 'marula:events',
  -- The consumer name that made the LAST attempt. Diagnostic: it says which
  -- replica last touched a stuck entry, not who created the row.
  consumer text NOT NULL,
  -- Envelope identity, when the entry parsed. Null for an entry whose body
  -- never parsed — those are still worth counting and dead-lettering.
  org_id uuid,
  event_id uuid,
  event_type text,
  attempts integer NOT NULL DEFAULT 0,
  first_attempt_at timestamptz NOT NULL DEFAULT now(),
  last_attempt_at timestamptz NOT NULL DEFAULT now(),
  -- When the next live retry is due. Null means "handle now" (a fresh entry,
  -- or one whose ack was lost after it settled).
  next_attempt_at timestamptz,
  last_error text,
  settled_at timestamptz,
  dead_at timestamptz,
  PRIMARY KEY (consumer_group, entry_id)
);

-- The live retry scan reads only unsettled, undead rows.
CREATE INDEX idx_consumer_deliveries_due
  ON consumer_deliveries (next_attempt_at)
  WHERE settled_at IS NULL AND dead_at IS NULL;

-- "What is dead in this group" is the operator question; the partial index
-- keeps that answer cheap without indexing the settled majority.
CREATE INDEX idx_consumer_deliveries_dead
  ON consumer_deliveries (consumer_group, dead_at)
  WHERE dead_at IS NOT NULL;

ALTER TABLE consumer_deliveries ENABLE ROW LEVEL SECURITY;
CREATE POLICY consumer_deliveries_org ON consumer_deliveries
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- The stream dead-letter queue. `payload` is text, not jsonb, on purpose: an
-- entry that failed to parse cannot be stored in a jsonb column at all, and
-- that is exactly the entry most worth keeping. A redrive reads it back and
-- re-parses with whatever the current contract is.
CREATE TABLE dead_events (
  id uuid PRIMARY KEY,
  stream text NOT NULL DEFAULT 'marula:events',
  consumer_group text NOT NULL,
  entry_id text NOT NULL,
  -- The consumer name that exhausted the budget.
  consumer text NOT NULL,
  org_id uuid,
  event_id uuid,
  event_type text,
  attempts integer NOT NULL,
  last_error text NOT NULL,
  payload text,
  dead_at timestamptz NOT NULL DEFAULT now()
);

-- One dead row per entry: a claimed entry is owned by a single consumer, and
-- the insert is `ON CONFLICT DO NOTHING`, so a redelivery after a lost ack
-- cannot produce a second record of the same abandonment.
CREATE UNIQUE INDEX idx_dead_events_entry ON dead_events (consumer_group, entry_id);
CREATE INDEX idx_dead_events_dead_at ON dead_events (dead_at DESC);

ALTER TABLE dead_events ENABLE ROW LEVEL SECURITY;
CREATE POLICY dead_events_org ON dead_events
  USING (org_id = current_setting('app.org_id', true)::uuid);
