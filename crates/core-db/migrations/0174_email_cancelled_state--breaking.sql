-- 0174 — a cancelled email is its own terminal state, not a dead one.
--
-- Why. `email_outbound.status` had four values and no spare terminal state, so
-- an operator cancellation left the row `dead`. The dead-work list hid it (the
-- cancellation table says so), but every count over the ledger kept calling it
-- dead: the summary strip, `outbound_status_counts`, and any alert built on the
-- dead bucket. An alert that can never clear is worse than no alert (Wave 62
-- lane `p62-data-truth`, ruling 2026-10-27).
--
-- Data impact: none is destroyed. The CHECK is widened (never narrowed) and the
-- backfill rewrites `status` on rows an operator had already cancelled, from
-- `dead` to `cancelled`. The constraint is dropped and re-added only because
-- PostgreSQL cannot alter a CHECK in place, which is why the filename carries
-- `--breaking`: the checker is mechanical and a human should read this before it
-- runs on a production ledger.
--
-- The backfill matters as much as the constraint: rows cancelled before this
-- migration carry their decision only in `dead_work_cancellations`, and leaving
-- them `dead` would keep the old lie in the historical counts.

ALTER TABLE email_outbound
  DROP CONSTRAINT IF EXISTS email_outbound_status_check;

ALTER TABLE email_outbound
  ADD CONSTRAINT email_outbound_status_check
  CHECK (status IN ('queued', 'sending', 'sent', 'dead', 'cancelled'));

UPDATE email_outbound q
   SET status = 'cancelled',
       updated_at = now()
 WHERE q.status = 'dead'
   AND EXISTS (
        SELECT 1 FROM dead_work_cancellations c
         WHERE c.source = 'email'
           AND c.id = q.id
           AND c.org_id = q.org_id
   );

-- The retention matrix prunes `sent` and `dead` rows. A status the sweep does
-- not name is a row nothing ever deletes, so the predicate moves with the
-- CHECK — `crates/core-db/src/retention.rs` carries the other half.
COMMENT ON COLUMN email_outbound.status IS
  'queued → sending → sent | dead | cancelled. `cancelled` is an operator decision (dead_work_cancellations) and is retained like `dead`.';
