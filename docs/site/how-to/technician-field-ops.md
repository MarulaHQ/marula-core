---
description: "The phone moves stock through the same ledger as the web app. A transfer you make in a basement and a transfer the stockroom makes at the desk are the same…"
---

# How to move stock and check out tools in the field

> **Audience:** both. Technicians run these flows on the phone. Owners and
> admins set up the locations and tools the flows read from.

The phone moves stock through the same ledger as the web app. A transfer you
make in a basement and a transfer the stockroom makes at the desk are the same
row on the same movement history. Nothing is mobile-only, and nothing lands
until you confirm it: a scan fills a form, it never records a movement.

## Scan a label

1. Open **Scan** and tap **Open the camera**, or type a code into the field
   under it. Both paths call the same resolver, so the result is the same.
2. The card tells you what the code means. **Open** goes to that record.
3. If two records share a code, the card lists them and asks you to pick. The
   app never guesses between two physical things.
4. If nothing matches, the card says so and keeps your text. Check the label,
   or type the code by hand.

The camera needs a real device. The simulator has no lens, so it points you at
the field instead. Every scan flow behaves the same once you are on a phone.

## Check out a toolbox

1. Scan the toolbox stencil, the code printed for the box (for example
   `KIT-FIELD-1`). The Scan tab resolves it as a **Toolbox**.
2. **Open** shows the kit, its member tools, and availability at each location.
   A member line reads **READY** or **SHORT**, and short tells you how many
   units are there.
3. Pick the location the kit leaves from, then **Check out kit to me**. The kit
   leaves as one transaction while each tool still gets its own ledger line.
   The single-holder rule holds per unit: one tool, one person.
4. To bring it back, open the same screen and tap **Return the kit**. Every
   outstanding unit closes together.

A short kit can still go out, because the field sometimes has to take what is
on the shelf. The button says so, and the availability lines tell you what you
are leaving with.

## Use a part on a ticket

1. Open the ticket and find the **Parts & shipping** card.
2. Tap **Use a part**, scan the item (or search the catalog), and pick the
   location it comes from.
3. Tap **Use from stock**. The ticket anchors the movement, so the ledger row
   points at the job. If a timer is running, the card says so and uses the same
   ticket.

## Move stock between the van and a shelf

1. Open **Move stock** from Today, or from the **Move** action on a stock item
   or location.
2. Choose **Transfer**.
3. Scan the location it leaves, then the location it arrives at, then the item
   or its serial. Scan order does not matter for the item. The two locations
   fill from first to second.
4. Set the quantity for loose stock, or scan the serialized unit itself, then
   confirm.

## Confirm a shipment

1. Open the ticket that expects the goods. The **Parts & shipping** card lists
   movements, including shipments that left the stockroom with a carrier and a
   tracking number.
2. Tap **Confirm** on a shipped line to open **Receive** with the ticket and
   the item already filled in.
3. Scan each arriving unit to confirm it, pick the location it lands in, then
   tap **Confirm receipt**. Serialized stock moves one unit per scan.

## Look up availability

Open **Stock** from Today or More. Search by name, SKU, or part number. Each
row shows what is on hand per location and, for tools, how many are available
and how many are out. The screen reads the device cache, so it works with no
signal and says when it is showing the last sync.

## What the lock screen shows

Starting a timer on a ticket mirrors it outside the app: a Live Activity on
iOS (Dynamic Island and lock screen) and an ongoing notification on Android,
both fed by the same server session as the timer bar. The bar carries a small
**LIVE** marker while the mirror is on. Turn it off under **More → Field →
Live timer on the lock screen**. The in-app bar keeps working either way.

The device captures the activity's push token, but the server has no
token-registration route yet, so the lock screen updates while the app runs
and not from a background push. The status screen under **More → Field**
reports the token and this limit.

## When something goes wrong

- **"That record no longer exists."** The item, location, or ticket was
  archived or moved out of your visibility. Refresh and scan again.
- **"This conflicts with an existing change."** Someone else touched the same
  record. Refresh the screen and repeat the movement.
- **A movement shows as queued.** You are offline. The write is on the device
  with an idempotency key and replays once when the connection returns. The
  queue inspector under **More → Dev tools** shows the trail.
- **The camera does not open.** The device has no camera permission, or you
  are on a simulator. Type the code instead. The resolver is the same.
