-- Ticket-less automation subjects (TASKLIST Wave 43 lane 3,
-- `p43-ticketless-automation-triggers`).
--
-- Before this lane every automation rule's subject was implicitly a TICKET:
-- the worker's `match_rules` bailed unless the envelope resolved one, so a
-- ticket-less envelope matched zero rules by construction — including every
-- envelope lane 1's generic inbound receiver emits.
--
-- `automation_rules` now declares its subject explicitly:
--   NULL      -- the row predates this migration. The subject is a ticket,
--                which is the historical behaviour, unchanged.
--   'ticket'  -- the same, said out loud.
--   'board'   -- the subject is the BOARD. The rule evaluates against a
--                ticket-less scope and fires on an envelope that carries
--                only a `board_id` (lane 1's receiver carries it on every
--                emitted payload).
--
-- The column is nullable and deliberately NOT backfilled: the historical
-- value IS the default read path, so one instant ALTER keeps this a
-- single-table, no-rewrite migration on a hot table.
--
-- No new table, so no new RLS policy; `automation_rules` keeps its
-- org-isolating policy from 0015 (assert_rls_coverage stays green).
ALTER TABLE automation_rules
    ADD COLUMN subject_kind text;

ALTER TABLE automation_rules
    ADD CONSTRAINT automation_rules_subject_kind_check
    CHECK (subject_kind IS NULL OR subject_kind IN ('ticket', 'board'));

COMMENT ON COLUMN automation_rules.subject_kind IS
    'NULL or ''ticket'' = the rule''s subject is a ticket; ''board'' = the rule fires on ticket-less envelopes scoped to a board';
