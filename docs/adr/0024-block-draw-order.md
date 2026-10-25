# ADR 0024: Prepaid block draw order — earliest expiry first

Date: 2026-10-25 · Status: Accepted · Lane: p54-money-and-roles

## Context

A prepaid block is sold as a number of minutes with an expiry date, and an invoice
draw consumes minutes from the org's blocks. Nothing ever wrote down which block a
draw should take from. The engine took the oldest sale first, by `(created_at, label)`,
and that order was an accident of a sort key rather than a rule anyone had chosen.

Two problems came out of it.

**The order was unstated, so it could not be reviewed.** A customer asking "why did
this entry eat my January block instead of the February one" had no answer better
than "the code sorts that way". The order also decided which minutes expired unused,
which moves the "minutes expiring" alert.

**The draw was applied more than once.** Repricing read a bucket of minutes by work
type with `get` where it needed `remove`. A cap scoped to one period emits a line per
cap window, and every one of those lines carries the same work type in its suffix, so
one draw priced once per line. At a block rate below the card rate that under-bills the invoice, and the
bug is live, not latent.

## Decision

Draws consume the block with the **earliest expiry** first. Blocks with no expiry
sort after every block that can lapse, because those are the minutes that can be
lost. Ties break on `(created_at, label)`, so the order is total and deterministic.

Each work-type bucket is consumed **exactly once** per re-price.

A draw that has already been recorded is **never rewritten**. The rule applies to
selection, not to history: existing invoices keep the block their draw name, and the
change is forward-only.

## Consequences

The draw's `block_consumption.draws` names a different block whenever a later block
expires sooner, and that is what an invoice run persists from now on.

An invoice total moves only when the two blocks price differently, because each
drawn minute re-prices at its own block's rate.

The "minutes expiring" alert moves with it. A block that used to lapse with minutes
unused may now be drained first, and a block that used to be drained may now lapse.

No migration, no data rewrite, and no per-org flag. This decision was taken before
any release, so there is no sold contract to grandfather: the invoicing rule should
be the one that is defensible, not the one that matches an accident.

## What would make this wrong

If a block's price is ever allowed to change after sale, earliest-expiry-first stops
being obviously fair, because the customer paid for those minutes at a stated rate
and the draw order now decides which rate a later minute carries. That would need a
rule about which block a credit reverses against, and it is not in scope here.
