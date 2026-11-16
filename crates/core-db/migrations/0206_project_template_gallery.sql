-- Migration 0206: the template gallery's own facts, and the lesson loop.
--
-- A template row could describe a plan but not the shape of the job, so the
-- gallery had nothing to sort, filter or compare. The surface pack's S11 cards
-- carry a category, a typical duration, an effort range and an archive state;
-- all four are catalogue facts about the template, not about any one project.
--
--   * category / typical_days / effort_min_minutes / effort_max_minutes —
--     the card's shape strip and its comparison line. They are nullable
--     because an organisation's older templates were written before the
--     catalogue existed, and an absent fact is drawn as absent, never zero.
--
--   * archived_at — archived templates are hidden by default and can still
--     explain old projects. A template is never deleted out from under a
--     project that was created from it; archiving is the honest retire.
--
--   * project_template_lessons gains the decision the editor makes: open,
--     accepted or dismissed, who decided, when, and why a dismissal happened.
--     A dismissal's reason is remembered so the same suggestion is not made
--     twice without new evidence.
--
-- Compat: additive (policy §4.1) — nullable columns with defaults, no
-- destructive statements, no backfill required.

ALTER TABLE project_templates
  ADD COLUMN category text,
  ADD COLUMN typical_days integer CHECK (typical_days IS NULL OR typical_days > 0),
  ADD COLUMN effort_min_minutes integer CHECK (effort_min_minutes IS NULL OR effort_min_minutes >= 0),
  ADD COLUMN effort_max_minutes integer CHECK (effort_max_minutes IS NULL OR effort_max_minutes >= 0),
  ADD COLUMN archived_at timestamptz,
  ADD COLUMN created_by uuid REFERENCES users(id) ON DELETE SET NULL;

-- The card draws a range from these two, so an inverted range is not a range.
ALTER TABLE project_templates
  ADD CONSTRAINT project_templates_effort_range
  CHECK (effort_min_minutes IS NULL OR effort_max_minutes IS NULL
         OR effort_min_minutes <= effort_max_minutes);

ALTER TABLE project_template_lessons
  ADD COLUMN state text NOT NULL DEFAULT 'open'
    CHECK (state IN ('open','accepted','dismissed')),
  ADD COLUMN dismiss_reason text,
  ADD COLUMN decided_by uuid REFERENCES users(id) ON DELETE SET NULL,
  ADD COLUMN decided_at timestamptz;

CREATE INDEX idx_project_templates_category
  ON project_templates (org_id, category)
  WHERE archived_at IS NULL;

CREATE INDEX idx_project_template_lessons_state
  ON project_template_lessons (org_id, template_id, state, created_at DESC);
