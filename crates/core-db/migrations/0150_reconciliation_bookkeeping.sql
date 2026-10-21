-- Reconciliation bookkeeping (Wave 52 lane R4, `p52-reconciliation-jobs`).
--
-- Why this exists: R2 gave every stream consumer a per-entry attempt counter
-- and a dead-letter table, R3 made the four ledgers visible and let an operator
-- requeue three of them, and R6 gave the growing tables a retention window. What
-- remained is the work nobody re-attempts on its own: an entry that exhausted
-- its budget, a report slot that failed once and was never claimed again, an
-- index row whose event was published before its consumer group existed.
--
-- This migration adds only the two facts those repairs cannot derive from the
-- ledgers as they stand:
--
--   1. Which dead stream entries have already been replayed, and how many times.
--      A replayed event that SUCCEEDS leaves no trace in `dead_events` — the row
--      stays exactly as abandoned. Without a marker the reconciler would
--      re-publish the same entry on every pass, forever, which is worse than the
--      stall it repairs.
--   2. An index for the report-delivery retry scan. `report_deliveries` is an
--      append-only ledger of every claimed slot, so the failed minority is a
--      rounding error in a table that grows with every run of every schedule;
--      the retry scan must not be a sequential scan of all of it.
--
-- Non-purpose: no automatic redrive for `jobs` or `email_outbound` (R3's
-- doctrine stands — an operator decides, and both already requeue in place), no
-- backfill of `dead_events` history (a row with no `replayed_at` is simply
-- eligible once), and no watermark table for the search index: the backfill
-- pass compares source rows against `search_index` directly, which is stateless
-- and therefore cannot drift.

-- ---------------------------------------------------------------------------
-- 1. A dead entry remembers that it was replayed
-- ---------------------------------------------------------------------------

-- Null means "not replayed yet". The reconciler selects on `replayed_at IS
-- NULL` and stamps the instant it publishes the stored payload back onto the
-- stream.
ALTER TABLE dead_events ADD COLUMN IF NOT EXISTS replayed_at timestamptz;

-- How many times this exact row has been put back. Counted on the row rather
-- than inferred from a sibling row so the operator list can show it beside the
-- attempts that got it there.
ALTER TABLE dead_events ADD COLUMN IF NOT EXISTS replay_count integer NOT NULL DEFAULT 0
  CHECK (replay_count >= 0);

-- The reconciler's scan: unreplayed rows only. Partial because a replayed row is
-- never selected again and the settled majority must not be walked.
CREATE INDEX IF NOT EXISTS idx_dead_events_replayable
  ON dead_events (dead_at)
  WHERE replayed_at IS NULL;

-- ---------------------------------------------------------------------------
-- 2. The report-delivery retry scan
-- ---------------------------------------------------------------------------

-- `idx_report_deliveries_schedule` (0038) serves one schedule's history, which
-- is the UI's read. The retry pass asks a different question — "every failed or
-- partial slot, oldest first" — and would scan the delivered majority to answer
-- it. `created_at` is the column the pass orders by: it is the attempt's own
-- instant, and a retry's delay is measured from it.
CREATE INDEX IF NOT EXISTS idx_report_deliveries_retry
  ON report_deliveries (created_at)
  WHERE status IN ('failed', 'partial');
