-- Migration 0067: FX reconciliation read model (ADR 0002, Wave 12 lane 10,
-- FX7).
--
-- Purpose: the deliverable to the books — ONE named view per ADR 0002:
-- "original amount, rate, source/date, base equivalent, sync target".
-- Reports sum base amounts from conversion snapshots; they never
-- re-convert, and realized/unrealized gain-loss stays in the accounting
-- system (the PSA states this boundary: no revaluation here).
--
-- `ops.v1_fx_reconciliation` (ops.v1_* convention, migration 0026/0059)
-- unions the cross-currency facts recorded at the five lock moments
-- (migration 0065):
--
--   kind            lock moment                     fact table
--   --------------- ------------------------------- -------------------
--   expense         expense entry                   ticket_expenses
--   cost_line       purchase entry                  ticket_cost_lines
--   invoice_issue   invoice issue                   invoices
--   labor_pricing   time-entry costing (run price)  invoice_lines
--   inventory       inventory receipt (purchase)    device_assets
--   payment         payment settlement              invoice_payments
--
-- One row per fact that carries a conversion snapshot (currency ≠ org
-- base at the lock moment). Single-currency facts need no reconciliation
-- row — their original amount IS the base amount, and they are already
-- covered by the other ops.v1_* views.
--
-- ADDITIVITY CONTRACT: the union is NOT additive across `kind`. One
-- cross-currency invoice appears as an `invoice_issue` row (document
-- total), one `labor_pricing` row per priced line (lines sum to ≈ the
-- total), and one `payment` row per settlement — the same money seen
-- through three lock moments by design ("original amount, rate, base
-- equivalent" per fact). NEVER sum across kinds; aggregate per (kind) —
-- or per invoice via invoice_id — or the books double-count.
--
-- Columns: original amount + currency, the immutable snapshot (rate in
-- the fact→base direction, rate date, source), the base equivalent, the
-- org base currency for the row's window, and the books sync target.
-- `sync_target` is 'none' until an accounting export target is
-- configured for the deployment (qbo-export-sync / xero-export-sync are
-- per-deployment connectors; a per-org target registry is a Relay-side
-- concern riding this view — the core exposes the column now so the
-- packet shape is stable).
--
-- security_invoker: rows are org-filtered by the underlying tables' RLS.

-- Mirrors core_domain::fx::div_half_even so the view's recomputed fact
-- amount matches the API's locked snapshot bit-for-bit (the write path
-- computes quantity × unit cost through the same half-even rule —
-- migration 0065 / core-domain §5: no float arithmetic on money facts).
CREATE OR REPLACE FUNCTION fx_half_even_div(numer numeric, denom numeric)
RETURNS bigint
LANGUAGE sql
IMMUTABLE
AS $$
  SELECT CASE
    WHEN numer >= 0 THEN (
      floor(numer / denom)
      + CASE
          WHEN (numer - floor(numer / denom) * denom) * 2 > denom THEN 1
          WHEN (numer - floor(numer / denom) * denom) * 2 = denom
               AND floor(numer / denom)::bigint % 2 = 1 THEN 1
          ELSE 0
        END
    ) ELSE -(
      floor((-numer) / denom)
      + CASE
          WHEN ((-numer) - floor((-numer) / denom) * denom) * 2 > denom THEN 1
          WHEN ((-numer) - floor((-numer) / denom) * denom) * 2 = denom
               AND floor((-numer) / denom)::bigint % 2 = 1 THEN 1
          ELSE 0
        END
    )
  END::bigint
$$;

CREATE VIEW ops.v1_fx_reconciliation WITH (security_invoker = true) AS
SELECT
  e.org_id,
  'expense'::text AS kind,
  e.id AS fact_id,
  e.ticket_id AS ticket_id,
  NULL::uuid AS invoice_id,
  e.currency AS original_currency,
  e.amount_minor AS original_amount_minor,
  e.fx_rate_used,
  e.fx_rate_date,
  e.fx_source,
  e.base_amount_minor,
  'none'::text AS sync_target,
  e.incurred_at AS fact_at
FROM ticket_expenses e
WHERE e.fx_rate_used IS NOT NULL

UNION ALL

SELECT
  c.org_id,
  'cost_line'::text,
  c.id,
  c.ticket_id,
  NULL::uuid,
  c.currency,
  -- Same integer-space half-even product the API's lock path used when
  -- the snapshot was taken (fact_amount_minor) — recomputing it with a
  -- float multiply here would make base/original drift off fx_rate_used
  -- on .5-boundary quantities.
  fx_half_even_div(round(c.quantity * 1000000) * c.unit_cost_minor, 1000000)
    AS original_amount_minor,
  c.fx_rate_used,
  c.fx_rate_date,
  c.fx_source,
  c.base_amount_minor,
  'none'::text,
  c.created_at
FROM ticket_cost_lines c
WHERE c.fx_rate_used IS NOT NULL

UNION ALL

SELECT
  i.org_id,
  'invoice_issue'::text,
  i.id,
  NULL::uuid,
  i.id,
  i.currency,
  i.total_minor,
  i.fx_rate_used,
  i.fx_rate_date,
  i.fx_source,
  i.base_total_minor,
  'none'::text,
  i.created_at
FROM invoices i
WHERE i.fx_rate_used IS NOT NULL

UNION ALL

SELECT
  l.org_id,
  'labor_pricing'::text,
  l.id,
  NULL::uuid,
  l.invoice_id,
  (SELECT inv.currency FROM invoices inv WHERE inv.id = l.invoice_id),
  l.amount_minor,
  l.fx_rate_used,
  l.fx_rate_date,
  l.fx_source,
  l.base_amount_minor,
  'none'::text,
  (SELECT inv.created_at FROM invoices inv WHERE inv.id = l.invoice_id)
FROM invoice_lines l
WHERE l.fx_rate_used IS NOT NULL

UNION ALL

SELECT
  d.org_id,
  'inventory_receipt'::text,
  d.id,
  NULL::uuid,
  NULL::uuid,
  d.purchase_currency,
  d.purchase_cost_minor,
  d.purchase_fx_rate_used,
  d.purchase_fx_rate_date,
  d.purchase_fx_source,
  d.base_purchase_cost_minor,
  'none'::text,
  d.created_at
FROM device_assets d
WHERE d.purchase_fx_rate_used IS NOT NULL

UNION ALL

SELECT
  p.org_id,
  'payment'::text,
  p.id,
  NULL::uuid,
  p.invoice_id,
  p.currency,
  p.amount_minor,
  p.fx_rate_used,
  p.fx_rate_date,
  p.fx_source,
  p.base_amount_minor,
  'none'::text,
  p.received_at
FROM invoice_payments p
WHERE p.fx_rate_used IS NOT NULL;

-- Query-supporting partial indexes: the reconciliation endpoint reads
-- each branch filtered to the snapshot-bearing subset (fx_rate_used IS
-- NOT NULL) ordered by the fact timestamp. Without these, every request
-- full-scans all six fact tables; with them, the RLS org predicate, the
-- NOT-NULL filter, and the fact_at DESC top-N ordering are all served by
-- one index probe per branch. Additive-only (mirrors the 0065 convention).
CREATE INDEX IF NOT EXISTS ticket_expenses_fx_reconciliation_idx
  ON ticket_expenses (org_id, incurred_at DESC) WHERE fx_rate_used IS NOT NULL;
CREATE INDEX IF NOT EXISTS ticket_cost_lines_fx_reconciliation_idx
  ON ticket_cost_lines (org_id, created_at DESC) WHERE fx_rate_used IS NOT NULL;
CREATE INDEX IF NOT EXISTS invoices_fx_reconciliation_idx
  ON invoices (org_id, created_at DESC) WHERE fx_rate_used IS NOT NULL;
CREATE INDEX IF NOT EXISTS invoice_lines_fx_reconciliation_idx
  ON invoice_lines (org_id, invoice_id) WHERE fx_rate_used IS NOT NULL;
CREATE INDEX IF NOT EXISTS device_assets_fx_reconciliation_idx
  ON device_assets (org_id, created_at DESC) WHERE purchase_fx_rate_used IS NOT NULL;
CREATE INDEX IF NOT EXISTS invoice_payments_fx_reconciliation_idx
  ON invoice_payments (org_id, received_at DESC) WHERE fx_rate_used IS NOT NULL;
