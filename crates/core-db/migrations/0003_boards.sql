-- Boards and per-board custom statuses bound to semantic categories (§5).

CREATE TABLE boards (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  description text,
  is_default boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, name)
);

CREATE TABLE board_statuses (
  id uuid PRIMARY KEY,
  board_id uuid NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  category text NOT NULL CHECK (category IN ('open','in_progress','waiting','closed')),
  sort_order integer NOT NULL DEFAULT 0,
  is_entry boolean NOT NULL DEFAULT false,
  color text,
  UNIQUE (board_id, name)
);
CREATE INDEX idx_board_statuses_board ON board_statuses (board_id, sort_order);

CREATE TABLE board_favorites (
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  board_id uuid NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  sort_order integer NOT NULL DEFAULT 0,
  PRIMARY KEY (user_id, board_id)
);

ALTER TABLE boards ENABLE ROW LEVEL SECURITY;
CREATE POLICY boards_org ON boards USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE board_statuses ENABLE ROW LEVEL SECURITY;
CREATE POLICY board_statuses_org ON board_statuses USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE board_favorites ENABLE ROW LEVEL SECURITY;
CREATE POLICY board_favorites_org ON board_favorites
  USING (EXISTS (SELECT 1 FROM users u WHERE u.id = board_favorites.user_id
                 AND u.org_id = current_setting('app.org_id', true)::uuid));

-- Default board + statuses are seeded per-org at org creation by core-api.
