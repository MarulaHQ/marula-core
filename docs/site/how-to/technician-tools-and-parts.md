---
description: "Your day pack lists what the jobs need: the tools, the consumables, and the parts already reserved against the tickets. It lives on Today under the schedule."
---

# How to check out tools and use parts on site

> **Audience:** both — the technician does the scans and taps; stock admins read
> the same ledger this page writes to.

## Before the first job

Your day pack lists what the jobs need: the tools, the consumables, and the
parts already reserved against the tickets. It lives on Today under the
schedule.

1. Open the day pack and find a tool row.
2. Tap **Check out**. The form arrives filled in: the item, where it sits, you
   as the holder, and a due date of tomorrow.
3. Confirm. The tool leaves the shelf through the checkout transaction, and
   the row's availability chip updates.

Scanning the toolbox label does the same thing for the whole kit in one pass.
Scan a shelf or van label first when the stock is not in the stockroom — the
checkout records where the tool actually came from.

## Use a part on the job

When a part goes into the work, record it against the ticket you are standing
in front of.

1. On the day pack row, tap **Use…**. The parts form opens with the item and a
   quantity filled in.
2. Pick the location the stock actually leaves. This is often the van, not the
   shelf.
3. Set the quantity and add a note if the job needs one.
4. Submit. The movement attaches to the active ticket, and the ticket's parts
   section shows it.

The same form is under the stock item itself: open **Stock**, pick the item,
and use the consume fields there. A scanned item opens that screen directly.

## Return tools

1. Open **Tools** on the phone. The list shows every checkout you hold, with
   its due date.
2. Tap **Return** on the row and pick where the tool goes back to. The default
   is where it came from.
3. Scanning the tool's own label returns that one unit without hunting for
   the row.

A late tool stays visible on the list with its due date until it comes back.
Nothing disappears from the ledger when the date passes.

## Offline

A checkout, a return, or a consumption queues on the device when signal
drops. A scan that cannot reach the resolver says so on the spot and consumes
nothing; the same scan works once the phone is back on the network. The queue
inspector under **More → Developer tools** lists what is waiting to replay.

## What the office sees

- The ticket's parts section shows each consumption with who recorded it and
  when.
- The movement ledger under the stock item shows every in and out, including
  checkouts and returns.
- The toolbox view lists the units inside the kit and who holds them.

Stock numbers, the ticket, and the ledger are the same records on the phone,
in the browser, and in the reports. There is no mobile-only copy of anything.
