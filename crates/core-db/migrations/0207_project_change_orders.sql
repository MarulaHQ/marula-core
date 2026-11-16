-- Migration 0207: change orders.
--
-- A change order is the DOCUMENT that records a change to the agreement; a
-- billing release is the ACT that puts money on an invoice. They are separate
-- rows because the surface pack is explicit that approval never bills, apply
-- never bills, and a failed release is its own state with the plan already
-- changed (S13.6, S13.8). Keeping them apart is what makes "applied but
-- billing failed" describable instead of a half-finished row.
--
--   * status is the document's lifecycle: draft → submitted → approved or
--     rejected → (awaiting_client) → applied → billed. `billing_failed` is an
--     applied row whose release was refused by the billing seam.
--
--   * scope_changes is the plan consequence in a small, inspectable shape:
--     additional tasks, removed tasks, date shifts and added effort. Apply
--     consumes it; the record shows it whether or not anyone applies.
--
--   * evidence is what the draft was raised from (a scope-drift signal, the
--     contract clause, the client's own message). It is stored so the same
--     claim can be checked later and so the model can cite rather than assert.
--
--   * amount_minor/currency are the proposed change to the agreement. They are
--     nullable: a zero-value change (a date extension with no money) is a real
--     change order and must not be forced to invent a figure.
--
--   * client_ack_required is the org option (off by default). When it is on,
--     an approved order waits for the portal contact before it can be applied.
--
-- Compat: additive (policy §4.1) — one new table, no destructive statements.

CREATE TABLE project_change_orders (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  reference text NOT NULL,
  title text NOT NULL,
  status text NOT NULL DEFAULT 'draft'
    CHECK (status IN ('draft','submitted','approved','rejected','awaiting_client',
                      'applied','billing_failed','billed')),
  -- How the draft arrived: written by hand, raised from a stored scope-drift
  -- signal, or sent from the client portal.
  source text NOT NULL DEFAULT 'manual'
    CHECK (source IN ('manual','scope_signal','client_request')),
  -- The four-part draft: why, what changes in the plan, the amount, and what
  -- the client is told.
  reason text NOT NULL DEFAULT '',
  scope_changes jsonb NOT NULL DEFAULT '{}'::jsonb,
  client_summary text,
  evidence jsonb NOT NULL DEFAULT '[]'::jsonb,
  -- The assistant's coverage call, when one was made. It is prose plus a
  -- confidence word, never a number, and it never sets the amount.
  coverage_note text,
  confidence text CHECK (confidence IS NULL OR confidence IN ('strong','fair','unsure')),
  contract_clause text,
  amount_minor bigint,
  currency text,
  submitted_at timestamptz,
  submitted_by uuid REFERENCES users(id) ON DELETE SET NULL,
  decided_at timestamptz,
  decided_by uuid REFERENCES users(id) ON DELETE SET NULL,
  decision_note text,
  applied_at timestamptz,
  applied_by uuid REFERENCES users(id) ON DELETE SET NULL,
  applied_baseline_id uuid REFERENCES project_baselines(id) ON DELETE SET NULL,
  billed_at timestamptz,
  billing_release_id uuid REFERENCES project_billing_releases(id) ON DELETE SET NULL,
  billing_error text,
  client_ack_required boolean NOT NULL DEFAULT false,
  client_ack_at timestamptz,
  client_ack_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  -- A money amount is never alone: an amount without a currency is a typo, and
  -- the pair is what the billing seam needs.
  CHECK ((amount_minor IS NULL) = (currency IS NULL)),
  UNIQUE (org_id, reference)
);
CREATE INDEX idx_project_change_orders_project
  ON project_change_orders (org_id, project_id, created_at DESC);
CREATE INDEX idx_project_change_orders_status
  ON project_change_orders (org_id, status, updated_at DESC);

ALTER TABLE project_change_orders ENABLE ROW LEVEL SECURITY;
CREATE POLICY project_change_orders_org ON project_change_orders
  USING (org_id = current_setting('app.org_id', true)::uuid);
