-- Migration 0066: `fx_rates` — append-only rate data (ADR 0002, Wave 12
-- lane 10, FX3).
--
-- Purpose: the org's rate book. One row per recorded rate: (base, quote,
-- as_of_date, rate, source, captured_at).
--
--   * Pair semantics are DIRECTIONAL: `base_currency` → `quote_currency`
--     with `rate` = how many QUOTE major units one BASE major unit buys
--     ("1 base = rate quote"). 1 USD = 17.85 ZAR is a row
--     (USD, ZAR, 17.85). Resolving the inverse direction (ZAR → USD) is
--     an explicit core-domain operation (`FxRate::inverse`, half-even at
--     12 decimal places), never a UI-side 1/x.
--
--   * Append-only: corrections ADD rows, history is never rewritten.
--     BEFORE UPDATE/DELETE triggers refuse any rewrite (the ledger is
--     the audit trail). The effective rate for a lookup is deterministic:
--     the newest `captured_at` among rows for the pair whose
--     `as_of_date` is the latest date ≤ the requested date.
--
--   * `source` names where the rate came from; the built-in manual-entry
--     provider stamps 'manual'. Managed daily feeds (Relay bundles on the
--     `FxRateProvider` trait) stamp their own source names.
--
--   * Manual entry is the standalone "none" mode: no rate service is
--     required. A cross-currency fact whose rate is missing fails the
--     write with a typed error naming the missing pair — the core never
--     implicitly converts and never silently defaults to 1:1.
--
-- RLS: org-scoped like every user-data table. Rate rows are org-private
-- (two orgs may disagree about the same pair — corrections are local).

CREATE TABLE fx_rates (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  base_currency text NOT NULL,
  quote_currency text NOT NULL,
  -- The rate date the rate applies to (the books' date, not capture time).
  as_of_date date NOT NULL,
  -- QUOTE major units per 1 BASE major unit; up to 12 decimal places
  -- (carried losslessly to core-domain's 1e12-scaled rate).
  rate numeric(24,12) NOT NULL CHECK (rate > 0),
  -- 'manual' for the built-in provider; managed feeds stamp their names.
  source text NOT NULL,
  -- When this row entered the book (append-only ordering key).
  captured_at timestamptz NOT NULL DEFAULT now(),
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  CONSTRAINT fx_rates_pair_distinct CHECK (base_currency <> quote_currency),
  CONSTRAINT fx_rates_iso_base CHECK (base_currency ~ '^[A-Z]{3}$'),
  CONSTRAINT fx_rates_iso_quote CHECK (quote_currency ~ '^[A-Z]{3}$')
);

CREATE INDEX idx_fx_rates_lookup
  ON fx_rates (org_id, base_currency, quote_currency, as_of_date DESC, captured_at DESC);

-- Append-only enforcement: the table is INSERT-only. UPDATE and DELETE
-- raise; the schema is the policy, not a convention.
CREATE OR REPLACE FUNCTION fx_rates_immutable() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'fx_rates is append-only: % is not permitted (row %)',
    TG_OP, OLD.id
    USING ERRCODE = 'check_violation';
END;
$$;
CREATE TRIGGER fx_rates_no_update
  BEFORE UPDATE ON fx_rates
  FOR EACH ROW EXECUTE FUNCTION fx_rates_immutable();
CREATE TRIGGER fx_rates_no_delete
  BEFORE DELETE ON fx_rates
  FOR EACH ROW EXECUTE FUNCTION fx_rates_immutable();

ALTER TABLE fx_rates ENABLE ROW LEVEL SECURITY;
CREATE POLICY fx_rates_org ON fx_rates
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);
