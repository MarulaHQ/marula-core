-- Dev-reset bypass for the append-only guards.
--
-- `stock_movements` (and friends) are append-only BY DESIGN: the ledger is
-- evidence. But the demo toolchain must be able to wipe the demo org
-- (`seed-demo --force`, which `scripts/e2e-stack.sh` runs on EVERY boot, and
-- `POST /api/dev/reset-org`) or the guard turns the tool into a one-way door:
-- once a demo org has a movement, the org can never be reseeded again.
--
-- So the guard grows an explicit, session-local escape that ONLY the dev
-- reset path sets. The app never sets it, so tenant traffic keeps the full
-- append-only guarantee; a forgotten flag in application code fails closed
-- (the guard still fires).

CREATE OR REPLACE FUNCTION stock_movements_append_only() RETURNS trigger AS $$
BEGIN
  IF current_setting('app.dev_reset_bypass', true) = 'on' THEN
    RETURN COALESCE(NEW, OLD);
  END IF;
  RAISE EXCEPTION 'stock_movements is append-only: % rejected', TG_OP;
END;
$$ LANGUAGE plpgsql;
