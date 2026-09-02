-- Portal v2 (TASKLIST Phase 3 "Portal v2: invoice view+pay, KB
-- self-service, appointment requests"; Wave 9 lane 3 — migration 0039 is
-- this lane's exclusive window).
--
-- Purpose: ONE new table — `portal_appointment_requests` — the intent row
-- between a client portal user and a staff calendar. Invoices and KB
-- self-service need NO new storage: the portal reads the existing
-- `invoices`/`invoice_lines`/`invoice_payments` (0036) and
-- `kb_categories`/`kb_articles` (0031) rows through client-scoped queries,
-- so those surfaces stay upgrade-stable with their owning lanes.
--
-- Appointment-request doctrine:
--   * A request is an INTENT, not a booking. Several clients may hold
--     overlapping pending requests; the staff approval decision is the
--     single point that turns one request into a real `appointments` row
--     (0022), after the slot is re-validated (availability containment +
--     no live booking/on-call overlap). No double-booking can enter
--     through this table because nothing here occupies a calendar.
--   * Status is a closed, forward-moving vocabulary:
--       pending → approved   (staff; links the created appointment)
--       pending → declined   (staff; reason optional)
--       pending → cancelled  (the requesting portal user withdrawing it)
--     approved/declined/cancelled are terminal — a new request books a
--     new slot, history is never rewritten.
--   * Provenance: `portal_user_id` pins WHO on the client side asked;
--     `decided_by` pins which staff user decided. Portal identities are
--     not staff rows, so nothing references users(id) on the client half.
--
-- The `kind` column reuses the 0022 `appointment_kind` enum additively so
-- the approval path can copy it straight onto the created appointment.
--
-- RLS: org-scoped policy like every user-data table (CI gate
-- `assert_rls_coverage`); portal/user scoping happens in the query layer
-- on top, exactly like portal v1 tickets.

CREATE TABLE portal_appointment_requests (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  client_id uuid NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
  -- The requesting portal identity (contact/approver/it_manager).
  portal_user_id uuid NOT NULL REFERENCES portal_users(id) ON DELETE CASCADE,
  -- The staff member whose calendar the client asked for.
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  title text NOT NULL,
  starts_at timestamptz NOT NULL,
  ends_at timestamptz NOT NULL,
  kind appointment_kind NOT NULL DEFAULT 'remote',
  notes text,
  status text NOT NULL DEFAULT 'pending'
    CHECK (status IN ('pending','approved','declined','cancelled')),
  decision_note text,
  decided_by uuid REFERENCES users(id) ON DELETE SET NULL,
  decided_at timestamptz,
  -- Set ONLY by the approval path; ON DELETE SET NULL keeps the history
  -- row meaningful even if the appointment is later deleted.
  appointment_id uuid REFERENCES appointments(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CHECK (ends_at > starts_at)
);

CREATE INDEX idx_portal_appt_requests_client ON portal_appointment_requests (org_id, client_id, created_at DESC);
CREATE INDEX idx_portal_appt_requests_status ON portal_appointment_requests (org_id, status, created_at DESC);
CREATE INDEX idx_portal_appt_requests_staff ON portal_appointment_requests (org_id, user_id, starts_at);

ALTER TABLE portal_appointment_requests ENABLE ROW LEVEL SECURITY;
CREATE POLICY portal_appointment_requests_org ON portal_appointment_requests
  USING (org_id = current_setting('app.org_id', true)::uuid);
