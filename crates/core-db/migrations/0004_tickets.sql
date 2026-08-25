-- Tickets, structured notes, pre-close checklists.
-- `category` is denormalized from board_statuses.category for cheap
-- filtering/sorting; it is updated transactionally on every status change.

CREATE TABLE tickets (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  number text NOT NULL,
  board_id uuid NOT NULL REFERENCES boards(id) ON DELETE RESTRICT,
  status_id uuid NOT NULL REFERENCES board_statuses(id) ON DELETE RESTRICT,
  title text NOT NULL,
  body text,
  priority text NOT NULL DEFAULT 'p3' CHECK (priority IN ('p1','p2','p3','p4')),
  client_id uuid REFERENCES clients(id) ON DELETE SET NULL,
  site_id uuid REFERENCES sites(id) ON DELETE SET NULL,
  contact_id uuid REFERENCES contacts(id) ON DELETE SET NULL,
  assignee_id uuid REFERENCES users(id) ON DELETE SET NULL,
  requester_id uuid REFERENCES users(id) ON DELETE SET NULL,
  category text NOT NULL CHECK (category IN ('open','in_progress','waiting','closed')),
  due_at timestamptz,
  resolved_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, number)
);
CREATE INDEX idx_tickets_board ON tickets (board_id, category, updated_at DESC);
CREATE INDEX idx_tickets_assignee ON tickets (org_id, assignee_id) WHERE assignee_id IS NOT NULL;
CREATE INDEX idx_tickets_client ON tickets (client_id);
CREATE INDEX idx_tickets_title_fts ON tickets USING gin (to_tsvector('english', title || ' ' || coalesce(body, '')));

CREATE TABLE ticket_comments (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  ticket_id uuid NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
  author_id uuid REFERENCES users(id) ON DELETE SET NULL,
  kind text NOT NULL DEFAULT 'internal' CHECK (kind IN ('internal','client_visible','in_house_update')),
  body text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz
);
CREATE INDEX idx_comments_ticket ON ticket_comments (ticket_id, created_at);

CREATE TABLE checklist_items (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  ticket_id uuid NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
  text text NOT NULL,
  checked boolean NOT NULL DEFAULT false,
  sort_order integer NOT NULL DEFAULT 0,
  checked_by uuid REFERENCES users(id) ON DELETE SET NULL,
  checked_at timestamptz
);
CREATE INDEX idx_checklist_ticket ON checklist_items (ticket_id, sort_order);

ALTER TABLE tickets ENABLE ROW LEVEL SECURITY;
CREATE POLICY tickets_org ON tickets USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE ticket_comments ENABLE ROW LEVEL SECURITY;
CREATE POLICY ticket_comments_org ON ticket_comments USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE checklist_items ENABLE ROW LEVEL SECURITY;
CREATE POLICY checklist_items_org ON checklist_items USING (org_id = current_setting('app.org_id', true)::uuid);
