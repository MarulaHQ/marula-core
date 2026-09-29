# Billing corrections, and where tax lives

> **Audience:** both — admins decide how a bill gets corrected, staff see
> the result on the invoice and in the portal.

An issued invoice is a record of what happened, not a draft that can be
tidied up later. That single decision is why every correction in Marula
billing is a new document instead of an edit, and it is also why no tax
number is computed on an invoice. Both fall out of the same boundary, so
they are worth stating in one place.

## Corrections are additive

Once a run issues, its lines are fixed. A run is a snapshot of the
numbers the engine evaluated for that window, and the snapshot is what
the client received. Editing it would mean the document no longer says
what was sent.

So the paths forward are all forward:

- **Credit notes** derive from the invoice and live in their own
  lifecycle. Applying or refunding one leaves a usage trail, and voiding
  one is a recorded state on the note.
- **Voiding** an issued invoice is a recorded state, not a deletion.
- **Collections** advance through a ladder (current → reminder sent →
  payment overdue → final notice → in collections). Each step needs a
  note, and the ladder never rewinds.

The same reasoning runs through the rest of the billing model, and the
shape is the same everywhere: keep the original, add the correction.

| Instead of | Marula does |
|---|---|
| Editing a time entry's minutes to lower a bill | an additive adjustment row, signed, with a reason |
| Editing an invoiced entry at all | refuse with 409, and keep the row visible and labelled locked |
| Rewriting a service credit that has been applied | refuse. Issue a new credit |
| Deleting a change order that a run billed | mark it billed, with the invoice that consumed it |
| Re-running a window that overlaps an issued run | allow it, warn about the overlap, keep both on record |
| Deleting a time category | retire it, and leave the entries keyed to its slug |

Verification is the one place a number can still move, and it is narrow
on purpose. An adjustment requires a reason and can only lower the bill.
Raising minutes is refused, because raising a bill is an edit to the
entry rather than a review decision. Every action writes an audit row
naming the actor, the moment, and the before and after.

## What "verified" means, and what it does not

A sign-off records that a person looked at a window and attested to three
things: every entry dispositioned, zero unclassified entries, preview
read. The server re-derives all three inside the writing transaction, so
the attestation cannot drift from the state it describes.

It stays evidence, not a lock. A run over an unverified window still
issues. The difference is that the sign-off is claimed by the run that
covers it, which turns "was this period checked, and by whom?" into one
lookup instead of an archaeology project.

## Why invoices carry no tax

Tax is absent by design, and the boundary was drawn deliberately when
invoicing shipped:

- **Totals are line sums.** An invoice total is what its lines add up to.
- **No tax engine runs on invoices.** Nothing is computed, stored, or
  rounded per jurisdiction, rate, or exemption.
- **The purchase-order number is the print boundary, not a tax field.**
  A contract's PO is copied onto every invoice it produces so the client's
  own paperwork matches. It is a reference, not a liability.

The consequence for an integrator is concrete: if a client needs tax on
the invoice document, that calculation happens in the accounting system
the invoice is exported to, using the lines as its input. Marula's job
ends at an accurate, complete, immutable set of lines.

The agreement shapes that produce those lines are in
[contract relationship types](contract-relationship-types.md), and the
data model behind them is in the
[billing schema reference](../reference/billing-schemas.md). The task
paths are [verifying a period](../how-to/admin-billing-verification.md)
and [pricing a contract](../how-to/admin-contract-types.md).

## The reason this is boring on purpose

Every mechanism above trades convenience for a trail an auditor can
follow without trusting anybody's memory: who approved the entry, who
adjusted it and why, who signed the window, which run consumed a block
balance, which invoice applied a credit. The cost is that fixing a
mistake takes a document instead of a keystroke. That is the trade, and
it is deliberate.
