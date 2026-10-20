-- One queued delivery per (surface, event): a dedupe key on `jobs`
-- (Wave 52 lane P1c, `p52-webhook-consumer-name`).
--
-- Why this exists: the webhooks-out consumer is at-least-once at the stream
-- level. A pending entry can be replayed after a crash, and a replica that
-- finds an entry idle past its claim threshold takes it from a consumer that
-- never acked. Each of those re-reads the same event, and `queue::enqueue`
-- used to insert a fresh `jobs` row every time, so the receiver got a second
-- signed POST for an event it had already accepted. The ledger write after
-- the POST is the only part that was unique-guarded
-- (`webhook_deliveries` on `(webhook_id, event_id, attempt)`, `ON CONFLICT DO
-- NOTHING`), which records the duplicate but cannot prevent it.
--
-- Why a key column and not a UNIQUE over the payload: the natural key lives
-- inside `payload` jsonb, so a constraint would have to be an expression
-- index over `payload->>'webhook_id'` and `payload->>'event_id'` — jsonb text
-- extraction, checked on every insert to the table, for one job kind. A plain
-- text column the producer fills in states the intent at the call site and
-- costs nothing for the jobs that do not opt in.
--
-- Why partial: every other job kind keeps today's behaviour exactly — no key,
-- no uniqueness, unlimited rows. A job kind that wants the guard passes a key;
-- `queue::enqueue_deduped` is the only writer and it is the only place that
-- knows the format.
--
-- LOCK NOTE: `jobs` receives every background unit of work in the product. The
-- index builds with a share of ACCESS EXCLUSIVE, so writes wait while it
-- builds. The table is kept small by the queue's own settle/retention
-- sweeps, and `IF NOT EXISTS` makes the statement a no-op on an install that
-- pre-built it with `CREATE INDEX CONCURRENTLY`.

ALTER TABLE jobs ADD COLUMN IF NOT EXISTS dedupe_key text;

CREATE UNIQUE INDEX IF NOT EXISTS idx_jobs_dedupe_key
  ON jobs (dedupe_key)
  WHERE dedupe_key IS NOT NULL;
