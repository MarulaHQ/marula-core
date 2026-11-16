-- Migration 0200: the approved contract clause library.
--
-- The quoting design pack draws contract drafting as "choose and fill clauses
-- from the approved library" and states the rule twice, once in the notes and
-- once in the state table: no legal wording is written by a model. A draft may
-- only name a clause and a parameter value. That needs somewhere for the
-- clauses to live.
--
-- Shape:
--   * `org_id IS NULL` rows are the shipped library every org reads. The seed
--     below inserts them, and an org that edits wording adds its own row at a
--     higher version rather than mutating the shipped one.
--   * `parameter_label` / `standard_value` carry the one thing a draft may
--     change (a notice period, a response time). A selection whose value
--     differs from `standard_value` is a variant and needs legal review; the
--     comparison is data, not a rule in code.
--   * `status` on the row is the clause's own standing in the library
--     (standard or variant); a draft's status is computed per selection.
--
-- Compat: additive (policy §4.1) — one new table plus seeds. Nothing existing
-- is rewritten or dropped.

CREATE TABLE contract_clauses (
  id uuid PRIMARY KEY,
  -- NULL = the shipped library; an org row overrides it at a higher version.
  org_id uuid REFERENCES orgs(id) ON DELETE CASCADE,
  key text NOT NULL,
  title text NOT NULL,
  body text NOT NULL,
  version integer NOT NULL DEFAULT 1 CHECK (version > 0),
  status text NOT NULL DEFAULT 'standard' CHECK (status IN ('standard', 'variant')),
  review_required boolean NOT NULL DEFAULT false,
  -- The one value a describing draft may fill, and the standard it is
  -- compared against. Both NULL for a clause with nothing to fill.
  parameter_label text,
  standard_value text,
  active boolean NOT NULL DEFAULT true,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

-- Two partial uniques, because `UNIQUE (org_id, key, version)` does not
-- constrain the NULL-org rows (NULLs are distinct in a unique index).
CREATE UNIQUE INDEX uq_contract_clauses_shipped
  ON contract_clauses (key, version)
  WHERE org_id IS NULL;
CREATE UNIQUE INDEX uq_contract_clauses_org
  ON contract_clauses (org_id, key, version)
  WHERE org_id IS NOT NULL;
CREATE INDEX idx_contract_clauses_org ON contract_clauses (org_id, key);

ALTER TABLE contract_clauses ENABLE ROW LEVEL SECURITY;
-- Read-only past the shipped rows and the caller's own: without FOR SELECT the
-- policy would also pass UPDATE and DELETE on shipped wording, because
-- permissive policies are combined with OR.
CREATE POLICY contract_clauses_read ON contract_clauses
  FOR SELECT
  USING (org_id IS NULL OR org_id = current_setting('app.org_id', true)::uuid);
-- Writes stay inside the caller's own org; the shipped rows are not writable
-- by an org at all (they are seeded by this migration, which runs privileged).
CREATE POLICY contract_clauses_write ON contract_clauses
  FOR ALL
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

-- The shipped library. Seven clauses, in the order a contract reads.
INSERT INTO contract_clauses
  (id, org_id, key, title, body, version, status, review_required,
   parameter_label, standard_value)
VALUES
  ('0c100000-0000-0000-0000-000000000001', NULL, 'scope', 'Services and scope',
   'The supplier provides the services described in the schedule attached to this agreement, during the agreed service hours, using suitably qualified staff.',
   1, 'standard', false, NULL, NULL),
  ('0c100000-0000-0000-0000-000000000002', NULL, 'response', 'Response times',
   'The supplier acknowledges a Priority 1 incident within the response time in the schedule and works continuously until service is restored. Lower priorities are acknowledged within the times in the schedule.',
   1, 'standard', false, 'Priority 1 response', '2 hours'),
  ('0c100000-0000-0000-0000-000000000003', NULL, 'exclusions', 'What is not included',
   'Work outside the services described in the schedule, including new hardware, projects and work at sites not listed, is quoted separately and needs the client''s written approval.',
   1, 'standard', false, NULL, NULL),
  ('0c100000-0000-0000-0000-000000000004', NULL, 'payment', 'Fees and payment',
   'Fees are invoiced in advance for each billing period and are payable within the payment terms in the schedule. Overdue amounts may be charged interest at the statutory rate.',
   1, 'standard', false, 'Payment terms', 'Net 30'),
  ('0c100000-0000-0000-0000-000000000005', NULL, 'term', 'Term and renewal',
   'This agreement starts on the start date in the schedule and continues for the term stated there. It renews for the same term unless either party gives notice under the notice clause.',
   1, 'standard', false, 'Term', '12 months'),
  ('0c100000-0000-0000-0000-000000000006', NULL, 'renewal_notice', 'Notice to cancel or change',
   'Either party may end this agreement at the end of a term by giving written notice before the end of the notice period in the schedule.',
   1, 'standard', false, 'Notice period', '90 days'),
  ('0c100000-0000-0000-0000-000000000007', NULL, 'data_handling', 'Data handling and access',
   'The supplier accesses client systems only to provide the services, keeps credentials in its vault, and returns or destroys client data on request at the end of the agreement.',
   1, 'standard', false, NULL, NULL)
ON CONFLICT DO NOTHING;
