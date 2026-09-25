# How to scan barcodes and QR codes

> **Audience:** both — technicians scan on receiving, on tickets, and at
> the tool crib. Admins scan to test templates. Nothing here changes org
> configuration.

## Prerequisites

- A signed-in session on the web app.
- A label to scan. Printed labels come from Settings → Labels & scanning
  ([how to set them up](admin-labels.md)). A scanner reads the QR or the
  Code128 band.
- A USB barcode scanner in its default "keyboard wedge" mode, or the
  keyboard itself. No driver or browser extension is involved.

## What a scan means

Every scan resolves to exactly ONE typed record before anything acts on
it. The resolver reads, in this order:

1. A **deep link** — the QR's `meridian://stock-item/…`,
   `meridian://stock-location/…`, `meridian://stock-serial/…`, or
   `meridian://device-asset/…`.
2. A **serial number** of a tracked unit.
3. A stock item's **SKU or part number**.
4. A location's **code**.
5. A device's **asset tag**.

The first match wins, case does not matter, and nothing leaves your org.
When a code matches more than one record, the bar lists the candidates
and waits — a scan never guesses between two physical items. When nothing
matches, it says so and keeps the text you scanned in view.

## Scan on a ticket (parts and shipping)

Open a ticket and press **Parts & shipping**. The scan bar sits above the
form.

1. Scan a stock item or a serialized unit.
2. The item is staged — and for a unit, its serial too. Choose the action:
   **Use** consumes the part on the job, **Ship out** sends it to the
   client, **Receive** returns it to stock.
3. Check the numbers, then press **Record movement**. The scan fills the
   form and stops there. It never records on its own.

A location or a device asset does not apply on this surface, and the bar
tells you that instead of filling something wrong. The movement dialog on
the **Stock** board also takes a location scan, and sets the bin from it.

The classic workspace's **Money** tab keeps its own scan bar above the
cost-line form. There a scan fills the invoice line: description, a
quantity of 1, and the item's last recorded unit cost when your role may
read costs. It waits for **Add line**.

## Scan in the stockroom

The **Stock** board carries its own scan bar. Scan a part or a unit and the
movement dialog opens with it staged. Pick the ticket from the picker, then
record. The stockroom can attach goods to a job without opening the ticket.

## Scan while receiving

On a purchase order, open **Receive goods** and scan a unit's label. Its
line stages one unit. Scan a bin code and the location follows. The receipt
posts only when **Record receipt** is pressed.

## Scan at the tool crib

On **Stock → Tools**, scan a unit to check it out. If the unit is already
out, the scan finds its open checkout and opens the check-in instead.

## Scanning while you type

The bar listens for the scanner's rhythm: a burst of characters with
gaps under 55 ms, ended by Enter. That is what a wedge scanner produces
and what typing does not.

If a scan arrives while your cursor is in another field, the bar takes
the characters back out of that field, keeps your text, swallows the
Enter so no form submits behind your back, and routes the scan. Focus
stays exactly where you left it. If you paste or type a code by hand,
press Enter in the scan field. The same resolution runs, with no burst
timing involved.

## Scan in Settings → Labels & scanning

The bar at the top of that page is the test bench. Scan a freshly printed
label there and the result line names the record it resolved to — the
quickest way to prove a template's QR is good before printing a roll. It
also accepts a typed SKU, serial, location code, or asset tag, which is
handy when the only thing at hand is a record's number.

## When a scan does nothing

- **Nothing in the bar** — the scanner may be in a non-keyboard mode
  (some ship with a prefix/suffix configuration card). The bar expects
  the payload, then Enter.
- **"Nothing matches"** — the record exists in another org, or the code
  was mistyped by hand. Scan again and the bar echoes what it read.
- **"…does not apply here"** — the code resolved to a kind this surface
  cannot use. That sentence names what the surface accepts.
- **Two candidates listed** — two records share that code (common with
  part numbers). Pick the exact one. The choice is not remembered.

## Where to go from here

- Binding a scanner to a workstation and printing the labels it reads:
  [How to set up labels and label templates](admin-labels.md).
- Where scanned stock lands and how quantities move:
  [Work a ticket](staff-work-a-ticket.md) covers the parts-and-billing
  side of the same loop.
