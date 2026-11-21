-- Tenant erasure must pass the rate book.
--
-- `fx_rates` is append-only by design: a rate is a fact, and a later rate is a
-- new row. That guard also refused the erasure cascade, because deleting the
-- `orgs` row deletes its rates, so a workspace that had ever recorded a rate
-- could never be erased. The exemption every append-only table from 0232 on
-- carries fixes it: a DELETE passes only when the parent org row is already
-- gone. Direct deletes still raise, and an UPDATE always raises.

CREATE OR REPLACE FUNCTION fx_rates_immutable() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP = 'DELETE'
     AND NOT EXISTS (SELECT 1 FROM orgs o WHERE o.id = OLD.org_id) THEN
    RETURN OLD;
  END IF;
  RAISE EXCEPTION 'fx_rates is append-only: % is not permitted (row %)',
    TG_OP, OLD.id
    USING ERRCODE = 'check_violation';
END;
$$;
