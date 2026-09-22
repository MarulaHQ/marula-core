# Tutorial: your first invoice

> **Audience:** admin (billing runs are an admin + bookkeeper surface) —
> staff contribute the billable time; this lesson runs the run itself.

The path from "someone's ticket work was approved" to money follows one
arrow that never bends near your calendar: time entry → approval → billing
run → invoice. This lesson walks that arrow on the seeded demo org
(`owner@demo.local` / `demo-password-123`).

## Prerequisites

- The seeded demo org (`owner@demo.local` / `demo-password-123`) or your own
  stack with at least one client and a time entry awaiting approval.
- Access to the Billing and Approvals surfaces (billing is admin +
  bookkeeper scope).


## 1. Confirm approved time exists

Open **Approvals** (/approvals). The queue lists submitted time
entries. Approved entries are what billing consumes, and entries once
invoiced lock permanently. There is no un-invoicing, only correction
documents. If the
demo queue is empty, work a ticket first ([first ticket lesson](first-ticket.md)
shows the timer path) and approve the entry. Note the client: the demo org
already has clients with contracts, so you are not creating scaffolding.

## 2. Create and run a billing run

Open **Billing** (/billing). Stat cards show open / overdue / outstanding /
paid. Below, the invoice-runs surface and the **unbilled ticket
candidates**
read-only panel — a list of tickets whose time and expenses (with cost and
expense kinds) are not yet on an invoice, including the explicit
consumption note (a consumed line is marked by `invoice_line_id`). Start a
run against the client. The run reads:
- **Contracts: drafts are excluded** — "no runs bill drafts" (contracts
  with status draft are skipped, while recurring and time-and-materials
  types both flow).
- The run creates invoice lines with kind `time_materials` and a period
  window. Lines are **fixed at run**: the invoice is a snapshot of the
  numbers, not a live re-render.

## 3. The issued invoice

Open the new invoice. It is **read-only once issued**: lines fixed, a
payments panel, and the actions available to correct rather than edit —
Issue credit note, Void, Record payment, Print document, Send statement.
Email delivery of a statement in the demo/none-mode is honest about being
queued, not sent.

## 4. If money reverses: credit notes and collections

Both corrections start from the invoice page's actions:

- **Credit note / void** live in the credit-notes lifecycle
  (/billing/credit-notes): notes are derived from usage, and applying /
  refunding / voiding leaves a bar-metered usage trail. There is no
  invoice-line rewriting — corrections preserve ledger truth.
- **Collections ladder** is forward-only: current → reminder sent → payment
  overdue → final notice → in collections. Each advance requires a note
  and the ladder never rewinds.

## 5. Multi-currency stacks: one note

If a contract is priced in a currency other than the org's base currency
(Settings→General), the run snapshots FX rates. Read the rationale in
[FX additivity](../explanation/fx-additivity.md) and record rates with the
[FX how-to](../how-to/admin-fx-rates.md). The demo contracts include a
priced-book stat. The mechanics are identical across currencies, and
the ledger stays anchored to base currency.

## What you learned

- Billing time comes from **approved** entries only, and invoiced
  entries lock.
- A billing run is a snapshot engine: lines fixed at run, contracts-drafts
  excluded, per-line kind `time_materials` against a period window.
- Corrections are additive (credit notes, voids, forward-only collections)
  — never rewrites.
- The reconciliation math this implies is explained in
  [FX additivity](../explanation/fx-additivity.md).
