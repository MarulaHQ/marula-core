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
  (c.quantity * c.unit_cost_minor)::bigint AS original_amount_minor,
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
