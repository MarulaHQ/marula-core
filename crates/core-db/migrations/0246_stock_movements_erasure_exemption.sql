-- Tenant erasure must pass the stock movement ledger.
--
-- `stock_movements` is append-only by design, and 0119 gave it one sanctioned
-- escape for the demo reset. A second removal is not an edit: the erasure
-- contract deletes the `orgs` row and lets the cascade remove every org-scoped
-- row, and the guard as written refused that cascade, so a workspace with any
-- stock history could never be erased. The same exemption every guard from
-- 0232 on carries fixes it: a DELETE passes only when the parent org row is
-- already gone. Direct deletes and updates still raise.
--
-- The dev-reset bypass is preserved unchanged.

CREATE OR REPLACE FUNCTION stock_movements_append_only() RETURNS trigger AS $$
BEGIN
  IF current_setting('app.dev_reset_bypass', true) = 'on' THEN
    RETURN COALESCE(NEW, OLD);
  END IF;
  IF TG_OP = 'DELETE'
     AND NOT EXISTS (SELECT 1 FROM orgs o WHERE o.id = OLD.org_id) THEN
    RETURN OLD;
  END IF;
  RAISE EXCEPTION 'stock_movements is append-only: % rejected', TG_OP;
END;
$$ LANGUAGE plpgsql;
