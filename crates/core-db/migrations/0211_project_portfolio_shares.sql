-- Migration 0211: shared portfolio views.
--
-- S7.8 asks for a link that reproduces the exact view: scope, range and zoom,
-- for anyone allowed to see it. The link carries its own snapshot, because a
-- shared view that silently changes under the recipient is worse than no link:
-- the coordinator sent a picture of the portfolio, and that picture is what the
-- recipient should see.
--
--   * token is the opaque public handle (a random value, not the row id).
--   * spec is the resolved request (from, to, group, filter, zoom), stored so
--     the view can be rebuilt without parsing a URL.
--   * snapshot is the rendered groups at share time. It is the honest answer
--     for a recipient whose permissions differ: it contains no amounts, and the
--     reader never queries the live plan.
--
-- Compat: additive (policy §4.1) — one new table, no destructive statements.

CREATE TABLE project_portfolio_shares (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  token text NOT NULL UNIQUE,
  spec jsonb NOT NULL,
  snapshot jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  expires_at timestamptz
);
CREATE INDEX idx_project_portfolio_shares_org
  ON project_portfolio_shares (org_id, created_at DESC);

ALTER TABLE project_portfolio_shares ENABLE ROW LEVEL SECURITY;
CREATE POLICY project_portfolio_shares_org ON project_portfolio_shares
  USING (org_id = current_setting('app.org_id', true)::uuid);
