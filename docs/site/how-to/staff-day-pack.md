---
description: "The day pack is the trip preparation panel at the top of Today. It answers three questions before you leave."
---

# How to work a day pack

> **Audience:** both — technicians run the pack from `/today`, and
> dispatchers or admins watch the same panel when they cover a route.

The day pack is the trip preparation panel at the top of **Today**. It
answers three questions before you leave. Which stops are on this trip.
What has to be in the van. Where each item comes from. Ticking a line is
a plan note, and it never moves stock. Stock moves when you check a tool
out or use a part, which is the next step on the same row.

## Prerequisites

- Your day has at least one appointment, and the appointment is on a
  ticket.
- Stock exists for the items the job needs, or a pick or dispatch rule
  points at the right item.
- For travel-aware grouping, set today's start point under **Dispatch**.
  Without one the pack still lists everything, and the start line says no
  start point is set instead of naming a place.

## 1. Read the trips

The pack splits the day into trips. A trip ends where you can restock,
at the office or at your own van.

- **Van pack** — the trip runs site to site. Everything on the list
  should already be in the van, or come out of the first stop's stock.
- **Collect at the office** — the trip starts at the office, or the
  travel record says you return there between stops. Collect the trip's
  list before you roll. A later trip gets its own list.

Back-to-back site days stay one trip pack. If you tick only part of a
multi-stop trip's consumables, the trip header warns how many units short
the rest of the trip is.

## 2. Tick as you collect

Each line has a tick box on the left.

- A tick writes a **plan note** for you and today's date. Other people's
  packs are unaffected.
- A second click removes the note.
- Ticking does not change stock on hand. It does not check anything out.
  The availability chip stays visible, so you can see what is still
  missing.

The header counts the pack. Stops, lines to grab, lines collected.

## 3. Check out tools, use consumables

Every row carries the action that keeps the ledger honest.

- **Check out** (tools) opens the checkout form with the item, the
  source location, you as holder, and tomorrow's due date filled in.
  Confirm it, and the tool leaves the shelf through the normal checkout
  transaction.
- **Use…** (consumables) opens the parts form on the ticket with the item
  and quantity filled in. Choose the location the stock actually leaves,
  then record it. The consumption lands on the ticket as a movement.

The chip on the right of each row is the availability truth.

- **Available** — enough units where the row points.
- **Partial: 2 of 4** — take what is there and collect the rest later.
- **None available** — the row names who has it and when it is due back.
- **Not stocked** — no stock records for the item yet. Create one under
  **Stock** or ask the stockroom.

## 4. Pin something from a ticket

Two ways an item lands on a pack.

1. **Pin to day pack** on the ticket header. Pick the item, quantity, and
   a note. Pins show as chips on the ticket, so anyone can unpin them.
   Stockroom staff use the same button when they stage a job.
2. **Dispatch defaults** — admins map a ticket type, category, or board
   to items or a whole kit under **Settings → Toolboxes & day pack**.
   Every matching ticket picks the mapping up automatically.

Open checklist tasks appear under the tickets too. When a task names a
stock item, the pack matches it and treats it as a suggestion. Otherwise
it stays a plain reminder with a **Pin item** shortcut.

## 5. Tomorrow, tonight

When tomorrow does not start at the office, the pack adds tomorrow's list
under **Tomorrow**, so equipment rides home in the van and goes straight
out again. Ticking a tomorrow line writes tomorrow's plan note, not
today's.

If tomorrow starts at the office, the preview stays off. You will collect
there in the morning. If no start point is set, the panel shows the list
with a note saying it assumed the van goes home.

## Troubleshooting

- **The panel is missing.** You have no scheduled stops today and no
  lines to collect. Book the job first.
- **A job needs something the pack does not show.** Pin it from the
  ticket, or add a dispatch default for its category.
- **An item shows "Not stocked".** The item exists but has no location
  levels or serial units. Receive stock first, then reopen Today.
- **You cancelled a checkout.** Ticking stays. The availability chip goes
  back to its old count on the next refresh.
