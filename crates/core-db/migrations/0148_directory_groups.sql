-- Directory groups (Wave 52 lane R10, `p52-directory-groups`).
--
-- Why this exists: a directory (Entra ID, Google Workspace, Okta) synchronizes
-- people and their group memberships, and the groups carry the authority. Core
-- had `user_roles` and nothing to put a synchronized membership into, so an
-- identity sync had to flatten every group into per-user role rows: it could
-- not express "these twelve people are technicians because they are in the
-- Tier 2 group", could not revoke the group's grants in one move, and could not
-- recognize a group it had already written. This migration adds the three
-- tables that answer those questions, and nothing that speaks the sync
-- protocol itself.
--
-- Shape:
--   1. `directory_groups` — one row per group in one org. `external_id` is the
--      identity-provider identifier the row is matched on, unique per org when
--      present, so a second sync run updates rather than duplicates. A locally
--      created group leaves it null.
--   2. `directory_group_members` — group → user. Cascade on either side: a
--      deleted group or a deleted user leaves no orphan membership.
--   3. `directory_group_roles` — group → role. Cascade on either side.
--
-- Effective authorization is the ADDITIVE UNION of `user_roles` and the roles
-- reachable through membership (`core_db::directory_groups::effective_roles_for_user`).
-- A group never subtracts: there is no deny grant, because a deny would need a
-- precedence rule between two grant sources and the union is what a directory
-- administrator already expects. Removing a membership therefore revokes
-- exactly the roles that membership was the only path to.
--
-- Non-purpose: this is storage and mapping. No sync protocol, no token, no
-- reconciliation loop, no tier knowledge — a caller that speaks a directory
-- protocol reads and writes these tables through the same API every other
-- client uses.

-- ---------------------------------------------------------------------------
-- 1. The groups
-- ---------------------------------------------------------------------------

CREATE TABLE directory_groups (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- Display name. Unique per org, the same rule `roles` uses, so a listing
  -- never shows two rows a human cannot tell apart.
  name text NOT NULL,
  description text,
  -- The identifier the directory knows this group by. Nullable because a group
  -- created in Marula has no external counterpart; unique when present so one
  -- external group cannot map to two local rows.
  external_id text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, name)
);

-- The lookup an identity sync performs on every pass: "do I already have this
-- external group?" Partial, because the locally-created rows carry null and
-- must not collide with each other.
CREATE UNIQUE INDEX idx_directory_groups_external
  ON directory_groups (org_id, external_id)
  WHERE external_id IS NOT NULL;

-- ---------------------------------------------------------------------------
-- 2. Membership
-- ---------------------------------------------------------------------------

CREATE TABLE directory_group_members (
  group_id uuid NOT NULL REFERENCES directory_groups(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  added_at timestamptz NOT NULL DEFAULT now(),
  -- Who put the person in the group: a staff user id, or null for a write that
  -- came through a sync. The audit row carries the same fact; this column
  -- answers "why is this person here" without a join to the audit trail.
  added_by uuid,
  PRIMARY KEY (group_id, user_id)
);

-- The direction authorization reads: every group one user belongs to.
CREATE INDEX idx_directory_group_members_user
  ON directory_group_members (user_id, group_id);

-- ---------------------------------------------------------------------------
-- 3. Role mapping
-- ---------------------------------------------------------------------------

CREATE TABLE directory_group_roles (
  group_id uuid NOT NULL REFERENCES directory_groups(id) ON DELETE CASCADE,
  role_id uuid NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
  granted_at timestamptz NOT NULL DEFAULT now(),
  granted_by uuid,
  PRIMARY KEY (group_id, role_id)
);

-- The reverse direction: "who grants this role", which a roles listing wants
-- when it explains a holder count.
CREATE INDEX idx_directory_group_roles_role
  ON directory_group_roles (role_id, group_id);

-- ---------------------------------------------------------------------------
-- 4. Row-level security
-- ---------------------------------------------------------------------------
--
-- `directory_groups` carries `org_id`, so its policy is the plain scope check
-- every tenant table uses. The two child tables have no `org_id` of their own:
-- they inherit their scope from the group, exactly as `user_roles` inherits it
-- from `users`. Denormalizing an org id into them would create a second source
-- of truth that a bug could set wrong, and the EXISTS is over a primary-key
-- lookup either way.

ALTER TABLE directory_groups ENABLE ROW LEVEL SECURITY;
CREATE POLICY directory_groups_org ON directory_groups
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE directory_group_members ENABLE ROW LEVEL SECURITY;
CREATE POLICY directory_group_members_org ON directory_group_members
  USING (EXISTS (
    SELECT 1 FROM directory_groups g
    WHERE g.id = directory_group_members.group_id
      AND g.org_id = current_setting('app.org_id', true)::uuid
  ));

ALTER TABLE directory_group_roles ENABLE ROW LEVEL SECURITY;
CREATE POLICY directory_group_roles_org ON directory_group_roles
  USING (EXISTS (
    SELECT 1 FROM directory_groups g
    WHERE g.id = directory_group_roles.group_id
      AND g.org_id = current_setting('app.org_id', true)::uuid
  ));

-- ---------------------------------------------------------------------------
-- 5. Privileges for the application role
-- ---------------------------------------------------------------------------
--
-- Explicit rather than riding only the schema's ALTER DEFAULT PRIVILEGES: the
-- default is cluster-wide state set by an earlier migration, so a database that
-- restored a dump or ran the migrations under a different owner would carry the
-- tables without the grants and fail every read. RLS still constrains the rows.

GRANT SELECT, INSERT, UPDATE, DELETE ON directory_groups TO marula_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON directory_group_members TO marula_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON directory_group_roles TO marula_app;
