-- Client portal v1 (TASKLIST 2.13): scoped portal roles, per-board exposure,
-- and a dedicated portal session family.
--
-- Purpose: give client personnel their own identity plane — `portal_users`
-- are NOT staff rows in `users` — bound to exactly one client, with a
-- three-tier role model (contact | approver | it_manager), a board×client
-- exposure flag that gates which tickets are even listable, and
-- `portal_sessions` as a SEPARATE token family from staff `auth_sessions`.
--
-- Visibility doctrine (v1, binding): portal surfaces read ticket comments
-- through the notes-taxonomy whitelist (`core_domain::notes::visible_kinds`).
-- contact/approver see ONLY `client_visible`; the it_manager tier ADDITIONALLY
-- reads `in_house_update`. NO portal role can ever read `internal` notes —
-- that rule is enforced in core-domain, not here.
--
-- Status-exposure doctrine: `board_portal_exposure.visible_status_ids` is an
-- empty array by default, which means "ALL statuses are client-visible"
-- (v1 semantics). A non-empty array narrows visibility to those status ids.
-- Customization adds depth; it never changes the shipped default.
--
-- Provenance doctrine: portal actors audit under their own actor_kind
-- (portal_contact / portal_approver / portal_it_manager) so the append-only
-- audit trail distinguishes client-side actions from staff actions at the
-- storage layer. The 0001 CHECK constraint is widened additively — existing
-- kinds keep their meaning, no row is touched.
--
-- Non-purpose: no portal-facing tables beyond v1 scope (invoices, KB,
-- quotes are Stage 3); no email-invite infrastructure (admin sets the
-- initial password out-of-band; first login flips invited → active);
-- no changes to staff auth tables or JWT handling.

CREATE TABLE portal_users (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- Every portal user belongs to EXACTLY one client; all portal reads are
  -- scoped by client_id IN ADDITION to the org RLS scope.
  client_id uuid NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
  email text NOT NULL,
  display_name text NOT NULL,
  password_hash text NOT NULL,
  role text NOT NULL DEFAULT 'contact'
    CHECK (role IN ('contact', 'approver', 'it_manager')),
  status text NOT NULL DEFAULT 'invited'
    CHECK (status IN ('invited', 'active', 'disabled')),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
-- Email is unique PER ORG, CASE-INSENSITIVELY (logins normalize case, so
-- provisioning must too): a functional unique index covers both the
-- uniqueness contract and the login lookup path.
CREATE UNIQUE INDEX uq_portal_users_org_email ON portal_users (org_id, lower(email));
CREATE INDEX idx_portal_users_client ON portal_users (org_id, client_id);

-- Per-board client exposure. One row per (board, client): `exposed` gates
-- whether the client's portal users see ANY tickets from that board;
-- `visible_status_ids` narrows which statuses are visible ('{}' = all).
CREATE TABLE board_portal_exposure (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  board_id uuid NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  client_id uuid NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
  exposed boolean NOT NULL DEFAULT false,
  visible_status_ids uuid[] NOT NULL DEFAULT '{}',
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (board_id, client_id)
);
CREATE INDEX idx_board_exposure_client ON board_portal_exposure (org_id, client_id);

-- Portal refresh sessions — a token family fully disjoint from staff
-- `auth_sessions`: different table, different resolver, different JWT kind.
CREATE TABLE portal_sessions (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  portal_user_id uuid NOT NULL REFERENCES portal_users(id) ON DELETE CASCADE,
  refresh_token_hash text NOT NULL,
  user_agent text,
  expires_at timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  revoked_at timestamptz
);
CREATE INDEX idx_portal_sessions_user ON portal_sessions (portal_user_id);

-- --- Row level security ----------------------------------------------------
ALTER TABLE portal_users ENABLE ROW LEVEL SECURITY;
CREATE POLICY portal_users_org ON portal_users
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE board_portal_exposure ENABLE ROW LEVEL SECURITY;
CREATE POLICY board_portal_exposure_org ON board_portal_exposure
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE portal_sessions ENABLE ROW LEVEL SECURITY;
CREATE POLICY portal_sessions_org ON portal_sessions
  USING (EXISTS (
    SELECT 1 FROM portal_users pu
    WHERE pu.id = portal_sessions.portal_user_id
      AND pu.org_id = current_setting('app.org_id', true)::uuid
  ));

-- --- Audit provenance: widen the actor-kind vocabulary ----------------------
-- Additive only: the five original kinds are unchanged; three portal kinds
-- join them so append-only provenance records WHO KIND of actor acted.
ALTER TABLE audit_events DROP CONSTRAINT audit_events_actor_kind_check;
ALTER TABLE audit_events ADD CONSTRAINT audit_events_actor_kind_check
  CHECK (actor_kind IN ('human','agent','plugin','connector','system',
                        'portal_contact','portal_approver','portal_it_manager'));

-- --- Login resolution bridge -------------------------------------------------
-- Portal login happens before any org is known, but portal_users is
-- RLS-scoped. Same narrow SECURITY DEFINER pattern as app_resolve_login
-- (0010): resolves identity only — never hashes — and the caller binds
-- app.org_id immediately afterwards so every later query runs under RLS.

CREATE OR REPLACE FUNCTION portal_resolve_login(p_email text)
RETURNS TABLE (portal_user_id uuid, org_id uuid, client_id uuid)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
STABLE
AS $$
  SELECT pu.id, pu.org_id, pu.client_id
  FROM portal_users pu
  WHERE lower(pu.email) = lower(p_email)
    AND pu.status IN ('active', 'invited')
  LIMIT 1
$$;

-- Refresh rotation resolves (portal_user_id, org_id, client_id) straight
-- from a live, unrevoked, unexpired session row — same narrow bridge shape
-- as app_resolve_session (0010). Never returns token material.
CREATE OR REPLACE FUNCTION portal_resolve_session(
  p_session_id uuid,
  p_token_hash text
)
RETURNS TABLE (portal_user_id uuid, org_id uuid, client_id uuid)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
STABLE
AS $$
  SELECT s.portal_user_id, s.org_id, pu.client_id
  FROM portal_sessions s
  JOIN portal_users pu ON pu.id = s.portal_user_id
  WHERE s.id = p_session_id
    AND s.refresh_token_hash = p_token_hash
    AND s.revoked_at IS NULL
    AND s.expires_at > now()
    AND pu.status = 'active'
  LIMIT 1
$$;

REVOKE ALL ON FUNCTION portal_resolve_login(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION portal_resolve_session(uuid, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION portal_resolve_login(text) TO meridian_app;
GRANT EXECUTE ON FUNCTION portal_resolve_session(uuid, text) TO meridian_app;
