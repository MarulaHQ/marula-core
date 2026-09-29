# How to price a service contract

> **Audience:** admin — the contract editor is a settings-owner surface
> (commercial edit plus `financial:read`).

A contract is one client's answer to *how do we get paid for this work?*
The type you pick decides what the engine evaluates. Everything else on
the page is the configuration that type reads.

Open **Contracts** (/contracts) and pick one. The editor is
per-agreement: identity and lifecycle at the top, the commercial fields
for the type, the child rows (blocks, caps, change orders, service
credits), then a billing preview over any window you choose.

## 1. Pick the type that matches the deal

| Type | What bills | Fields that matter |
|---|---|---|
| Recurring | a flat price per period | price, frequency, billing anchor |
| Retainer | a per-period base that buys included minutes | price, included minutes, overage rate, blocks |
| Time & materials | approved labor off a rate card | default hourly rate, rate card, caps, blocks, minimum |
| Fixed fee | dated schedule entries | fee schedule |
| Per unit | price × count, counted at evaluation | price per unit, count by devices or users |
| Per incident | a price per closed ticket in a priority band | price per closed ticket, ticket band |

Two rules the server enforces, so the form never offers what would be
refused: prepaid blocks ride T&M and retainer only, and NTE caps are
T&M-only.

## 2. Blocks: hours the client bought up front

A block is a prepaid balance: minutes sold at a sale price, drawn down
by approved work, oldest block first. The editor's **Prepaid blocks**
card takes a label, total minutes, sale price, an optional block rate,
a rollover policy, and an expiry date.

- **Block rate** is a discount against the rate card for work priced
  against that block. Leave it empty and draws bill at the card rate.
- **Rollover** is `expire` (unused minutes lapse), `rollover` (they mint
  a successor block), or `none` (they hold, with no expiry).

Blocks are replaced wholesale when you save the card, and each save mints
new rows rather than editing the old ones: a block's identity is its draw
history. The balance itself is not on this page. It is on the
[verification workspace](admin-billing-verification.md), which is where a
period gets checked.

## 3. Caps: a ceiling, and what happens at it

A not-to-exceed cap puts a bound on billable work. Each cap says what
reaching it does, so it is never silent:

- **Warn only** — the cap rides the evaluation as a warning and the bill
  continues.
- **Stop at the cap** — work past the ceiling is dropped.
- **Bill the overage** — work past the ceiling bills at the cap's
  overage rate.

Scope is per period or per contract. A cap can also name one work type,
which is how a cap covers remote work without capping on-site work. Save
the card to replace the set. Saving an empty set removes every cap.

## 4. Minimums and escalators

A **minimum commitment** tops a short period up. A period that bills
below the minimum gets a true-up line, so the contract's floor is real
rather than a hope. The engine counts the labor the period consumed
alongside the base, so a T&M contract carries a floor too. Clear the
field and save to remove it.

An **escalator** lifts the base at each billing anniversary: 300 basis
points is +3.0% a year. It walks anniversaries off the billing anchor, so
it needs a billing frequency and only the types that have one offer it:
recurring and retainer. Set the date to start the uplift, or leave it
empty and the first uplift lands on the anchor's first full period. Clear
it the same way as a minimum.

## 5. Change orders

A change order is approved scope that bills as its own line. Add one
with a number, a title, an amount, and an optional description. The
lifecycle is `draft → approved | rejected | cancelled`, and `billed` is
run-owned: the invoice run flips an approved order to billed and links
the invoice. Until it is approved, a change order prices nothing.

Approve or reject from the row. Cancelling an approved order withdraws
it before it reaches an invoice. Once billed, the row is history.

## 6. Service credits

A service credit is what the agreement owes back when a target was
missed. It needs a reason in words ("resolution target breached by 2d
4h"), an amount, and the date whose billing window it lands in. A
pending credit in a run's window becomes a negative line, and that run
marks the credit applied. Applied credits are immutable, so a mistake is
corrected by issuing a new credit, not by editing the old one. Pending
credits can be cancelled from the row.

## 7. The purchase-order number

One field, copied onto every invoice the contract produces. Tax is not
computed here and not stored on the invoice: the boundary is deliberate
and [explained](../explanation/contract-relationship-types.md). A blank
field leaves the stored PO alone, because a patch that sends nothing
means "unchanged".

## 8. Read the preview before you run

The **Billing preview** card evaluates the contract over any window you
pick: line count and total, through the same engine a run uses. The
per-contract detail is on the
[verification workspace](admin-billing-verification.md), which pairs
that preview with the entries behind it.

## What you learned

- Blocks are prepaid balances and caps are ceilings with a typed
  response. Both are replaced wholesale, never edited row by row.
- Minimums true up against base plus consumed labor. Escalators lift on
  the anniversary, so they need a billing frequency.
- Change orders bill only once approved. Service credits bill once, then
  freeze.
- The PO number prints on invoices while tax stays at the
  accounting-export boundary.
