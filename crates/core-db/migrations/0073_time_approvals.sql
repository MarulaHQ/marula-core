-- Wave 14 lane 1 `p14-time-approvals` — time-entry approval workflows.
--
-- Lifecycle: draft → submitted → approved | rejected (rejected → draft on
-- reopen/edit). Approval state lives ON the entry (single source of truth,
-- queryable), while every transition is appended to an audit table that
-- also provides the per-entry monotonic decision sequence.
--
-- Locked periods after invoicing: an entry referenced by
-- `invoice_time_entries` (migration 0036) is frozen — no submit, decide,
-- reopen, or content edit. The lock is a read of the existing link table,
-- not new state; the API layer enforces it with a typed reason code.
--
-- Existing entries backfill to 'draft': the pre-approval world was
-- implicitly unapproved, so nothing retroactively approves.

ALTER TABLE time_entries
  ADD COLUMN approval_status text NOT NULL DEFAULT 'draft'
    CHECK (approval_status IN ('draft', 'submitted', 'approved', 'rejected')),
  ADD COLUMN approval_submitted_at timestamptz,
  ADD COLUMN approval_submitted_by uuid REFERENCES users(id) ON DELETE SET NULL,
  ADD COLUMN approval_decided_at timestamptz,
  ADD COLUMN approval_decided_by uuid REFERENCES users(id) ON DELETE SET NULL,
  ADD COLUMN approval_reason_code text;

-- Approver queue: submitted entries, oldest submission first.
CREATE INDEX idx_entries_approval_queue
  ON time_entries (org_id, approval_submitted_at)
  WHERE approval_status = 'submitted';

-- Append-only decision trail: who did what to which entry, with the typed
-- rejection reason code when the action is a rejection. `seq` is per-entry
-- monotonic so clients can order a timeline without clock trust.
CREATE TABLE time_entry_approvals (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  time_entry_id uuid NOT NULL REFERENCES time_entries(id) ON DELETE CASCADE,
  seq integer NOT NULL,
  action text NOT NULL CHECK (action IN ('submitted', 'approved', 'rejected', 'reopened')),
  actor_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  reason_code text,
  note text,
  created_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT uq_time_entry_approval_seq UNIQUE (time_entry_id, seq)
);
CREATE INDEX idx_time_entry_approvals_entry ON time_entry_approvals (time_entry_id, seq);
CREATE INDEX idx_time_entry_approvals_actor ON time_entry_approvals (org_id, actor_id, created_at DESC);

ALTER TABLE time_entry_approvals ENABLE ROW LEVEL SECURITY;
CREATE POLICY time_entry_approvals_org ON time_entry_approvals
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- Reports read model: carry the approval state through ops.v1_time_entries
-- (column appended at the end — CREATE OR REPLACE keeps the v1 order).
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
       e.approval_status
FROM time_entries e
JOIN tickets t ON t.id = e.ticket_id
LEFT JOIN clients c ON c.id = t.client_id
LEFT JOIN users u ON u.id = e.user_id;
