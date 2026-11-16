-- An inverted effort range is not a range. 0206 created the two columns and is
-- already applied; the constraint lands here so deployed databases get it.
ALTER TABLE project_templates
  ADD CONSTRAINT project_templates_effort_range
  CHECK (effort_min_minutes IS NULL OR effort_max_minutes IS NULL
         OR effort_min_minutes <= effort_max_minutes);
