-- Correct asset custody classes that an insert-then-patch order left behind.
--
-- The 0232 insert trigger derives `custody_class` from `scope` and `ownership`.
-- The asset create route wrote the row first and applied `lifecycle.ownership`
-- afterwards, so a rented or leased unit created through that route took the
-- class of the owned default and kept it after the ownership patch. The same
-- order bug reached client-scoped assets, where the insert default is
-- `client_custody` and a leased unit belongs in `rental_out`.
--
-- The write order is fixed at the callers: the final ownership travels in the
-- insert, and an ownership change on an existing asset writes a class movement.
-- This migration repairs the rows the old order already wrote, and it writes
-- them as movements so no class changes silently.
--
-- Only the exact bug shape is matched: a class that is still the insert default
-- for the row's scope, with an ownership that no longer agrees with it. A unit
-- deliberately moved to sale stock, tools or a terminal class is untouched,
-- because its class is not one of those two defaults.
--
-- A unit on an open loan, checkout or ticket cannot move (the 0232 guard). That
-- refusal is correct: the block leaves the row for its next class action and
-- says so, instead of failing an upgrade on a legitimate open agreement.

DO $$
DECLARE
  r record;
BEGIN
  FOR r IN
    SELECT a.id,
           a.org_id,
           a.ownership,
           a.custody_class AS from_class,
           CASE WHEN a.scope = 'internal_org' THEN 'rental_in' ELSE 'rental_out' END
             AS to_class
      FROM device_assets a
     WHERE a.ownership IN ('leased', 'rented')
       AND (
         (a.scope = 'internal_org' AND a.custody_class = 'internal_fleet')
         OR (a.scope = 'client' AND a.custody_class = 'client_custody')
       )
  LOOP
    BEGIN
      INSERT INTO inventory_class_movements
        (id, org_id, unit_kind, device_asset_id, from_class, to_class, reason,
         source_kind, source_id)
      VALUES
        (gen_random_uuid(), r.org_id, 'asset', r.id, r.from_class, r.to_class,
         format('ownership was recorded as %s after the asset was created', r.ownership),
         'device_asset', r.id);
    EXCEPTION WHEN raise_exception THEN
      RAISE NOTICE
        'asset % stays in % until its next class movement: %',
        r.id, r.from_class, SQLERRM;
    END;
  END LOOP;
END $$;
