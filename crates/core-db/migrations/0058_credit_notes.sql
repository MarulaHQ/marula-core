-- Credit notes (the private build tracker Money+docs backend gap, recorded 2026-09-10;
-- migration 0058 is this lane's exclusive window).
--
-- Purpose: PSA-standard credit-note flow the invoicing surface lacked
-- (routes/invoices.rs shipped only void + payments + dunning):
--
--   * credit_notes — the credit document. Issued against a client
--     (optionally riding one invoice as the issuing context); money is
--     integer minor units + ISO 4217 currency, never converted (ADR 0002
--     doctrine: the credit's currency matches its invoice when one is
--     named, else the org's invoice currency at issue time). One state
--     machine as data, never side effects:
--       status: 'issued' → 'void' (document lifecycle).
--     Allocation is tracked as SUMS, not a status: applied_minor (moved
--     onto invoices) + refunded_minor (paid back out) against
--     total_minor ⇒ remaining = total − applied − refunded ≥ 0 is
--     enforced by a CHECK and by Rust validation on every write.
--
--   * credit_note_applications — one row per (credit note → invoice)
--     allocation. An application IS the money move: it inserts a real
--     invoice_payments receipt (provider 'credit_note', provider_ref =
--     the credit note number) through the SAME apply path payments use,
--     so invoice totals, paid-state transitions, and the overpayment
--     guards stay in one place. The row is the audit ledger; the
--     payment receipt is the invoice-side effect.
--
--   * refunds — refunded_minor on the credit note row; the refund is
--     recorded by the API (event + write trail + audit) — no external
--     PSP refund in v1 (the PaymentProvider seam has no refund method;
--     none-mode refunds approve locally exactly like charges).
--
-- Non-purpose: no negative invoices, no FX conversion (credits never
-- cross currencies), no automatic dunning interaction — applying a
-- credit can close an invoice through the ordinary paid transition, and
-- dunning states stay data.

CREATE TABLE credit_notes (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  number text NOT NULL,
  client_id uuid NOT NULL REFERENCES clients(id) ON DELETE RESTRICT,
  -- The invoice this credit was raised against (issuing context). NULL
  -- = a free-standing client credit. Applications may target ANY issued
  -- invoice of the same client + currency.
  issued_against_invoice_id uuid REFERENCES invoices(id) ON DELETE SET NULL,
  -- issued | void (document lifecycle). 'applied'/'refunded' are DERIVED
  -- from the counters below, never stored — they move continuously.
  status text NOT NULL DEFAULT 'issued'
    CHECK (status IN ('issued','void')),
  currency text NOT NULL,
  total_minor bigint NOT NULL CHECK (total_minor > 0),
  applied_minor bigint NOT NULL DEFAULT 0,
  refunded_minor bigint NOT NULL DEFAULT 0,
  reason text,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, number)
);
CREATE INDEX idx_credit_notes_client ON credit_notes (org_id, client_id);
CREATE INDEX idx_credit_notes_status ON credit_notes (org_id, status);

-- One (credit note, invoice) application pair per row; the SUM guard
-- lives in Rust (single-transaction validation) with the credit-note
-- row's remaining check as the concurrency backstop.
CREATE TABLE credit_note_applications (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  credit_note_id uuid NOT NULL REFERENCES credit_notes(id) ON DELETE CASCADE,
  invoice_id uuid NOT NULL REFERENCES invoices(id) ON DELETE RESTRICT,
  amount_minor bigint NOT NULL CHECK (amount_minor > 0),
  payment_id uuid REFERENCES invoice_payments(id) ON DELETE SET NULL,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_credit_note_applications_note
  ON credit_note_applications (credit_note_id, created_at DESC);
CREATE INDEX idx_credit_note_applications_invoice
  ON credit_note_applications (org_id, invoice_id);

ALTER TABLE credit_notes ENABLE ROW LEVEL SECURITY;
CREATE POLICY credit_notes_org ON credit_notes
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE credit_note_applications ENABLE ROW LEVEL SECURITY;
CREATE POLICY credit_note_applications_org ON credit_note_applications
  USING (org_id = current_setting('app.org_id', true)::uuid);
