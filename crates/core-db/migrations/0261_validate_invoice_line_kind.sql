-- Validate the widened invoice-line kind check in its own transaction.
--
-- 0260 adds the constraint NOT VALID, because the table it locks is large and
-- the migrator runs each migration file in one transaction. Validating in a
-- separate file means the scan runs after 0260 commits, and the ACCESS
-- EXCLUSIVE lock it takes is held only for the scan rather than for the whole
-- of 0260's work.
ALTER TABLE invoice_lines VALIDATE CONSTRAINT invoice_lines_kind_check;
