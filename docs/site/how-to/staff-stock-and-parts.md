# How to run stock and use parts

> **Audience:** both — technicians receive and consume parts at the job;
> admins own the item list and the reorder thresholds.

Stock lives at **Stock** (`/stock`). The board reads the same on-hand rows
the low-stock alerts use, so the numbers you see here are the numbers
automations act on. The configuration-item ledger at `/inventory` is a
different surface: it tracks identity and merge state, not quantities.

## Prerequisites

- A signed-in session with commercial view permission. Cost fields need
  `financial:read`; without it the cost columns render as `—` and every
  other column still works.
- At least one location. Stock must land somewhere before it can be
  received.

## 1. Create the items you track

Open **Stock → New item**. Name is the only required field. Two tracking
kinds exist:

- **Consumable** — counted in bulk (a box of RJ45 ends, a spool of cable).
- **Serialized** — one row per physical unit, so every movement names the
  unit it moved.

Set the unit (each, box, or meter) once and keep it consistent. Quantities
are stored in thousandths of that unit, and every ledger row reports the
same unit. A catalog link is optional and only supplies pricing context.

Tick **This is a tool** for anything checked out to a person rather than
consumed. Tool items are serialized by definition, and they appear on
**Stock → Tools**.

## 2. Receive stock

Press **Receive stock** on the board (or **Receive** on a row). Pick the
item, the destination location, the quantity, and optionally the unit
cost. The receipt posts one append-only ledger row immediately, and the
level at that location grows in the same commit.

For a serialized item, type one serial number per line. Each line becomes
its own ledger row and its own unit — three lines receive three units.

When the cost currency differs from the org base currency, the receipt
locks the exchange rate onto the ledger row (ADR 0002). Later screens
convert for display; the locked rate never changes.

## 3. Parts and shipping on a ticket

Open a ticket and press **Parts & shipping** in the header action row. The
panel completes the physical side of the job without a trip to `/stock`:

- **Use** consumes parts on site. Search the item by name, SKU, or part
  number, or scan its label. Then pick the source location and the
  quantity. The dialog shows what is on hand at that location before you
  submit, and it refuses a quantity larger than the shelf holds.
- **Ship out** sends goods to the ticket's client. Carrier and tracking
  reference are optional. The receipt that follows carries a **Print
  label** button for the shipped item, or for its serial unit.
- **Receive** returns unused or replaced parts to stock. The movement is
  anchored to the ticket, so a return shows on the ticket's timeline.

One submit writes the ledger row, anchors the movement to the ticket, and
the movement appears on the activity timeline under the **Parts** filter
chip. Nothing else needs logging.

## 4. Dispatch stock to a ticket from the stockroom

Open **Stock**. Scan a part or unit into the scan bar and the movement
dialog opens with that item staged. You can also press **Dispatch to
ticket** in the header, or **Use** on a row.

Pick the ticket from the picker, choose the source location, and record.
The ledger row carries the ticket anchor, and that ticket's timeline gains
the entry. The ticket itself never opens.

## 5. Move, adjust, or return stock

Every other physical action runs through the same movement dialog:

- **Transfer** shrinks one location and grows another. A serialized
  transfer moves one unit and picks that unit explicitly.
- **Adjust** records a signed correction (`−2` after a stocktake found
  two missing). Adjustments are refused for serialized items — move the
  unit instead, so its history stays physical.
- **Ship out** sends stock to a client or a job site, with an optional
  carrier and tracking number. The tracking number is what the receiving
  side of a purchase order can be deeplinked to.
- **Receive** brings stock back in: a client return, a restock after a
  job, or a delivery that arrived outside a purchase order. Add the ticket
  when the return belongs to a job, so the ticket timeline carries it.

## 6. Set low-stock thresholds

Open an item and find its **Quantities** table. Press **Thresholds** on a
location row and set the minimum and reorder point. The row's low tag, the
banner on the stock board, and the `stock.level_low` automation trigger all
read those numbers. Clearing a field removes the threshold.

Low stock notifies the people who work from the location. When a movement
is tied to a ticket, the alert carries the ticket so ticket-scoped
automation rules can fire.

## 7. Manage locations

**Stock → Locations** holds every place stock sits, from a warehouse down
to a van shelf. Van and truck locations can be assigned to a person. A
location can nest inside another (a shelf inside a warehouse), so roll-ups
stay readable. Archiving a location hides it from
pick lists while its ledger history remains.
