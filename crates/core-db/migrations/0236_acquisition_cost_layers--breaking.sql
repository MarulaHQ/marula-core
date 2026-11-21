-- Acquisition cost layers and their allocations (ADR 0035 decision 3, plan
-- §6; migration 0236). The identity, the money vocabulary and the purchase
-- intent already exist (0232-0235); this file is where a charge that arrived
-- after the goods becomes a document with a cost.
--
-- Two tables:
--
--   * `acquisition_costs` — one layer per charge document: freight, duty,
--     brokerage, insurance or handling. Vendor, the vendor's own reference,
--     our reference, the document date, the currency and the amount in
--     integer minor units, the allocation method and the rule that was
--     applied, the flag that decided whether the charge entered inventory
--     cost, and the reporting class it reports under. The FX snapshot is the
--     same atomic column group as every other money fact (0065/0103): the
--     layer locks the org's rate at ITS document date, and the read paths
--     never convert again.
--
--   * `acquisition_cost_allocations` — the split. One row per receipt line,
--     carrying the line's share in the layer's currency and in the org base
--     currency, plus the basis the share was computed from. Both totals are
--     exact: the writer apportions with a largest-remainder rule and a
--     statement guard refuses a set that does not sum to the layer.
--
-- Receipt cost is never edited. A wrong layer is reversed by a new layer that
-- names the one it reverses; a reversal carries the negated amounts, the
-- negated allocations and the same locked rate, so the pair nets to zero in
-- the base currency instead of re-converting a historical fact.
--
-- Mode. `orgs.landed_cost_mode` (0234) decides whether a new layer capitalises
-- into inventory cost (`landed_costs`) or posts as an expense under its
-- reporting class (`invoice_values`, the default). The mode in force when the
-- layer is recorded is stored ON the layer (`policy_mode`), together with the
-- decision it produced (`capitalised`). A later mode switch therefore cannot
-- change what a recorded layer means: the read paths answer from the layer,
-- never from the current policy.
--
-- Guards the schema enforces, because a concurrent writer can bypass any check
-- the API makes:
--
--   * an allocation set must sum to its layer, in the layer currency and in
--     the base currency, and the base columns are all set or all NULL;
--   * a layer's class and its capitalised flag agree: a capitalised charge is
--     `inventory`, and everything else is one of the seven expense classes;
--   * a reversal is negative, copies its allocation rather than splitting
--     again, and names a layer other than itself; one reversal per layer;
--   * a layer and its allocations are append-only (below).
--
-- UPGRADE NOTE on the two DROP CONSTRAINT statements at the end. They swap
-- `ticket_expenses` CHECKs for wider ones, which is why this file carries the
-- `--breaking` marker. Nothing is lost: the new CHECKs are supersets of the old,
-- every existing row still satisfies them, and no row is rewritten. The swap
-- validates the table once under ACCESS EXCLUSIVE (two statements on a table of
-- ticket-sized rows), which is the only cost an upgrade pays.
--
-- The ticket-expense CHECK widening. In `invoice_values` mode a layer posts as
-- an expense on the ticket the purchase serves. OD-1's rule is that a wrong
-- layer is corrected by a new document, so the reversal posts an expense too,
-- and that expense is negative. The schema refused negative money anywhere;
-- the two CHECKs relaxed below admit one shape only — a classed, unbilled
-- expense — which is exactly the row this module's reversal writes and no
-- other writer produces (`ticket_expenses.reporting_class` is written by
-- nothing else). The ticket surface's own entry path still refuses a negative
-- amount, and a negative row is never billable, so it never reaches the
-- invoice candidate feed.
--
-- Tenant erasure (core-db/src/erasure.rs). One `DELETE FROM orgs` must cascade
-- through every org-scoped row:
--
--   * Both append-only guards let a DELETE pass only when the parent org row is
--     already gone, and an UPDATE pass only when its sole effect is a nullable
--     actor column cleared by `ON DELETE SET NULL`. Everything else raises.
--   * `acquisition_cost_allocations` is the only foreign key child, with
--     `ON DELETE CASCADE`, so it cannot outlive its layer. Ids that name
--     another org-scoped row — the purchase order, the receipt, the receipt
--     lines, the vendor, the project, the ticket, the expense — are plain
--     uuids validated at write time, because a RESTRICT blocks the cascade and
--     a CASCADE would erase the history the layer exists to keep.
--
-- Lock cost: two new tables, their indexes and their triggers. Nothing here
-- touches a hot table except the two CHECK swaps on `ticket_expenses`, which
-- validate a table of ticket-sized rows (hundreds to low thousands at MSP
-- sizes) while holding ACCESS EXCLUSIVE for two statements. Indexes are plain
-- (not CONCURRENTLY) because the runner wraps a migration in a transaction.
--
-- Additive-only apart from those two CHECKs, which are widened to a superset:
-- no column is dropped or retyped, no row is rewritten.

-- --- the layer ---------------------------------------------------------------

CREATE TABLE acquisition_costs (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- The order the goods were bought on. A cascade parent: a layer cannot
  -- outlive its purchase.
  purchase_order_id uuid NOT NULL REFERENCES purchase_orders(id) ON DELETE CASCADE,
  -- The shipment the charge belongs to. A citation, not a foreign key: the
  -- write path proves the receipt is in the org and computes the split over
  -- its lines, and the layer survives a receipt-row deletion as the document
  -- it is.
  receipt_id uuid NOT NULL,
  -- The five charge kinds plan §6.2 names. `kind` is what the document is;
  -- `reporting_class` is how it reports, and they are separate because a
  -- capitalised freight charge reports as inventory.
  kind text NOT NULL
    CHECK (kind IN ('freight', 'duty', 'brokerage', 'insurance', 'handling')),
  -- The vendor that charged us. Plain uuid: the vendor row is org-scoped and
  -- may be deleted without rewriting the document that names it.
  vendor_id uuid,
  -- The vendor's own invoice number, as printed on their document.
  vendor_reference text
    CHECK (vendor_reference IS NULL
           OR char_length(btrim(vendor_reference)) BETWEEN 1 AND 100),
  -- Our reference for the document. Required: it is the stable name the
  -- accountant handoff cites.
  reference text NOT NULL
    CHECK (char_length(btrim(reference)) BETWEEN 1 AND 100),
  -- The document's own date, and the FX lock date (plan §6.3).
  document_date date NOT NULL,
  currency text NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
  -- Non-zero, signed: a charge is positive, a reversal is negative.
  amount_minor bigint NOT NULL CHECK (amount_minor <> 0),
  allocation_method text NOT NULL
    CHECK (allocation_method IN ('extended_value', 'quantity', 'weight')),
  -- Which rule produced the split. `largest_remainder` is the rule a split
  -- uses; `reversal_copy` means the rows were copied from the reversed layer,
  -- negated, rather than apportioned again.
  allocation_rule text NOT NULL
    CHECK (allocation_rule IN ('largest_remainder', 'reversal_copy')),
  -- The org's landed-cost mode when this layer was recorded, and the decision
  -- it produced. Stored, not derived: a later switch never rewrites a layer.
  policy_mode text NOT NULL CHECK (policy_mode IN ('invoice_values', 'landed_costs')),
  capitalised boolean NOT NULL,
  reporting_class text NOT NULL
    CHECK (reporting_class IN ('inventory', 'freight_in', 'duty', 'service_cost',
                               'rental_expense', 'ewaste_cost', 'overhead', 'cogs')),
  -- Where an expensed layer posts. Citations, resolved from the purchase order
  -- header at write time; the layer keeps them if the project or ticket is
  -- later deleted, which is the money history's job.
  project_id uuid,
  ticket_id uuid,
  -- The client the purchase ultimately serves, copied from the order header.
  -- A citation (no foreign key) like the rest: the layer is a document and the
  -- client row may be deleted. It exists so a restricted client's freight money
  -- is gated on every read without walking back to the order.
  client_id uuid,
  -- The ticket expense this layer posted, when it was expensed. One direction
  -- only (the expense cannot cite the layer: 0234 gave expenses a reporting
  -- class but no source pair).
  posted_ticket_expense_id uuid,
  -- The layer this one reverses. Same table, no foreign key: the reversal is
  -- its own document, and a chain of two reversals is refused by the shape
  -- CHECK and the unique index rather than by a parent link.
  reverses_acquisition_cost_id uuid,
  fx_rate_used numeric(24,12) CHECK (fx_rate_used IS NULL OR fx_rate_used > 0),
  fx_rate_date date,
  fx_source text,
  -- Null when the layer is denominated in the org base currency (the 0065
  -- rule: a single-currency fact carries no conversion snapshot and its
  -- amount IS its base amount). Signed for a reversal.
  base_amount_minor bigint,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT acquisition_costs_fx_snapshot_atomic CHECK (
    (fx_rate_used IS NULL AND fx_rate_date IS NULL AND fx_source IS NULL
      AND base_amount_minor IS NULL)
    OR
    (fx_rate_used IS NOT NULL AND fx_rate_date IS NOT NULL AND fx_source IS NOT NULL
      AND base_amount_minor IS NOT NULL)
  ),
  -- Only `landed_costs` mode capitalises, and only inventory is capitalised.
  CONSTRAINT acquisition_costs_capitalised_shape CHECK (
    (capitalised AND policy_mode = 'landed_costs' AND reporting_class = 'inventory')
    OR (NOT capitalised AND reporting_class <> 'inventory')
  ),
  CONSTRAINT acquisition_costs_reversal_shape CHECK (
    (reverses_acquisition_cost_id IS NULL
      AND amount_minor > 0
      AND allocation_rule = 'largest_remainder')
    OR
    (reverses_acquisition_cost_id IS NOT NULL
      AND amount_minor < 0
      AND allocation_rule = 'reversal_copy'
      AND reverses_acquisition_cost_id <> id)
  )
);

CREATE INDEX idx_acquisition_costs_receipt
  ON acquisition_costs (org_id, receipt_id, created_at);
CREATE INDEX idx_acquisition_costs_order
  ON acquisition_costs (org_id, purchase_order_id, document_date);
CREATE INDEX idx_acquisition_costs_class
  ON acquisition_costs (org_id, reporting_class, document_date);
CREATE INDEX idx_acquisition_costs_client
  ON acquisition_costs (org_id, client_id) WHERE client_id IS NOT NULL;

-- One reversal per layer. An index rather than a table constraint, for the
-- same reason 0235 records for its scope rows: the key is not user input (the
-- target id comes from the path and is checked by the write path), so a
-- collision is a writer bug that surfaces as a sanitised 409.
CREATE UNIQUE INDEX uq_acquisition_costs_reversal
  ON acquisition_costs (org_id, reverses_acquisition_cost_id)
  WHERE reverses_acquisition_cost_id IS NOT NULL;

-- --- the split ---------------------------------------------------------------

CREATE TABLE acquisition_cost_allocations (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  acquisition_cost_id uuid NOT NULL
    REFERENCES acquisition_costs(id) ON DELETE CASCADE,
  -- The receipt line this share belongs to. Plain uuid: the layer is the
  -- document, the line is what it was split over, and a deleted line must not
  -- erase the layer's split or block an erasure.
  receipt_line_id uuid NOT NULL,
  -- The share in the layer's currency, and in the org base currency. The base
  -- column is NULL for a layer already denominated in base currency, and the
  -- read uses COALESCE — the same rule the receipt movement carries.
  allocated_minor bigint NOT NULL,
  allocated_base_minor bigint,
  -- What the share was apportioned from under `allocation_method`: extended
  -- value in the order currency, quantity in milli-units, or weight in grams.
  -- Recorded so the split can be explained without recomputing it.
  basis_value bigint NOT NULL CHECK (basis_value >= 0),
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX idx_acquisition_cost_allocations_layer
  ON acquisition_cost_allocations (org_id, acquisition_cost_id);
CREATE INDEX idx_acquisition_cost_allocations_line
  ON acquisition_cost_allocations (org_id, receipt_line_id);
-- One share per layer per line: a split names each line once.
CREATE UNIQUE INDEX uq_acquisition_cost_allocations_line
  ON acquisition_cost_allocations (org_id, acquisition_cost_id, receipt_line_id);

-- The split must sum to the document it splits, in both currencies, and the
-- base column must be used one way for the whole set. A statement-level guard
-- over the inserted set is what makes "allocations sum exactly" structural
-- rather than a promise the writer keeps: the writer inserts the whole split
-- in one statement, and a partial or drifting set raises instead of landing.
CREATE FUNCTION acquisition_cost_allocations_balanced() RETURNS trigger AS $$
DECLARE
  layer_id uuid;
  layer_amount bigint;
  layer_base bigint;
  allocated bigint;
  allocated_base bigint;
  base_rows bigint;
BEGIN
  FOR layer_id IN SELECT DISTINCT acquisition_cost_id FROM new_rows LOOP
    SELECT amount_minor, COALESCE(base_amount_minor, amount_minor)
      INTO layer_amount, layer_base
      FROM acquisition_costs WHERE id = layer_id;
    IF NOT FOUND THEN
      RAISE EXCEPTION 'acquisition cost allocation names a layer that does not exist';
    END IF;
    SELECT COALESCE(SUM(allocated_minor), 0),
           COALESCE(SUM(COALESCE(allocated_base_minor, allocated_minor)), 0),
           COUNT(*) FILTER (WHERE allocated_base_minor IS NOT NULL)
      INTO allocated, allocated_base, base_rows
      FROM acquisition_cost_allocations WHERE acquisition_cost_id = layer_id;
    IF base_rows > 0 AND base_rows <> (SELECT COUNT(*) FROM acquisition_cost_allocations
                                       WHERE acquisition_cost_id = layer_id) THEN
      RAISE EXCEPTION 'acquisition cost allocation mixes base and non-base rows for one layer';
    END IF;
    IF allocated <> layer_amount THEN
      RAISE EXCEPTION 'acquisition cost allocations sum to % in %, not the layer amount %',
        allocated, (SELECT currency FROM acquisition_costs WHERE id = layer_id), layer_amount;
    END IF;
    IF allocated_base <> layer_base THEN
      RAISE EXCEPTION 'acquisition cost allocations sum to % in base currency, not the layer amount %',
        allocated_base, layer_base;
    END IF;
  END LOOP;
  RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER acquisition_cost_allocations_balanced
  AFTER INSERT ON acquisition_cost_allocations
  REFERENCING NEW TABLE AS new_rows
  FOR EACH STATEMENT EXECUTE FUNCTION acquisition_cost_allocations_balanced();

-- --- append-only -------------------------------------------------------------

-- A layer is a fact. Correcting it is a new layer that reverses it, never an
-- edit. The two exceptions are not edits and must pass, or tenant erasure
-- cannot succeed: a cascade reaches this row after its org is already gone,
-- and deleting the actor row nulls `created_by` while the record stays
-- otherwise identical. Both allowances mirror 0232's ledger guard and 0235's
-- recharge guard.
CREATE FUNCTION acquisition_costs_append_only() RETURNS trigger AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN
    IF NOT EXISTS (SELECT 1 FROM orgs o WHERE o.id = OLD.org_id) THEN
      RETURN OLD;
    END IF;
  ELSIF TG_OP = 'UPDATE'
        AND NEW.created_by IS NULL
        AND (to_jsonb(NEW) - 'created_by') = (to_jsonb(OLD) - 'created_by') THEN
    RETURN NEW;
  END IF;
  RAISE EXCEPTION 'acquisition_costs is append-only: % rejected', TG_OP;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER acquisition_costs_append_only_guard
  BEFORE UPDATE OR DELETE ON acquisition_costs
  FOR EACH ROW EXECUTE FUNCTION acquisition_costs_append_only();

CREATE FUNCTION acquisition_cost_allocations_append_only() RETURNS trigger AS $$
BEGIN
  IF TG_OP = 'DELETE'
     AND NOT EXISTS (SELECT 1 FROM orgs o WHERE o.id = OLD.org_id) THEN
    RETURN OLD;
  END IF;
  RAISE EXCEPTION 'acquisition_cost_allocations is append-only: % rejected', TG_OP;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER acquisition_cost_allocations_append_only_guard
  BEFORE UPDATE OR DELETE ON acquisition_cost_allocations
  FOR EACH ROW EXECUTE FUNCTION acquisition_cost_allocations_append_only();

-- --- row-level security ------------------------------------------------------

ALTER TABLE acquisition_costs ENABLE ROW LEVEL SECURITY;
CREATE POLICY acquisition_costs_org ON acquisition_costs
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE acquisition_cost_allocations ENABLE ROW LEVEL SECURITY;
CREATE POLICY acquisition_cost_allocations_org ON acquisition_cost_allocations
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- --- the reversed document's posting -----------------------------------------

-- See the header: a reversal of an expensed layer posts a negative expense, and
-- these two CHECKs are the only place the schema admits negative money. The
-- widened arm is a classed, unbilled expense — the row this module's reversal
-- writes. Nothing else writes `ticket_expenses.reporting_class`, the ticket
-- entry path refuses a negative amount, and a negative row is never billable.
ALTER TABLE ticket_expenses
  DROP CONSTRAINT ticket_expenses_amount_minor_check;
ALTER TABLE ticket_expenses
  ADD CONSTRAINT ticket_expenses_amount_minor_check
  CHECK (amount_minor >= 0
         OR (reporting_class IS NOT NULL AND invoice_line_id IS NULL));

ALTER TABLE ticket_expenses
  DROP CONSTRAINT ticket_expenses_base_amount_minor_check;
ALTER TABLE ticket_expenses
  ADD CONSTRAINT ticket_expenses_base_amount_minor_check
  CHECK (base_amount_minor IS NULL
         OR base_amount_minor >= 0
         OR (reporting_class IS NOT NULL AND invoice_line_id IS NULL));
