-- Board organization + board-level RBAC (TASKLIST 2.11, workflow
-- customization v1 trio).
--
-- Purpose: (1) per-org board groups so the left nav can render a board
-- tree; (2) explicit per-user/per-board membership rows with a three-tier
-- `board_role` (viewer | member | admin) that core-api checks BEFORE the
-- org-level permission matrix — a board denial always wins (§6); (3)
-- defaults-first backfill: every holder of an org-level admin/owner role
-- becomes a board admin on each existing board, so nothing regresses for
-- orgs that never touch per-board RBAC.
--
-- Non-purpose: no change to ticket/comment data; no org-role changes;
-- favorites already exist since 0003 and are left untouched.
--
-- RLS: every new user-data table carries an org-scoped policy — the CI
-- gate (`assert_rls_coverage`) fails otherwise.

CREATE TYPE board_role AS ENUM ('viewer', 'member', 'admin');

CREATE TABLE board_groups (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- The group's controller (creator). Group management is a board-admin /
  -- ManageBoards concern; the column keeps provenance and lets owners
  -- rename their own groups without broader grants.
  owner_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name text NOT NULL,
  position integer NOT NULL DEFAULT 0,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, name)
);
CREATE INDEX idx_board_groups_org ON board_groups (org_id, position);

ALTER TABLE board_groups ENABLE ROW LEVEL SECURITY;
CREATE POLICY board_groups_org ON board_groups
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- A board sits in at most one group. Additive nullable column on the
-- shared boards table; deleting a group leaves boards in place (ungrouped).
ALTER TABLE boards
  ADD COLUMN board_group_id uuid REFERENCES board_groups(id) ON DELETE SET NULL;
CREATE INDEX idx_boards_group ON boards (board_group_id);

CREATE TABLE board_members (
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  board_id uuid NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role board_role NOT NULL DEFAULT 'member',
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, board_id)
);
CREATE INDEX idx_board_members_board ON board_members (board_id);

ALTER TABLE board_members ENABLE ROW LEVEL SECURITY;
CREATE POLICY board_members_org ON board_members
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- Defaults-first backfill: org-level Owner/Admin holders become board
-- admins on every existing board of their org. Idempotent shape (no
-- conflicts possible on the fresh table, but ON CONFLICT keeps the
-- statement safe under re-entry).
INSERT INTO board_members (org_id, board_id, user_id, role)
SELECT b.org_id, b.id, ur.user_id, 'admin'::board_role
FROM boards b
JOIN roles r ON r.org_id = b.org_id AND r.builtin IN ('owner', 'admin')
JOIN user_roles ur ON ur.role_id = r.id
ON CONFLICT (user_id, board_id) DO NOTHING;
