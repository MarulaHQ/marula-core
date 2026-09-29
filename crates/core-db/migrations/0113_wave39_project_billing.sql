-- Wave 39 lane 4: project billing closure (report P1–P5; TASKLIST Wave 39
-- lane `p39-project-billing`; migration 0113 is this lane's exclusive
-- window, head is lane 3's 0112).
--
-- Purpose: connect the project→phase substrate to the invoicing pipeline.
-- Three seams, all additive (policy §4.1):
--
--   (B) project↔contract link — `projects.billing_contract_id`: a project
--       bills THROUGH one contract (its rate card, blocks, caps).
--       Nullable forever: non-billable projects keep NULL, and the link
--       validates only in the app layer (client match + status), the
--       0029 cross-column rule.
--
--   (E) project T&M without tickets — `time_entries.phase_id`: an entry
--       can scope to a phase (which implies its project) INSTEAD of a
--       ticket. Exactly ONE of (ticket_id, phase_id) must be present —
--       a CHECK both world can see, so no entry floats parentless. The
--       timer SEQUENCE keeps requiring a ticket (a timer always runs on
--       one); only manual/direct entry inserts can take the project path.
--
--   (C/D) phase budgets + fixed-fee releases — `project_phase_budgets`,
--       one optional budget row per phase: `budget_minor` + `currency`
--       (the burn math's denominator) and `fee_minor` + `fee_currency`
--       (the fixed fee a completed non-milestone phase releases). The
--       release ledger itself (`project_billing_releases`) is the
--       consumer's durable record: one row PER released milestone when
--       the invoice line actually lands on an invoice, so a re-run can
--       never double-bill and the burn model can never count money that
--       has not actually been invoiced.
--
-- Non-purpose: no per-phase rate cards (phase T&M rides the linked
-- contract's card — the link IS the rate source), no budget RLS rework
-- (projects already carry org RLS; these columns ride the same rows), no
-- scheduler/subscriber here (the worker consumer owns that machinery).

-- (B) the project bills through this contract. ON DELETE RESTRICT: a
-- contract with billing projects cannot be deleted while linked —
-- unlink explicitly first (the API layer owns that check).
ALTER TABLE projects ADD COLUMN billing_contract_id uuid
  REFERENCES contracts(id) ON DELETE RESTRICT;

-- (E) project-scoped entries. CHECK-enforced exactly-one-parent so the
-- NOT NULL on ticket_id relaxes to a mutual exclusivity story instead of
-- two truths.
ALTER TABLE time_entries ALTER COLUMN ticket_id DROP NOT NULL;
ALTER TABLE time_entries ADD COLUMN phase_id uuid
  REFERENCES project_phases(id) ON DELETE SET NULL;
ALTER TABLE time_entries ADD CONSTRAINT time_entries_one_parent_check
  CHECK (
    (ticket_id IS NOT NULL AND phase_id IS NULL) OR
    (ticket_id IS NULL AND phase_id IS NOT NULL)
  );
CREATE INDEX idx_entries_phase ON time_entries (phase_id);

-- Reports read model restatement (same discipline as 0073: CREATE OR
-- REPLACE, columns APPENDED at the end so the v1 order never moves).
-- The ticket join becomes LEFT: a project-scoped entry (P5) has no
-- ticket, and dropping it from every report would lie about capacity.
-- Phase/project context rides the two new tail columns.
CREATE OR REPLACE VIEW ops.v1_time_entries WITH (security_invoker = true) AS
SELECT e.id,
       e.org_id,
       e.ticket_id,
       t.number AS ticket_number,
       t.client_id,
       c.name AS client_name,
       e.user_id,
       u.display_name AS user_display_name,
       e.started_at,
       e.ended_at,
       e.minutes,
       e.billable,
       e.work_type,
       CAST(e.started_at AT TIME ZONE 'UTC' AS date) AS day_utc,
       e.timer_session_id,
       e.created_at,
       e.approval_status,
       e.phase_id
FROM time_entries e
LEFT JOIN tickets t ON t.id = e.ticket_id
LEFT JOIN project_phases pp ON pp.id = e.phase_id
LEFT JOIN projects p ON p.id = pp.project_id
LEFT JOIN clients pc ON pc.id = p.client_id
LEFT JOIN clients c ON c.id = COALESCE(t.client_id, pc.id)
LEFT JOIN users u ON u.id = e.user_id;

-- (C/D) the per-phase budget/fee record. One row per phase (a phase
-- carries at most one budget); the row is the single place the burn
-- read model and the fixed-fee release both read.
CREATE TABLE project_phase_budgets (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  phase_id uuid NOT NULL UNIQUE REFERENCES project_phases(id) ON DELETE CASCADE,
  -- (C) the contracted denominator: consumed minutes price against THIS
  -- when the phase carries a T&M budget, or the fee stands alone (D).
  budget_minor bigint CHECK (budget_minor IS NULL OR budget_minor >= 0),
  currency text,
  -- (D) the fee a completed NON-milestone phase releases (milestone
  -- phases use the phase's own milestone_amount columns — unchanged).
  fee_minor bigint CHECK (fee_minor IS NULL OR fee_minor >= 0),
  fee_currency text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  -- Money pairs validate in the app layer (0029 cross-column rule).
  CONSTRAINT phase_budget_shape CHECK (budget_minor IS NOT NULL OR fee_minor IS NOT NULL)
);
ALTER TABLE project_phase_budgets ENABLE ROW LEVEL SECURITY;
CREATE POLICY project_phase_budgets_org ON project_phase_budgets
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- (A/D) the durable release ledger. The engine emits candidates; this
-- table is the CONSUMER's record that a specific milestone release
-- reached a specific invoice line, keyed so a replay can never insert
-- twice. `phase_id NULL` = a task-milestone release (the event carries
-- task_id instead). `invoice_line_id` fills at run time — a pending row
-- (NULL line) bills on the NEXT run that reaches the window.
CREATE TABLE project_billing_releases (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  phase_id uuid REFERENCES project_phases(id) ON DELETE SET NULL,
  task_id uuid REFERENCES project_tasks(id) ON DELETE SET NULL,
  -- The milestone name AT RELEASE TIME (the event's name, frozen — the
  -- phase/task row may rename later; what was billed stands).
  label text NOT NULL,
  amount_minor bigint NOT NULL CHECK (amount_minor >= 0),
  currency text NOT NULL,
  released_on date NOT NULL,
  source_event_id uuid,
  status text NOT NULL DEFAULT 'pending'
    CHECK (status IN ('pending','billed','cancelled')),
  invoice_id uuid REFERENCES invoices(id) ON DELETE SET NULL,
  invoice_line_id uuid,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  -- One release per completed milestone: the same (project, phase) can
  -- release at most once as pending-or-billed; cancelled rows step
  -- aside so an admin can re-release after a correction (the lifecycle
  -- rides the API layer; cancelled is terminal here).
  UNIQUE (project_id, phase_id, status) WHERE status <> 'cancelled'
);
CREATE INDEX idx_billing_releases_org_project
  ON project_billing_releases (org_id, project_id, status);
ALTER TABLE project_billing_releases ENABLE ROW LEVEL SECURITY;
CREATE POLICY project_billing_releases_org ON project_billing_releases
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- (A/D) invoice line kind: the released-milestone candidate rides the
-- same invoice_lines table; CHECK widens and the description-keyed
-- dedupe index restates with the new kind (the ledger row's id rides
-- the description, so identity is (contract, released_on, description)
-- — the same shape fixed fees bill on).
ALTER TABLE invoice_lines DROP CONSTRAINT invoice_lines_kind_check;
ALTER TABLE invoice_lines ADD CONSTRAINT invoice_lines_kind_check
  CHECK (kind IN ('recurring_base','retainer_base','retainer_overage',
                  'time_materials','fixed_fee','per_unit_base','per_incident',
                  'block_sale','block_drawdown','cap_overage','minimum_trueup',
                  'change_order','sla_credit','escalator','project_release'));

DROP INDEX IF EXISTS uq_invoice_lines_fixed_fee;
CREATE UNIQUE INDEX uq_invoice_lines_fixed_fee
  ON invoice_lines (org_id, contract_id, kind, period_start, description)
  WHERE kind IN ('fixed_fee','block_sale','block_drawdown','cap_overage',
                 'change_order','sla_credit','per_incident','project_release');
