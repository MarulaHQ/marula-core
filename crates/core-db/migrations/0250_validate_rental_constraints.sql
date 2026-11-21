-- Validate the constraints migration 0237 added NOT VALID.
--
-- 0237 widens the invoice-line kind vocabulary and adds the wipe shape on the
-- class ledger. It adds both NOT VALID, so they enforce every row written from
-- that migration on, while the scan of rows that already existed is this file's
-- job. The `rental_in` shape on `device_assets` stays NOT VALID on purpose: its
-- historical rows are the backfill's, and refusing an upgrade over one of them
-- would be worse than leaving the constraint to police new writes.
-- Splitting the two matters for the lock: the scan in 0237 would run inside the
-- transaction that still holds the ACCESS EXCLUSIVE lock its `ALTER TABLE` took,
-- and on a populated `invoice_lines` that is a write outage for the length of
-- the scan. `VALIDATE CONSTRAINT` takes SHARE UPDATE EXCLUSIVE instead, which
-- lets reads and writes through.
--
-- Every row these constraints cover was written under a narrower rule (the
-- invoice kind vocabulary is only widened here, and a wipe timestamp could not
-- exist before the column did), so validation is expected to pass. If a row on
-- an installation does violate one, this migration fails and names the
-- constraint, which is the honest outcome: the data needs a look, not a
-- silently skipped scan.

ALTER TABLE invoice_lines
  VALIDATE CONSTRAINT invoice_lines_kind_check;

ALTER TABLE inventory_class_movements
  VALIDATE CONSTRAINT inventory_class_movements_wipe_shape;
