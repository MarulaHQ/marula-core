-- Workflow constructs (Wave 11 lane 9, TASKLIST; migration 0056 is this
-- lane's exclusive window — 0055 reserved if the lane splits).
--
-- Four ticket-workflow constructs, additive-only (U9 gate: new tables plus
-- ONE nullable column on tickets; nothing existing is rewritten or dropped):
--
--   * tags — an org-wide tag taxonomy (flat labels with colors) applied to
--     tickets through the `ticket_tags` join. Filtering rides a semi-join
--     against the join table in the ticket list query (tickets.rs).
--
--   * ticket_links — typed ticket-to-ticket edges. v1 types:
--       'parent_child'   — a child ticket rolls work up to its parent;
--       'major_incident' — a member ticket is folded into a major incident.
--     Links are org-scoped, audited on every mutation, and read as ROLLUPS
--     (child/member counts + summed time + summed billable costs).
--
--   * merge — a survivor/merged pair of tickets. The source ticket's
--     children (via repointed links), comments, time entries, cost lines,
--     and expenses FOLD INTO the survivor in one audited transaction; the
--     source keeps its row but is marked `merged_into_ticket_id` and
--     closed. Typed conflicts (board mismatch, closed survivor, self-merge,
--     already-merged source) refuse the merge before any write.
--
--   * ticket_cost_lines + ticket_expenses — per-ticket money lines: parts /
--     flat / other cost lines and expense entries, each with a `billable`
--     flag. Billable-but-unbilled rows are the invoicing candidate feed:
--     `invoice_cost_line_id` / `invoice_expense_id` on invoice_lines' twin
--     columns mark consumption, mirroring `invoice_time_entries` ("billed
--     once, ever") without touching the Wave 9 invoicing schema.
--
--   * pm_schedules + pm_runs — recurring/scheduled tickets (PM schedules):
--     a template (board + typed cadence, same shape as report_schedules)
--     the worker tick claims when due; core-api generates the ticket
--     instance through the normal write path and advances the cadence in
--     the same transaction. `pm_runs` is the append-only run ledger —
--     generated ticket, slot, outcome — so the ledger is the run record
--     and the worker stays read-only on domain tables (§4.1).
--
-- Non-purpose: no tag colors validation beyond length (UI concern), no
-- cross-org tag sharing, no expense receipts/attachments (attachments'
-- existing surface covers files), no cron strings, no per-ticket billing
-- engine (candidates are listed; invoicing consumes them).

-- --- tags ---------------------------------------------------------------------

CREATE TABLE tags (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  color text,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, name)
);
CREATE INDEX idx_tags_org ON tags (org_id, name);

ALTER TABLE tags ENABLE ROW LEVEL SECURITY;
CREATE POLICY tags_org ON tags
  USING (org_id = current_setting('app.org_id', true)::uuid);

CREATE TABLE ticket_tags (
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  ticket_id uuid NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
  tag_id uuid NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
  applied_by uuid REFERENCES users(id) ON DELETE SET NULL,
  applied_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (ticket_id, tag_id)
);
CREATE INDEX idx_ticket_tags_org_tag ON ticket_tags (org_id, tag_id);

ALTER TABLE ticket_tags ENABLE ROW LEVEL SECURITY;
CREATE POLICY ticket_tags_org ON ticket_tags
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- --- ticket links (parent-child, major-incident) --------------------------------

CREATE TABLE ticket_links (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  from_ticket_id uuid NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
  to_ticket_id uuid NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
  -- 'parent_child' (from = child, to = parent) | 'major_incident'
  -- (from = member, to = the major incident).
  link_type text NOT NULL CHECK (link_type IN ('parent_child','major_incident')),
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (from_ticket_id, to_ticket_id, link_type)
);
CREATE INDEX idx_ticket_links_to ON ticket_links (org_id, to_ticket_id, link_type);
CREATE INDEX idx_ticket_links_from ON ticket_links (org_id, from_ticket_id);

ALTER TABLE ticket_links ENABLE ROW LEVEL SECURITY;
CREATE POLICY ticket_links_org ON ticket_links
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- Merge marker (additive): the survivor this ticket folded into. A
-- merged ticket keeps its row (history stays queryable — doctrine #9)
-- but is closed via its status and never re-opened.
ALTER TABLE tickets
  ADD COLUMN merged_into_ticket_id uuid REFERENCES tickets(id) ON DELETE SET NULL;
CREATE INDEX idx_tickets_merged_into ON tickets (merged_into_ticket_id)
  WHERE merged_into_ticket_id IS NOT NULL;

-- --- ticket costs & expenses -----------------------------------------------------

CREATE TABLE ticket_cost_lines (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  ticket_id uuid NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
  -- 'parts' | 'flat' | 'other'
  kind text NOT NULL CHECK (kind IN ('parts','flat','other')),
  description text NOT NULL,
  quantity numeric NOT NULL DEFAULT 1 CHECK (quantity > 0),
  -- Minor-unit unit cost; line total is quantity * unit_cost_minor.
  unit_cost_minor bigint NOT NULL CHECK (unit_cost_minor >= 0),
  currency text NOT NULL,
  billable boolean NOT NULL DEFAULT false,
  -- Consumption marker: the invoice line this cost became, once billed.
  invoice_line_id uuid,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_ticket_cost_lines_ticket ON ticket_cost_lines (ticket_id, created_at);
CREATE INDEX idx_ticket_cost_lines_billable
  ON ticket_cost_lines (org_id, ticket_id) WHERE billable AND invoice_line_id IS NULL;

ALTER TABLE ticket_cost_lines ENABLE ROW LEVEL SECURITY;
CREATE POLICY ticket_cost_lines_org ON ticket_cost_lines
  USING (org_id = current_setting('app.org_id', true)::uuid);

CREATE TABLE ticket_expenses (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  ticket_id uuid NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
  description text NOT NULL,
  amount_minor bigint NOT NULL CHECK (amount_minor >= 0),
  currency text NOT NULL,
  billable boolean NOT NULL DEFAULT false,
  -- Consumption marker: the invoice line this expense became, once billed.
  invoice_line_id uuid,
  incurred_by uuid REFERENCES users(id) ON DELETE SET NULL,
  incurred_at timestamptz NOT NULL DEFAULT now(),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_ticket_expenses_ticket ON ticket_expenses (ticket_id, incurred_at);
CREATE INDEX idx_ticket_expenses_billable
  ON ticket_expenses (org_id, ticket_id) WHERE billable AND invoice_line_id IS NULL;

ALTER TABLE ticket_expenses ENABLE ROW LEVEL SECURITY;
CREATE POLICY ticket_expenses_org ON ticket_expenses
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- --- PM schedules (recurring/scheduled tickets) -----------------------------------

CREATE TABLE pm_schedules (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  board_id uuid NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  -- Ticket template applied on every generated instance.
  title text NOT NULL,
  body text,
  priority text NOT NULL DEFAULT 'p3' CHECK (priority IN ('p1','p2','p3','p4')),
  client_id uuid REFERENCES clients(id) ON DELETE SET NULL,
  site_id uuid REFERENCES sites(id) ON DELETE SET NULL,
  assignee_id uuid REFERENCES users(id) ON DELETE SET NULL,
  -- Typed cadence, identical shape to report_schedules (0038).
  cadence text NOT NULL CHECK (cadence IN ('daily','weekly','monthly')),
  time_of_day time NOT NULL,
  day_of_week smallint CHECK (day_of_week BETWEEN 0 AND 6),
  day_of_month smallint CHECK (day_of_month BETWEEN 1 AND 28),
  timezone text NOT NULL DEFAULT 'UTC',
  enabled boolean NOT NULL DEFAULT true,
  -- Materialized claim state: due when enabled AND next_run_at <= now().
  next_run_at timestamptz NOT NULL,
  last_run_at timestamptz,
  last_error text,
  created_by uuid NOT NULL REFERENCES users(id),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, name)
);

ALTER TABLE pm_schedules
  ADD CONSTRAINT pm_schedules_cadence_shape CHECK (
    (cadence = 'daily'    AND day_of_week IS NULL     AND day_of_month IS NULL)
    OR (cadence = 'weekly'  AND day_of_week IS NOT NULL AND day_of_month IS NULL)
    OR (cadence = 'monthly' AND day_of_week IS NULL     AND day_of_month IS NOT NULL)
  );

CREATE INDEX idx_pm_schedules_org ON pm_schedules (org_id, name);
CREATE INDEX idx_pm_schedules_due ON pm_schedules (next_run_at) WHERE enabled;

ALTER TABLE pm_schedules ENABLE ROW LEVEL SECURITY;
CREATE POLICY pm_schedules_org ON pm_schedules
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- Append-only run ledger: one row per generated instance (the slot the
-- schedule discharged). Unique per (schedule, slot) so a retried job can
-- never double-generate; the ledger is the run record — the worker emits
-- nothing (mirrors the report-delivery doctrine).
CREATE TABLE pm_runs (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  schedule_id uuid NOT NULL REFERENCES pm_schedules(id) ON DELETE CASCADE,
  -- The claimed slot (== the schedule's next_run_at at claim time).
  slot_at timestamptz NOT NULL,
  -- generated | skipped (schedule disabled between claim and apply).
  status text NOT NULL CHECK (status IN ('generated','skipped')),
  ticket_id uuid REFERENCES tickets(id) ON DELETE SET NULL,
  detail jsonb NOT NULL DEFAULT '{}' CHECK (jsonb_typeof(detail) = 'object'),
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (schedule_id, slot_at)
);
CREATE INDEX idx_pm_runs_schedule ON pm_runs (schedule_id, slot_at DESC);

ALTER TABLE pm_runs ENABLE ROW LEVEL SECURITY;
CREATE POLICY pm_runs_org ON pm_runs
  USING (org_id = current_setting('app.org_id', true)::uuid);