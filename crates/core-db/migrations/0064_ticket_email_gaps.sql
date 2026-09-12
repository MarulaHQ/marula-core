-- Wave 12 lane 6 (p12-ticket-email-gaps): ticket/email follow-up bundle.
--
-- Purpose (two schema deltas; the lane's other deliverables are code-only):
--   1. `checklist_items.na` — server-side "not applicable" state so N-A is
--      SHARED truth across every member instead of per-user panel state.
--      Backward-compatible by construction: every pre-existing row reads
--      na=false, and `checked` semantics are untouched — N-A is a separate
--      column, never an overload of `checked` (a checked item can never be
--      N-A and vice versa; the API layer enforces the mutual exclusion).
--      Close-time enforcement and derived progress treat na=true required
--      items as exempt, which changes nothing for existing rows.
--   2. `org_pod_visibility` — the org-scoped store for the ticket-workspace
--      admin per-RBAC pod-visibility matrix, previously device-local via
--      the panel-state engine. One row per org; `config` is the JSONB
--      matrix pod_id → RBAC level → visible. Absent pods/levels fall back
--      to the client registry defaults (defaults-first doctrine), so an
--      org that never touches the matrix keeps today's behavior.
--
-- Non-purpose: no email schema — the per-ticket email read endpoint reads
-- the existing `email_messages` + `email_outbound` ledger (0014); no
-- settings rows — the email greeting/sign-off are registry declarations
-- resolved defaults-first through the existing settings store (0018); no
-- scheduling schema — the template auto-email now rides the existing
-- `email_outbound` exactly-once ledger.

ALTER TABLE checklist_items ADD COLUMN na boolean NOT NULL DEFAULT false;

-- The admin per-RBAC pod-visibility matrix:
-- { "<pod_id>": { "<rbac_level>": true|false } }.
-- Shape validation (keys bounded, levels whitelisted) lives in core-api;
-- the store is deliberately opaque JSONB so the pod registry can grow
-- without schema churn.
CREATE TABLE org_pod_visibility (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL UNIQUE REFERENCES orgs(id) ON DELETE CASCADE,
  config jsonb NOT NULL DEFAULT '{}'::jsonb,
  updated_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE org_pod_visibility ENABLE ROW LEVEL SECURITY;
CREATE POLICY org_pod_visibility_org ON org_pod_visibility
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

-- The public booking surface now enqueues the template auto-email through
-- the `email_outbound` exactly-once ledger INSIDE the booking transaction
-- (a transport failure after commit retries via the worker instead of
-- being logged and lost). The confirmation rides the ledger's ticket_id
-- so it also renders in the originating ticket's email timeline; the
-- token resolver therefore carries the link's ticket cross-ref out of
-- the definer seam. Postgres cannot CREATE OR REPLACE a function whose
-- OUT parameters changed, so this is a DROP + CREATE of the function
-- object only — pure code, no data — followed by the same EXECUTE grant
-- 0055 issued (dropping a function drops its grants).
DROP FUNCTION IF EXISTS scheduling_link_resolve(text);
CREATE FUNCTION scheduling_link_resolve(p_token_hash text)
RETURNS TABLE (
  link_id uuid, org_id uuid, meeting_type_id uuid, link_status text,
  expires_at timestamptz, label text, meeting_type_name text,
  meeting_type_description text, duration_minutes integer,
  buffer_minutes integer, booking_window_days integer, parameters jsonb,
  assignee_scope text, assignee_user_id uuid, assignee_group_id uuid,
  auto_email_enabled boolean, auto_email_subject text, auto_email_body text,
  meeting_type_is_active boolean,
  first_viewed_at timestamptz, view_count integer, scheduled_at timestamptz,
  r_ticket_id uuid
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
AS $$
  SELECT l.id, l.org_id, mt.id, l.status, l.expires_at, l.label,
         mt.name, mt.description, mt.duration_minutes, mt.buffer_minutes,
         mt.booking_window_days, mt.parameters,
         mt.assignee_scope, mt.assignee_user_id, mt.assignee_group_id,
         mt.auto_email_enabled, mt.auto_email_subject, mt.auto_email_body,
         mt.is_active, l.first_viewed_at, l.view_count, l.scheduled_at,
         l.ticket_id
  FROM scheduling_links l
  JOIN meeting_types mt ON mt.id = l.meeting_type_id
  WHERE l.token_hash = $1
$$;

REVOKE ALL ON FUNCTION scheduling_link_resolve(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION scheduling_link_resolve(text) TO meridian_app;
