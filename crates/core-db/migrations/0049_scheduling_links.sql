-- Calendly-style scheduling links (Wave 11 lane 4; migration 0049).
--
-- Purpose: client-facing booking links built on the 2.12 availability
-- foundation:
--   * `scheduling_groups` — FREE-FORM admin-defined groups/departments
--     ("Service Desk", "Networking"). Deliberately NOT the org chart:
--     no reporting lines, no permissions, pure availability pools.
--   * `meeting_types` — bookable templates (duration, buffer, booking
--     window, editable `parameters` JSON) whose assignee scope is either
--     ONE user's calendar or the UNIONED availability of a group.
--   * `scheduling_links` — shareable, revocable tokens (SHA-256 stored;
--     the raw token is shown once at create). Lifecycle fields
--     (`first_viewed_at`, `scheduled_at`, `revoked_at`, `expires_at`)
--     give the tech the created / viewed / scheduled / expired trail.
--   * `appointments.scheduling_link_id` — a completed booking links back
--     to its link (and carries the ticket cross-ref set at link create).
--
-- Booking validation itself is code-side (core-api re-checks windows −
-- exclusions − bookings at book time exactly like portal appointment
-- requests); the appointments EXCLUDE constraint (0022) remains the
-- race-safety backstop.
--
-- RLS: org-scoped like every tenant table (0047/0048 pattern). The
-- PUBLIC booking endpoint resolves the token through the narrow
-- SECURITY DEFINER lookup `scheduling_link_resolve` (0014 seam
-- pattern) — the definer function reads before any org scope exists,
-- and every subsequent query inside the resolved transaction runs with
-- `app.org_id` bound.

CREATE TABLE scheduling_groups (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  description text,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, name)
);

CREATE TABLE scheduling_group_members (
  group_id uuid NOT NULL REFERENCES scheduling_groups(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  added_by uuid REFERENCES users(id) ON DELETE SET NULL,
  added_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (group_id, user_id)
);
CREATE INDEX idx_scheduling_group_members_user ON scheduling_group_members (user_id);

CREATE TABLE meeting_types (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  description text,
  duration_minutes integer NOT NULL CHECK (duration_minutes BETWEEN 5 AND 480),
  buffer_minutes integer NOT NULL DEFAULT 0 CHECK (buffer_minutes BETWEEN 0 AND 120),
  -- How many days ahead a client may book.
  booking_window_days integer NOT NULL DEFAULT 14 CHECK (booking_window_days BETWEEN 1 AND 365),
  -- Org-editable free-form parameters (location hint, agenda, questions…).
  -- Editable per the QA ask; behavior keys off nothing here (§5 doctrine
  -- for classification-only trees applies by analogy: consumers must
  -- tolerate any shape).
  parameters jsonb NOT NULL DEFAULT '{}'::jsonb,
  -- 'user' = one staff calendar; 'group' = unioned group availability.
  assignee_scope text NOT NULL CHECK (assignee_scope IN ('user', 'group')),
  assignee_user_id uuid REFERENCES users(id) ON DELETE CASCADE,
  assignee_group_id uuid REFERENCES scheduling_groups(id) ON DELETE CASCADE,
  -- Auto-email sent on a completed booking (client confirmation).
  auto_email_enabled boolean NOT NULL DEFAULT false,
  auto_email_subject text,
  auto_email_body text,
  is_active boolean NOT NULL DEFAULT true,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  -- Exactly one assignee target, matching the declared scope.
  CONSTRAINT meeting_types_scope_target CHECK (
    (assignee_scope = 'user' AND assignee_user_id IS NOT NULL AND assignee_group_id IS NULL)
    OR
    (assignee_scope = 'group' AND assignee_group_id IS NOT NULL AND assignee_user_id IS NULL)
  )
);
CREATE INDEX idx_meeting_types_org ON meeting_types (org_id) WHERE is_active = true;

CREATE TABLE scheduling_links (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  meeting_type_id uuid NOT NULL REFERENCES meeting_types(id) ON DELETE RESTRICT,
  -- SHA-256 of the raw token; the raw token is returned ONCE by the
  -- create response and never stored.
  token_hash text NOT NULL UNIQUE,
  label text,
  -- Optional ticket cross-ref so a completed booking lands back on the
  -- originating ticket (the send-UI lane's anchor).
  ticket_id uuid REFERENCES tickets(id) ON DELETE SET NULL,
  -- Soft state: 'active' | 'revoked' | 'used'. 'used' = a booking was
  -- made through it (single-use semantics; expiry is DERIVED from
  -- expires_at + the meeting type's booking window, never stored).
  status text NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'revoked', 'used')),
  expires_at timestamptz,
  first_viewed_at timestamptz,
  view_count integer NOT NULL DEFAULT 0,
  scheduled_at timestamptz,
  scheduled_appointment_id uuid,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_scheduling_links_org ON scheduling_links (org_id, created_at DESC);
CREATE INDEX idx_scheduling_links_ticket ON scheduling_links (ticket_id)
  WHERE ticket_id IS NOT NULL;

CREATE TABLE scheduling_link_bookings (
  -- Mirrors the appointment it created (same id) for an auditable
  -- link → booking trail independent of the appointment row.
  link_id uuid NOT NULL REFERENCES scheduling_links(id) ON DELETE CASCADE,
  appointment_id uuid NOT NULL REFERENCES appointments(id) ON DELETE CASCADE,
  booker_name text,
  booker_email text,
  booker_notes text,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (link_id, appointment_id)
);

-- A completed booking links back to its scheduling link (additive,
-- nullable: appointments created elsewhere stay valid).
ALTER TABLE appointments
  ADD COLUMN scheduling_link_id uuid REFERENCES scheduling_links(id) ON DELETE SET NULL;
CREATE INDEX idx_appointments_scheduling_link ON appointments (scheduling_link_id)
  WHERE scheduling_link_id IS NOT NULL;

ALTER TABLE scheduling_groups ENABLE ROW LEVEL SECURITY;
CREATE POLICY scheduling_groups_org ON scheduling_groups
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE scheduling_group_members ENABLE ROW LEVEL SECURITY;
CREATE POLICY scheduling_group_members_org ON scheduling_group_members
  USING (EXISTS (
    SELECT 1 FROM scheduling_groups g
    WHERE g.id = scheduling_group_members.group_id
      AND g.org_id = current_setting('app.org_id', true)::uuid
  ))
  WITH CHECK (EXISTS (
    SELECT 1 FROM scheduling_groups g
    WHERE g.id = scheduling_group_members.group_id
      AND g.org_id = current_setting('app.org_id', true)::uuid
  ));
ALTER TABLE meeting_types ENABLE ROW LEVEL SECURITY;
CREATE POLICY meeting_types_org ON meeting_types
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE scheduling_links ENABLE ROW LEVEL SECURITY;
CREATE POLICY scheduling_links_org ON scheduling_links
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);
-- scheduling_link_bookings has no org_id column; scope through the link.
ALTER TABLE scheduling_link_bookings ENABLE ROW LEVEL SECURITY;
CREATE POLICY scheduling_link_bookings_org ON scheduling_link_bookings
  USING (EXISTS (
    SELECT 1 FROM scheduling_links l
    WHERE l.id = scheduling_link_bookings.link_id
      AND l.org_id = current_setting('app.org_id', true)::uuid
  ))
  WITH CHECK (EXISTS (
    SELECT 1 FROM scheduling_links l
    WHERE l.id = scheduling_link_bookings.link_id
      AND l.org_id = current_setting('app.org_id', true)::uuid
  ));

-- PUBLIC booking seam (0014/0046 pattern): the unauthenticated booking
-- endpoint must resolve a token hash to its org + link + meeting type
-- BEFORE any org scope is bound. Neither function accepts or returns
-- credential material; the caller then binds `app.org_id` and every
-- further query runs RLS-scoped.
CREATE OR REPLACE FUNCTION scheduling_link_resolve(p_token_hash text)
RETURNS TABLE (
  link_id uuid, org_id uuid, meeting_type_id uuid, link_status text,
  expires_at timestamptz, label text, meeting_type_name text,
  meeting_type_description text, duration_minutes integer,
  buffer_minutes integer, booking_window_days integer, parameters jsonb,
  assignee_scope text, assignee_user_id uuid, assignee_group_id uuid,
  auto_email_enabled boolean, auto_email_subject text, auto_email_body text,
  meeting_type_is_active boolean,
  first_viewed_at timestamptz, view_count integer, scheduled_at timestamptz
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
         mt.is_active, l.first_viewed_at, l.view_count, l.scheduled_at
  FROM scheduling_links l
  JOIN meeting_types mt ON mt.id = l.meeting_type_id
  WHERE l.token_hash = $1
$$;

CREATE OR REPLACE FUNCTION scheduling_link_mark_viewed(p_token_hash text)
RETURNS void
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
AS $$
  UPDATE scheduling_links
  SET first_viewed_at = COALESCE(first_viewed_at, now()),
      view_count = view_count + 1
  WHERE token_hash = $1
$$;

-- Single-winner gate: flips ONLY an active link and reports whether it
-- won. A revoked/used/unknown token updates nothing and returns false,
-- so two racing bookings cannot both claim the link.
CREATE OR REPLACE FUNCTION scheduling_link_mark_scheduled(
  p_token_hash text, p_appointment_id uuid
)
RETURNS boolean
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
AS $$
  UPDATE scheduling_links
  SET status = 'used',
      scheduled_at = now(),
      scheduled_appointment_id = p_appointment_id
  WHERE token_hash = $1 AND status = 'active'
  RETURNING true
$$;

REVOKE ALL ON FUNCTION scheduling_link_resolve(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION scheduling_link_mark_viewed(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION scheduling_link_mark_scheduled(text, uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION scheduling_link_resolve(text) TO meridian_app;
GRANT EXECUTE ON FUNCTION scheduling_link_mark_viewed(text) TO meridian_app;
GRANT EXECUTE ON FUNCTION scheduling_link_mark_scheduled(text, uuid) TO meridian_app;
