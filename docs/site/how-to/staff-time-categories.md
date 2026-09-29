# How to log billable time with categories

> **Audience:** both — staff log the time, admins own the taxonomy the
> categories come from.

Every time entry records what kind of work it was, and that one field
decides how it bills. The category is a slug from your org's taxonomy,
not free text. This page covers both halves: logging it, and owning the
list.

## 1. Log an entry

Three surfaces write time, and all three take the same two fields.

- **Timer stop** on the workbench or the day page.
- **Quick log** on Today.
- **The entry form** on a ticket or a project phase.

You pick a category from the ordered picker the admin arranged in
Settings. The picker shows live categories only, in the order the admin
set. Two fields ride with it:

- **Billable** — the category's default applies, so most entries need no
  thought.
- **Reason** — required when a category is set to demand one and you
  turn billing off. The set is fixed: warranty, courtesy,
  in_retainer_block, sla_covered, internal, mistake. Sending an entry
  non-billable with no reason is refused with a 422.

On a ticket the wrap-up sheet also offers a suggestion chip from the
billing audit, showing the probability it read of the entry's billability
("Looks billable", "Looks non-billable — add a reason?"). Clicking it
prefills the form. Nothing is written until you submit.

## 2. Where a category lands

The slug keys three places at once, which is why it is a slug and not a
label:

- The entry's `work_type`.
- The T&M rate card on the contract, so a rate card row for `on_site`
  prices on-site work.
- The reports and scorecards, which read the same column, and the report
  builder's field catalog.

A label can change any time. A slug never does, because changing it would
re-point every historical entry and every rate card row at once.

## 3. Work logged before the taxonomy existed

Capture surfaces normalize pasted free text through a mapping table, so
an old value still lands correctly. `onsite` resolves to `on_site`.
Anything unmapped is refused rather than guessed — a silently mispriced
entry is worse than a rejected one.

Admins manage that map on the same settings page. Add a row: the free
text a tech might type, and the category it should resolve to. Matching
is case-insensitive and ignores surrounding spaces. Migration 0110 seeded
the map once from the free text already in the database; anything you
add shows as "added here" rather than "migration 0110".

**Re-key history** is a separate, deliberate action on a row you added.
It rewrites every entry (and rate-card row) still carrying the legacy
value onto the category's slug, and reports how many it touched. Adding
a mapping does not rewrite history on its own.

## 4. Admin: run the taxonomy

Open **Settings → Time categories**. Each row carries the three flags the
capture surfaces read:

- **Billable by default** — applied to new entries in that category.
- **Ask for a reason when non-billable** — the entry form demands a typed
  reason before billing can be switched off.
- **Keyed by cards** — documentation that T&M rate cards are expected to
  price this category. It is a hint for whoever maintains contracts, not
  a write path.

The up and down buttons set the order, and that order is what every
capture picker shows. Reordering writes the whole order in one call.

## 5. Admin: retire, never delete

Retire takes a category out of the pickers while leaving its entries
alone. Entries and rate-card rows keep the slug they were written with,
so historical work keeps its meaning, and the **Entries** column on each
row tells you how much history rides on it before you retire anything.
Retired rows group at the bottom of the list for reading; their stored
positions are untouched. Restore puts one back in the pickers.

Deleting a category would orphan entries and silently re-price them at
the contract default. There is no delete for that reason.

## 6. What an admin cannot see here

Rate-card rows. Contracts stay contract-owned: a category's "keyed by
cards" flag says what a contract is expected to price, and editing the
card itself happens on the contract. See
[how to price a service contract](admin-contract-types.md).

## What you learned

- A category is a permanent slug, and it decides billing before approval.
- Turning billing off on a reason-demanding category requires a typed
  reason, or the entry is refused.
- Legacy free text normalizes through a mappable table. Unmapped values
  are refused.
- Categories are retired with their history intact, never deleted.
