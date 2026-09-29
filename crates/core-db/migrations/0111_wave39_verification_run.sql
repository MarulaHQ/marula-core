-- Billing verification run (Wave 39 lane 2, `p39-verification-run`;
-- migration 0111 is this lane's exclusive window; head was 0109 at lane
-- start — 0110/0112/0113 from sibling lanes landed before this file).
--
-- Purpose: the verification run seams (report V1/V2/V7/V8/V9). The run
-- exists, the preview exists, but nothing lets an admin verify a period
-- BEFORE invoicing it: no per-entry dispositions, no minute adjustments
-- with evidence, no close ritual, no run-window overlap warning. This
-- migration adds the ledger rows the workspace reads and writes:
--
--   * `time_entry_adjustments` — verification-minute edits (reduce,
--     zero). REASON REQUIRED (the column is NOT NULL); pre-run only.
--     The additive rule holds because the ORIGINAL entry row is never
--     rewritten for billing purposes: the adjustment is a separate
--     row, the run sums (entry.minutes - approved adjustment) instead.
--   * `time_entry_dispositions` — the human disposition per entry
--     (approved / rejected / needs_review); at most one CURRENT row per
--     entry (history kept — verification evidence must survive).
--   * `invoice_run_signoffs` — the close ritual (report V8): checklist
--     booleans + reviewer identity recorded per run, one row per run.
--
-- Invoiced entries stay locked WITHOUT a new table: `invoice_time_
-- entries` already links consumed entries (UNIQUE (time_entry_id));
-- the API layer refuses any verification write on an entry that row
-- links. The lock is enforced at the write seam, not by copying state.
--
-- Overlap warning (V8) needs no table either: invoice_runs.period_from/
-- period_to already hold every issued window — the workspace GET checks
-- (from, to) against them and returns a typed warning.
--
-- Additive only: three new org-scoped RLS tables, three indexes.
-- Nothing existing is rewritten or dropped.

-- (V2) verification adjustments. One row per adjustment action on one
-- entry. `delta_minutes` is signed: negative reduces the billed
-- minutes, zero zeroes the entry out for billing. The reason is the
-- evidence (V9) — an adjustment without it is a DB-level impossibility.
CREATE TABLE time_entry_adjustments (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  time_entry_id uuid NOT NULL REFERENCES time_entries(id) ON DELETE CASCADE,
  run_id uuid REFERENCES invoice_runs(id) ON DELETE SET NULL,
  -- The minutes BEFORE the adjustment (denormalized evidence).
  original_minutes integer NOT NULL CHECK (original_minutes >= 0),
  -- Signed: negative = fewer billed minutes. CHECK bars positive and
  -- delta < -original at the API layer (effective_minutes >= 0).
  delta_minutes integer NOT NULL CHECK (delta_minutes <= 0),
  reason text NOT NULL CHECK (char_length(btrim(reason)) BETWEEN 1 AND 500),
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_time_entry_adjustments_entry
  ON time_entry_adjustments (org_id, time_entry_id);
CREATE INDEX idx_time_entry_adjustments_run
  ON time_entry_adjustments (org_id, run_id) WHERE run_id IS NOT NULL;

ALTER TABLE time_entry_adjustments ENABLE ROW LEVEL SECURITY;
CREATE POLICY time_entry_adjustments_org ON time_entry_adjustments
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- (V1) per-entry dispositions. Status vocabulary matches the workspace
-- badges; `rejected` and `non_billable` require the typed unbillable
-- reason (V3) — app-layer validation, legacy rows predate it. One
-- CURRENT row per entry via the partial unique index; superseding a
-- disposition flips its `current` flag (history preserved, evidence
-- survives: verification down the line can prove what was decided
-- when, and by whom).
CREATE TABLE time_entry_dispositions (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  time_entry_id uuid NOT NULL REFERENCES time_entries(id) ON DELETE CASCADE,
  status text NOT NULL
    CHECK (status IN ('approved','rejected','non_billable','needs_review')),
  unbillable_reason text,
  note text CHECK (note IS NULL OR char_length(note) <= 1000),
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  current boolean NOT NULL DEFAULT true
);
-- At most ONE current disposition per entry (org scoping is implied by
-- the entry FK). A partial unique index — not a table-level UNIQUE —
-- keeps the history rows exempt.
CREATE UNIQUE INDEX uq_time_entry_dispositions_current
  ON time_entry_dispositions (time_entry_id) WHERE current;
CREATE INDEX idx_time_entry_dispositions_entry
  ON time_entry_dispositions (org_id, time_entry_id, created_at);

ALTER TABLE time_entry_dispositions ENABLE ROW LEVEL SECURITY;
CREATE POLICY time_entry_dispositions_org ON time_entry_dispositions
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- (V8) the close ritual. A reviewer signs off the WINDOW (contract +
-- period) BEFORE the run fires, so the row is keyed by that window and
-- one at a time; `run_id` is the run that later claimed the sign-off
-- (the run create path stamps it). The checklist booleans are the
-- evidence, the reviewer the identity — the copy ("a week closes only
-- after a human says it is right") gets a row to point at.
CREATE TABLE invoice_run_signoffs (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  contract_id uuid NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
  period_from date NOT NULL,
  period_to date NOT NULL,
  -- The run that claimed this sign-off; NULL until a run covers the
  -- window (a re-sign after a failed run clears it back to NULL).
  run_id uuid REFERENCES invoice_runs(id) ON DELETE SET NULL,
  entries_dispositioned boolean NOT NULL DEFAULT false,
  zero_unclassified boolean NOT NULL DEFAULT false,
  preview_seen boolean NOT NULL DEFAULT false,
  reviewer_id uuid REFERENCES users(id) ON DELETE SET NULL,
  reviewer_note text CHECK (reviewer_note IS NULL OR char_length(reviewer_note) <= 1000),
  signed_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, contract_id, period_from, period_to),
  CHECK (period_from <= period_to)
);
CREATE INDEX idx_invoice_run_signoffs_org
  ON invoice_run_signoffs (org_id, contract_id, signed_at DESC);
CREATE INDEX idx_invoice_run_signoffs_run
  ON invoice_run_signoffs (org_id, run_id) WHERE run_id IS NOT NULL;

ALTER TABLE invoice_run_signoffs ENABLE ROW LEVEL SECURITY;
CREATE POLICY invoice_run_signoffs_org ON invoice_run_signoffs
  USING (org_id = current_setting('app.org_id', true)::uuid);
