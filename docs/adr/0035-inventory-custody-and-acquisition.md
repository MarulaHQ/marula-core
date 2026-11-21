# ADR 0035: Inventory custody and acquisition for billable product lines

**Date:** 2026-11-20
**Status:** Accepted · 2026-11-21
**Ruling:** A physical unit has one identity and one custody class at a time.
Acquisition records why the unit was bought and where it must land. Cost is
layered and never rewritten. Money corrections and physical returns are
separate documents, and both cite the record that caused them.

## Context

Marula already ships four inventories that look similar and share almost
nothing:

- sellable stock (`stock_items`, `stock_levels`, `stock_serials`,
  `stock_movements`);
- the organisation's own fleet and client devices (`device_assets`, with
  `scope = internal_org | client`);
- tools checked out to people (`stock_items.is_tool` plus
  `tool_checkout_lines`);
- devices loaned or leased to clients (`client_loans`,
  `client_loan_lines`).

Purchasing feeds only the first of them. A purchase order names a project and
a stock item. Receiving refuses a line that has no stock item, so a purchase
can produce owned stock and nothing else. A bill rolls into a project's cost
rollup and stops there. There is no client on a purchase order, no ticket, no
statement of intent, and no route to an asset or a rental pool.

Billing has the same shape at the next layer. A ticket cost line is free text with a
quantity and a unit cost. It carries no catalog item, no stock item, no serial,
and no unit of measure. An invoice line records the contract that produced it,
not the ticket line a run consumed. A credit note is a total with no lines, so
a credit against a physical product cannot say what was credited or what should
happen to the goods.

Three consequences follow, and they are the reason for this decision:

1. The same part can be consumed from stock once and billed zero or twice.
2. A credited part cannot be restocked, because no record connects the credit
   to a unit, a quantity, or a serial.
3. A decommissioned laptop cannot become sale stock, and a purchased rental
   unit can only be recorded as if it were for sale.

## Decision

### 1. One unit, one custody class

Every tracked physical unit has exactly one identity (a serial, an asset, or a
lot line) and exactly one custody class:

| Class | Owner | Valuated as |
|---|---|---|
| `sale_stock` | Organisation | Inventory at cost |
| `internal_fleet` | Organisation (owned) | Capital |
| `rental_in` | Vendor (including gear the organisation leases or rents for its own use) | Rental expense, never depreciated |
| `rental_out` | Organisation, placed with a client | Rental income |
| `client_custody` | Client | Not ours |
| `ewaste_custody` | Client, organisation is custodian | Custody only |
| `tools` | Organisation, held by a person | Inventory at cost |

A unit that leaves service ends in one terminal disposition: `sold`,
`recycled`, `destroyed`, or `returned`. The identity and its serial or asset
row stay; what changes is that the unit no longer sits in a live class.

A move between classes is a movement in the append-only ledger, with a reason,
an actor, and the document that caused it. No class is inferred from a status
string, and no unit is in two classes at once.

`device_assets` remains the lifecycle projection for anything a person uses.
`stock_serials` remains the identity for anything stocked. Each side carries a
nullable reference to the other, so a conversion updates one unit rather than
creating a second one.

### 2. Acquisition records intent and destination

A purchase order line declares what the purchase is for and where the goods
must land: sale stock, internal fleet, rental pool, client custody (stocked or
drop-shipped), a project or ticket cost object, or an e-waste service.

Receipt is the creation moment. One receipt line produces exactly one
consequence in the destination class: a stock movement, an asset row, or a
custody row. A drop-ship purchase creates a client asset and a recharge, and
changes owned quantity by zero.

A purchase made on a client's behalf resolves a recharge rule at the moment it
is recorded. The rule is `at_cost`, a percentage markup, a fixed charge, or
`none`, with ticket/project overriding client/agreement overriding the
organisation default. A fixed charge replaces the markup: it is the whole
charge in the rule's currency, not an addition to the receipt cost. The resolved
rule and the basis it was applied to are stored on the recharge line, so a later
policy change cannot rewrite history.

### 3. Cost is layered and never rewritten

Receipt cost is a fact. Freight, duty, brokerage, insurance and handling are
later documents, not edits. Each acquisition cost is a layer with its own
currency, its own FX snapshot at its document date, and an allocation to
receipt lines.

Allocation is by extended value, quantity, or weight, in integer minor units,
with a recorded largest-remainder rule so allocations sum to the layer exactly.
Unit cost basis is the receipt plus its allocated layers.

An organisation may instead record invoice values only. In that mode the charge
posts to the project or ticket as an expense with a reporting class, so margin
still reflects it and the accountant still sees the class. Switching modes
applies to new receipts and never rewrites a historical layer.

Reporting classes (`inventory`, `freight_in`, `duty`, `service_cost`,
`rental_expense`, `ewaste_cost`, `overhead`, `cogs`) are data carried by the
document, not a convention inferred by a report. They map to external ledger
accounts through the same pattern the tax-code mapping already uses.

### 4. Rentals are custody, not stock

Owned rental gear is a distinct pool from sale stock. A unit may be commissioned
from sale stock into the pool or decommissioned back, by movement, with a
condition grade and data-wipe evidence where the device stores data. A unit on
an open rental agreement cannot move.

A lease to a client records its terms as an agreement and produces rental
charges that bill through the ordinary invoice run. A rental from a vendor
records the vendor agreement, the rental expense and the return. Rented-in
units never depreciate and never enter owned valuation.

### 5. E-waste is custody with a disposition

Client e-waste is received as an intake: client, site, ticket if any, date,
received by, data-destruction requirement, hazard flags, and item lines with
weight and serial where known. It never enters owned stock levels or valuation.

Every outbound pathway is recorded: recycled, destroyed, resold, returned to
the client, or transferred to a partner. Each carries a date, a weight, a
partner where applicable, and a certificate reference. A disposition is refused
while required data destruction is pending. Where the item was already a client
asset, intake retires it in the client's fleet with the certificate as
evidence, rather than creating a duplicate.

### 6. Billing corrections reach physical goods

A billable line carries its product identity: catalog item, stock item, serial,
quantity and unit. An invoice line carries the source line it consumed. A
credit note carries lines, not only a total, and each physical credit line
declares a disposition: restock to sale, restock to the rental pool, refurbish,
parts, or e-waste.

The existing correction doctrine is unchanged: an issued invoice is a record,
corrections are new documents, and nothing is rewritten. What changes is that a
correction can now name the unit and dispose of it.

### 7. The accounting boundary stays where it is

Marula is not a general ledger. It does not compute tax, post journals, or
close periods. It produces accurate documents, cost layers, and custody
records, each with a stable reference and its FX snapshot, and exports them to
the accounting system or as a parseable package for an accountant.

## Consequences

- Existing stock, fleet, tool and loan surfaces keep their behaviour. The new
  columns are nullable and the defaults preserve current meaning.
- Every new physical capability needs a class and a movement. A feature that
  cannot name both is not ready.
- Inventory valuation becomes derivable from layers, so a replayed ledger and a
  stored valuation must agree.
- The product gains a small amount of ceremony: a credit for a physical product
  asks a disposition question. That question is the audit trail.
- Export grows from invoices and credit notes to vendor bills, cost layers,
  valuation snapshots, rental charges, and e-waste dispositions.

## Alternatives considered

- **One new table for every inventory kind.** Rejected: it repeats the identity
  and the ledger four times, and the copies drift.
- **Treat fleet and rentals as stock items with a flag.** Rejected: a flag
  cannot express "never sell this" without leaking into every query, and the
  cost rules differ from sale stock in both valuation and depreciation.
- **Edit receipt cost when freight arrives.** Rejected: it breaks the lock that
  makes historical margin and FX reproducible.
- **Restock automatically on credit.** Rejected: a credit does not prove the
  goods came back, and a service row must never add stock.

## Compliance

- Corrections remain additive. No issued document is edited.
- Cost layers and movements are append-only, and every reversal is a new row.
- Every physical credit and every e-waste disposition names its evidence.
- No customer data, credential, or commercial term is required by the model.
