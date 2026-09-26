# How to set up toolboxes and dispatch defaults

> **Audience:** admin — toolboxes and dispatch mappings are organization
> configuration.

**Settings → Toolboxes & day pack** is the config home for toolboxes. It
holds the kits themselves, the tools inside them, the mappings the day
pack reads, and the box tags that scan back to a kit.

## Prerequisites

- The tools exist as stock items with **This is a tool** ticked, and at
  least one unit is received.
- Boards, ticket types, or ticket categories exist, since every dispatch
  rule names one.
- For box tags, a label template whose purpose is **Toolbox** exists
  under **Settings → Labels & scanning**. The template's QR always
  carries the kit's deep link. A Code128 carries the kit code.

## 1. Create a toolbox

1. Open **Settings → Toolboxes & day pack**.
2. Under **New toolbox**, type a name ("Network field kit") and a short
   code ("KIT-NET-01"). The code is what you stencil on the box, and it
   is what a code scan resolves.
3. Press **Create toolbox**.

The kit appears in the list on the left. Selecting it opens its editor.

## 2. Set membership

1. In the editor, pick a tool under **Add a tool** and set the quantity.
   Most kits hold one of each tool, so the default of 1 is usually right.
2. Press **Add member**. Repeat for every tool in the box.
3. Remove a member with the × on its row. Removing a member never touches
   stock. It changes what the kit checks out as a unit.

Readiness is per member. **In stock** means enough serial units exist for
a checkout. "1 of 2" means the kit is short until the other unit comes
back. The editor header summarizes the whole kit.

## 3. Print and scan the box tag

1. Select the kit and press **Print label**.
2. Choose the Toolbox template. The QR encodes
   `meridian://tool-kit/<id>`. If the template carries a Code128, that
   encodes the kit code.
3. Scan the printed tag with the scan bar at the top of the page. The
   editor selects that kit, which proves the tag resolves before it goes
   on a box.

The same tag resolves on any surface that accepts toolbox scans. A scan
never guesses. If two kits share a code, you get a candidate list.

## 4. Map a job type to items or a kit

Dispatch defaults are the typed org mapping behind the day pack's
recommendations. A rule has a scope and a target.

- **Scope** — ticket type, ticket category, or board.
- **Target** — one or more recommended items, or one default kit.

Examples:

- Ticket category **Network** recommends **Cable tester** ×1 and
  **RJ45 ends** ×4.
- Board **Field service** dispatches the **Network field kit** as the
  default kit.

To add one, choose the scope kind, pick the row, choose the target kind,
pick the item or kit, set the quantity, and press **Add rule**.

Rules apply to every scheduled ticket whose scope matches. A pin on the
ticket itself always wins over a rule for the same item. A checklist
mention only suggests an item when nothing else already covers it.

Quantity semantics differ by item kind, because the hardware does.

- **Tools** are units. The pack needs one available unit no matter how
  many stops use it.
- **Consumables** add up across the day's stops. Four ends for one stop
  and four for another means eight in the van.

Turn a rule off with **Turn off** when a category is being reworked. The
row stays for later. Remove it with × when it is wrong.

## Notes and limits

- Dispatch rules are org-wide. There is no per-technician override yet.
  A pin on the ticket is the supported exception.
- Availability on the day pack is computed live from location levels and
  serial status. A rule never reserves stock.
- Toolbox tags use the same renderer and printer path as every other
  label, so anything that prints a stock label prints a box tag.
