-- Outbox bounds (Wave 52 lane P1, `p52-event-stream-bounds`).
--
-- `outbox_events` is the delivery trail: every mutation inserts its contract
-- event here inside the mutating transaction, the worker relays unpublished
-- rows to the event stream and stamps `published_at`. Nothing deleted from it,
-- and the only index was partial on the unpublished rows, so the two queries
-- that read it after a row is published had no usable index at all.
--
-- This migration adds exactly those two indexes. The retention window itself
-- is enforced from the worker (`crates/worker/src/outbox_retention_sweep.rs`),
-- not from SQL — the window is a deployment setting, and a `DELETE` on a
-- schedule belongs with the other housekeeping passes where its cadence and
-- its failures are visible.
--
-- NUMBERING: this is 0138, not 0137. Sibling lane `security-audit` claims
-- 0137 (`0137_plugin_catalog_origin_org.sql`) in its own worktree. Re-check
-- main's head immediately before merge — a board claim is a reservation at
-- claim time, not a guarantee at merge time (the lesson migration 0131's
-- header records).
--
-- LOCK NOTE (the precedent migration 0131 set): plain `CREATE INDEX` takes a
-- share of ACCESS EXCLUSIVE for the length of the build, so writes to
-- `outbox_events` — that is, every mutation in the product, since each one
-- writes an outbox row — wait for it. On a fresh or demo install the table is
-- small and the window is milliseconds. An install whose outbox has already
-- grown to millions of rows should build these out of band with
-- `CREATE INDEX CONCURRENTLY` first; the statements below then become no-ops
-- under `IF NOT EXISTS`. Make that release note, not a surprise.

-- The SSE resume query (`core-api`'s `realtime::replay_after`) reads
-- `WHERE org_id = $1 AND occurred_at > $2 ORDER BY occurred_at, id LIMIT $3`,
-- and its scoped variant adds `entity`/`entity_id`. Before this index every
-- reconnect scanned a table that only grows, and the RLS policy's
-- `org_id = current_setting('app.org_id', true)::uuid` predicate had nothing
-- to narrow it either. Column order is the query's own: equality first, then
-- the range, then the tiebreaker `id` so the `ORDER BY` is satisfied by the
-- index rather than a sort.
CREATE INDEX IF NOT EXISTS idx_outbox_org_occurred_id
  ON outbox_events (org_id, occurred_at, id);

-- The retention sweep deletes published rows past the window:
--   WHERE published_at IS NOT NULL AND published_at < $1
-- `published_at` is nullable, so the partial predicate keeps unpublished rows
-- (which the sweep must never touch) out of the index entirely, and keeps it
-- the size of the trimmed window rather than the whole trail. Runs alongside
-- the existing `idx_outbox_unpublished`, which serves `fetch_unpublished`.
CREATE INDEX IF NOT EXISTS idx_outbox_published_at
  ON outbox_events (published_at) WHERE published_at IS NOT NULL;
