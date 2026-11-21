-- One open client agreement per unit, not only per serial.
--
-- 0104 guarded the serial side: `uq_client_loan_open_unit` stops a second open
-- line for the same serial. An asset-only line (a fleet device lent out, or a
-- unit leased straight from an asset row) carries no serial, so nothing stopped
-- the same asset appearing on two open agreements. The loan path's check between
-- insert and the unique index is not a guard under concurrency; this index is.
--
-- The partial predicate matches the serial index: only open lines are unique, a
-- unit may be issued, returned and issued again. A stock-sourced line carries
-- both ids, so it is covered by both indexes; that is intended, not redundant —
-- either one alone would let the other shape through.
--
-- Failure mode: if an installation already has one asset on two open lines, the
-- index build fails. The preflight below names each offending asset and the
-- lines, and prints the statement to resolve it, so the operator is not left to
-- infer a recovery from a bare unique violation.

DO $$
DECLARE
  dup record;
BEGIN
  FOR dup IN
    SELECT org_id, device_asset_id, count(*) AS open_lines,
           string_agg(id::text, ', ' ORDER BY id) AS line_ids
      FROM client_loan_lines
     WHERE returned_at IS NULL AND device_asset_id IS NOT NULL
     GROUP BY org_id, device_asset_id
    HAVING count(*) > 1
  LOOP
    RAISE EXCEPTION
      'asset % in org % has % open client-loan lines (%); close all but the one that is still out: UPDATE client_loan_lines SET returned_at = now() WHERE org_id = % AND device_asset_id = % AND id <> <survivor id>',
      dup.device_asset_id, dup.org_id, dup.open_lines, dup.line_ids,
      dup.org_id, dup.device_asset_id;
  END LOOP;
END $$;

CREATE UNIQUE INDEX uq_client_loan_open_asset
  ON client_loan_lines (org_id, device_asset_id)
  WHERE returned_at IS NULL AND device_asset_id IS NOT NULL;
