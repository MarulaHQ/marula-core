-- Wave 41 lane 8 (`p41-ui-money`) — the approval trail gains a `fixed`
-- action, so an approver's one-click P3 fix (adopt the server's suggested
-- description, or trim the overlap off a double-claimed entry) is recorded
-- on the same append-only trail as the decisions it sits beside.
--
-- Additive on purpose: the column's vocabulary is a CHECK, so it is widened
-- by dropping and re-adding the constraint rather than editing 0073, which
-- has already run in every environment. Existing rows are untouched, and the
-- new value is only ever written together with a `note` describing the
-- change, so the trail can always answer "what did they change, and why".
ALTER TABLE time_entry_approvals
  DROP CONSTRAINT IF EXISTS time_entry_approvals_action_check;
ALTER TABLE time_entry_approvals
  ADD CONSTRAINT time_entry_approvals_action_check
  CHECK (action IN ('submitted', 'approved', 'rejected', 'reopened', 'fixed'));
