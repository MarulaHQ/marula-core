-- Class-to-account mapping for the accountant package.
--
-- A reporting class (freight_in, service_cost, inventory, sales, rental_income
-- …) is the vocabulary every money row already carries. The accountant package
-- turns each class into the account code of the org's books, once per class,
-- so a package parses without a mapping meeting. An org that has not chosen
-- an account falls back to the built-in default the code applies.
--
-- Additive: one table, org-scoped, RLS on.

CREATE TABLE accounting_class_accounts (
  id             uuid PRIMARY KEY,
  org_id         uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  reporting_class text NOT NULL,
  account_code   text NOT NULL,
  account_name   text NOT NULL,
  created_at     timestamptz NOT NULL DEFAULT now(),
  updated_at     timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT accounting_class_accounts_code
    CHECK (account_code <> '' AND account_name <> '')
);

CREATE UNIQUE INDEX uq_accounting_class_accounts_class
  ON accounting_class_accounts (org_id, reporting_class);

ALTER TABLE accounting_class_accounts ENABLE ROW LEVEL SECURITY;
CREATE POLICY accounting_class_accounts_org ON accounting_class_accounts
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);
