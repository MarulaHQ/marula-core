-- Structured notes taxonomy + enforced pre-close checklists (TASKLIST 2.8).
--
-- Purpose: give per-board pre-close checklist templates a home, link each
-- ticket checklist instance back to its template item, and carry the
-- per-board enforcement toggle — so close-time gating can list exactly the
-- incomplete REQUIRED items.
--
-- Defaults-first doctrine (documented contract): closing is blocked ONLY
-- when BOTH are true — (a) the board has a checklist template and (b) the
-- board's `boards.require_checklist_close` toggle is enabled. The shipped
-- default is enforcement OFF (`false`): zero-config boards keep closing
-- exactly as before; enabling is an explicit board-admin act.
--
-- Note-kind column: `ticket_comments.kind` (CHECK internal |
-- client_visible | in_house_update, DEFAULT 'internal') has existed since
-- migration 0004, so every pre-existing row is already 'internal' by
-- construction. The UPDATE below is a defensive re-sync of any NULL kind
-- (impossible under the NOT NULL constraint, hence a guaranteed no-op) and
-- is idempotent by its WHERE clause — re-running changes nothing.
--
-- Instance progress doctrine: ticket checklist progress is DERIVED at read
-- time (COUNT over checklist_items), never materialized. Checklists are
-- small (<100 rows/ticket) and single-index-query cheap; materialized
-- counters would need transactional maintenance on every toggle for no
-- measurable gain and risk drift under concurrent edits.
--
-- Non-purpose: no workflow/transition changes (0011 owns those), no
-- comment content changes, no portal tables.

-- Per-board pre-close enforcement toggle (see defaults-first note above).
ALTER TABLE boards ADD COLUMN require_checklist_close boolean NOT NULL DEFAULT false;

CREATE TABLE board_checklist_templates (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  board_id uuid NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  name text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  -- Exactly ONE active pre-close checklist per board keeps close-time
  -- enforcement deterministic: there is never ambiguity about which
  -- template governs a closure.
  UNIQUE (board_id)
);

CREATE TABLE checklist_template_items (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  template_id uuid NOT NULL REFERENCES board_checklist_templates(id) ON DELETE CASCADE,
  text text NOT NULL,
  sort_order integer NOT NULL DEFAULT 0
);
CREATE INDEX idx_template_items_template ON checklist_template_items (template_id, sort_order);

-- Provenance link: instances created from a template item remember it, so
-- close-time enforcement counts "required" instances by linkage rather
-- than fragile text matching. Removing a template item demotes its
-- instances to ad-hoc items (SET NULL) — history is preserved, nothing
-- blocks that is no longer required.
ALTER TABLE checklist_items ADD COLUMN template_item_id uuid
  REFERENCES checklist_template_items(id) ON DELETE SET NULL;
CREATE INDEX idx_checklist_template_link ON checklist_items (template_item_id);

-- Defensive taxonomy backfill: force any NULL kind (none can exist today)
-- to the 'internal' default. WHERE-clause scoped ⇒ idempotent, non-flagged
-- by the destructive-migration gate.
UPDATE ticket_comments SET kind = 'internal' WHERE kind IS NULL;

ALTER TABLE board_checklist_templates ENABLE ROW LEVEL SECURITY;
CREATE POLICY board_checklist_templates_org ON board_checklist_templates
  USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE checklist_template_items ENABLE ROW LEVEL SECURITY;
CREATE POLICY checklist_template_items_org ON checklist_template_items
  USING (org_id = current_setting('app.org_id', true)::uuid);
