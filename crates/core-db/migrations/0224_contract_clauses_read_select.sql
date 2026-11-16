-- Limit the contract clause read policy to SELECT.
--
-- 0200 created the read policy without a command, so permissive policies
-- combined with OR and it also passed UPDATE and DELETE on shipped wording.
-- The base migration is already applied on deployed databases and must not be
-- edited, so the narrowed policy lands here. Shipped rows (org_id IS NULL) and
-- the caller's own rows stay readable; writes stay on the org-scoped policy.
--
-- A policy replacement removes no data. The destructive-statement gate should
-- read it as such: DROP POLICY discards a rule, not rows.
DROP POLICY IF EXISTS contract_clauses_read ON contract_clauses;
CREATE POLICY contract_clauses_read ON contract_clauses
  FOR SELECT
  USING (org_id IS NULL OR org_id = current_setting('app.org_id', true)::uuid);
