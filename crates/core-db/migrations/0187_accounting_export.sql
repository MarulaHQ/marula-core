-- 0187 — the accounting export ledger: runs, receipts, tax mappings.
--
-- Three tables, because an export has three facts that fail independently:
--
--   * `accounting_export_runs` — "this Marula document is being sent to that
--     connection, and here is where it stands". It is the row a person reads
--     when an invoice did not reach QuickBooks: state, typed reason, attempts,
--     and when the last attempt claimed the row.
--
--   * `accounting_export_receipts` — "the vendor created THIS record for it".
--     The vendor id is the whole point: a payment, a credit note and a later
--     reconcile all need it, and it must never be guessed from a document
--     number a vendor does not keep unique.
--
--   * `accounting_tax_mappings` — "this Marula tax code is that vendor tax
--     code, on this connection". Per connection, not per org: two clients'
--     books name their taxes differently, and a mapping that spanned them
--     would be the guess claims-gap G3 refuses.
--
-- Why `held` is a state and not a failure. A run whose tax code has no mapping
-- has not failed — nothing was sent, and a human has a decision to make. It
-- carries the same `reason` column as `failed` so the report renders both from
-- one shape, and `state` is what keeps them apart in the filter. Collapsing
-- them would tell an operator to retry work that is waiting on them.
--
-- Why the run is unique per document and connection. A retry is the same run
-- moving forward, not a second row: the claim, the reconcile and the receipt
-- all read "the run for this document on this connection", and a second row
-- would let two retries push the same invoice twice.
--
-- Idempotency ordering (the crash window). The run is claimed (`in_flight`,
-- receipt still absent) and COMMITTED before the vendor call, because a claim
-- that rolls back with the request is no claim at all. The receipt is written
-- in the same transaction that moves the run to `succeeded`, so a receipt
-- never exists without its terminal state and a state never claims success
-- without the id. A process that dies between the two leaves `in_flight` with
-- no receipt, and the retry reconciles against the vendor by the marker the
-- adapter stamps into the document (`[marula:<kind>:<uuid>]`) before it sends
-- anything.
--
-- RLS doctrine: org-scoped user data, org-bound reads. `assert_rls_coverage`
-- requires the column and the policy together, and every table below carries
-- both. Client scoping is enforced in the query layer, not by RLS, for the
-- same reason migration 0185 states: a policy that also encoded client access
-- would duplicate `core_db::client_access` in a second language.

-- 1. Runs ------------------------------------------------------------------

CREATE TABLE accounting_export_runs (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- The connection is the target company/tenant AND the credential. Two QBO
  -- companies are two connections, so a receipt can never be read against the
  -- wrong books.
  connection_id uuid NOT NULL REFERENCES connector_connections(id) ON DELETE CASCADE,
  -- Resolved from the connection's installed manifest at claim time and stored
  -- here: a manifest that is upgraded or uninstalled later must not rewrite
  -- the history of what this run talked to.
  vendor text NOT NULL CHECK (vendor IN ('qbo', 'xero')),
  record_kind text NOT NULL CHECK (record_kind IN ('invoice', 'credit_note')),
  -- The Marula document id. No FK on purpose: this table outlives a document's
  -- row in a restore-to-an-earlier-point scenario, and the run's job is to be
  -- readable, not to cascade a money record out of existence.
  record_id uuid NOT NULL,
  state text NOT NULL
    CHECK (state IN ('in_flight', 'succeeded', 'failed', 'held')),
  -- Typed reason code, NULL when state = 'succeeded'. Held and failed share
  -- the column and are told apart by `state` — see the header.
  reason text,
  -- The vendor's own words, verbatim, when the vendor said anything. Never
  -- parsed into prose: a paraphrase of a validation error sends an operator to
  -- the wrong field.
  reason_detail text,
  -- The Marula tax code the caller named for this document, remembered so a
  -- resume after a mapping is added needs no re-issue and no second request
  -- body. NULL means the caller named none.
  tax_code text,
  -- The vendor tax code the mapping resolved to. Kept even when the run is
  -- held, so the report can say WHICH vendor code was missing from the books.
  vendor_tax_code text,
  attempts integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
  requested_by uuid REFERENCES users(id) ON DELETE SET NULL,
  -- When the current attempt took the row. The retry policy is an age test on
  -- this column, which is why it is separate from `updated_at`.
  claimed_at timestamptz,
  completed_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT accounting_export_runs_once
    UNIQUE (org_id, connection_id, record_kind, record_id)
);

-- The report's default view: one org's runs, newest first.
CREATE INDEX accounting_export_runs_recent_idx
  ON accounting_export_runs (org_id, created_at DESC);
-- The "waiting on a human" queue, which is the only filter the UI draws as a
-- work list.
CREATE INDEX accounting_export_runs_attention_idx
  ON accounting_export_runs (org_id, state)
  WHERE state IN ('held', 'failed');
-- The reconcile path reads by document, not by run id.
CREATE INDEX accounting_export_runs_document_idx
  ON accounting_export_runs (org_id, record_kind, record_id);

-- 2. Receipts --------------------------------------------------------------

CREATE TABLE accounting_export_receipts (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  connection_id uuid NOT NULL REFERENCES connector_connections(id) ON DELETE CASCADE,
  record_kind text NOT NULL CHECK (record_kind IN ('invoice', 'credit_note', 'payment')),
  record_id uuid NOT NULL,
  -- The vendor's id (QBO `Id`, Xero `InvoiceID`/`CreditNoteID`/`PaymentID`).
  -- This is what a credit note links against and what a payment is applied to;
  -- it is never the document number.
  external_id text NOT NULL CHECK (length(btrim(external_id)) > 0),
  -- The vendor's document number when it assigned or echoed one.
  external_number text,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  -- One receipt per document per connection: a second row would mean two
  -- vendor records for one Marula document, which is the double-count this
  -- table exists to make impossible.
  CONSTRAINT accounting_export_receipts_once
    UNIQUE (org_id, connection_id, record_kind, record_id)
);

-- The credit-note join: "what vendor invoice id did this invoice get?".
CREATE INDEX accounting_export_receipts_lookup_idx
  ON accounting_export_receipts (org_id, record_kind, record_id);

-- 3. Tax mappings ----------------------------------------------------------

CREATE TABLE accounting_tax_mappings (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  connection_id uuid NOT NULL REFERENCES connector_connections(id) ON DELETE CASCADE,
  -- The Marula-side code. Core carries no tax on an invoice (the ADR 0036
  -- boundary), so this is a label the export names — never parsed, only
  -- matched.
  marula_tax_code text NOT NULL
    CHECK (length(btrim(marula_tax_code)) BETWEEN 1 AND 120),
  -- The vendor-side id a document line carries (QBO `TaxCode.Id`, Xero
  -- `TaxType`). Stored as text because both vendors use opaque strings.
  vendor_tax_code text NOT NULL
    CHECK (length(btrim(vendor_tax_code)) BETWEEN 1 AND 120),
  -- The vendor's human name for it, captured when the mapping was written, so
  -- the report names a code a person recognises even after the vendor renames
  -- it.
  vendor_tax_code_name text,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  -- One vendor code per Marula code per connection. A second mapping would be
  -- two answers to one question, and the export would have to pick.
  CONSTRAINT accounting_tax_mappings_once
    UNIQUE (org_id, connection_id, marula_tax_code)
);

CREATE INDEX accounting_tax_mappings_connection_idx
  ON accounting_tax_mappings (org_id, connection_id, marula_tax_code);

-- 4. Row-level security ----------------------------------------------------

ALTER TABLE accounting_export_runs ENABLE ROW LEVEL SECURITY;
CREATE POLICY accounting_export_runs_org ON accounting_export_runs
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE accounting_export_receipts ENABLE ROW LEVEL SECURITY;
CREATE POLICY accounting_export_receipts_org ON accounting_export_receipts
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE accounting_tax_mappings ENABLE ROW LEVEL SECURITY;
CREATE POLICY accounting_tax_mappings_org ON accounting_tax_mappings
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- 5. What each table is for, in the catalogue ------------------------------

COMMENT ON TABLE accounting_export_runs IS
  'One Marula document being exported to one accounting connection: state '
  '(in_flight/succeeded/failed/held), typed reason, attempt count and claim '
  'time. Unique per (connection, document), so a retry advances the same run '
  'instead of adding a second one.';
COMMENT ON TABLE accounting_export_receipts IS
  'The vendor record created for a Marula document (QBO Id / Xero UUID). The '
  'credit-note link and every payment application resolve through this row, so '
  'the vendor id is stored rather than derived from a document number.';
COMMENT ON TABLE accounting_tax_mappings IS
  'Per connection, Marula tax code to vendor tax code. An export whose code has '
  'no mapping is held with a typed reason instead of guessing a default.';
