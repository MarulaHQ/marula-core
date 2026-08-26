-- Workflow customization v1 (TASKLIST 2.1): explicit per-board status
-- transitions + defensive seeding of the four semantic category defaults.
--
-- Purpose: give every board an explicit transition adjacency that gates the
-- ticket lifecycle, and guarantee every pre-existing board carries one status
-- per semantic category (open/in_progress/waiting/closed) with a valid entry
-- status — without touching any ticket row that is already consistent.
--
-- Semantics contract: an EMPTY adjacency means the semantic-category floor
-- applies (core-domain `workflow::default_category_allows`: any→any except
-- identity and closed→open). Once rows exist they are authoritative.
--
-- Ticket backfill note: `tickets.status_id` has always been NOT NULL with a
-- RESTRICT foreign key, and `tickets.category` is denormalized inside the
-- same transaction as every status change, so no ticket can reference a
-- missing/invalid status. The UPDATE below is a defensive re-sync only —
-- a no-op on healthy data, never destructive.

-- Backing unique index for the composite foreign keys below.
ALTER TABLE board_statuses
  ADD CONSTRAINT board_statuses_board_id_id_key UNIQUE (board_id, id);

CREATE TABLE board_status_transitions (
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  board_id uuid NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  from_status_id uuid NOT NULL REFERENCES board_statuses(id) ON DELETE CASCADE,
  to_status_id uuid NOT NULL REFERENCES board_statuses(id) ON DELETE CASCADE,
  PRIMARY KEY (from_status_id, to_status_id),
  CONSTRAINT board_transitions_distinct CHECK (from_status_id <> to_status_id),
  -- Both endpoints must belong to the SAME board: composite FKs make a
  -- cross-board edge impossible even if application validation regresses.
  CONSTRAINT board_transitions_from_same_board FOREIGN KEY (board_id, from_status_id)
    REFERENCES board_statuses (board_id, id) ON DELETE CASCADE,
  CONSTRAINT board_transitions_to_same_board FOREIGN KEY (board_id, to_status_id)
    REFERENCES board_statuses (board_id, id) ON DELETE CASCADE
);
CREATE INDEX idx_board_transitions_board ON board_status_transitions (board_id);

ALTER TABLE board_status_transitions ENABLE ROW LEVEL SECURITY;
CREATE POLICY board_status_transitions_org ON board_status_transitions
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- Seed any missing semantic-category default onto boards that lack one
-- (defaults-first doctrine: customization adds depth, never removes the
-- baseline). Name collisions with an unrelated same-named custom status are
-- suffixed rather than skipped, so the CATEGORY coverage is guaranteed.
INSERT INTO board_statuses (id, board_id, org_id, name, category, sort_order, is_entry)
SELECT gen_random_uuid(),
       b.id,
       b.org_id,
       d.name || CASE WHEN EXISTS (
         SELECT 1 FROM board_statuses x
         WHERE x.board_id = b.id AND x.name = d.name AND x.category <> d.category
       ) THEN ' (default)' ELSE '' END,
       d.category,
       d.sort_order,
       false
FROM boards b
CROSS JOIN (VALUES
  ('New'::text, 'open'::text, 0),
  ('In Progress', 'in_progress', 10),
  ('Waiting', 'waiting', 20),
  ('Resolved', 'closed', 30)
) AS d(name, category, sort_order)
WHERE NOT EXISTS (
  SELECT 1 FROM board_statuses s
  WHERE s.board_id = b.id AND s.category = d.category
);

-- Entry-status backfill: a board without any entry status cannot create
-- tickets. Promote its lowest-sorted open-category status (any status if no
-- open one exists); never demote an existing entry.
UPDATE board_statuses s
SET is_entry = true
WHERE s.is_entry = false
  AND NOT EXISTS (
    SELECT 1 FROM board_statuses e WHERE e.board_id = s.board_id AND e.is_entry
  )
  AND s.id = (
    SELECT c.id FROM board_statuses c
    WHERE c.board_id = s.board_id
    ORDER BY (c.category = 'open') DESC, c.sort_order, c.id
    LIMIT 1
  );

-- Defensive ticket re-sync: realign the denormalized category/resolved_at
-- with each ticket's actual status category (no-op on healthy data).
UPDATE tickets t
SET category = s.category,
    resolved_at = CASE
      WHEN s.category = 'closed' AND t.resolved_at IS NULL THEN now()
      ELSE t.resolved_at
    END
FROM board_statuses s
WHERE t.status_id = s.id
  AND (t.category <> s.category
       OR (s.category = 'closed' AND t.resolved_at IS NULL));
