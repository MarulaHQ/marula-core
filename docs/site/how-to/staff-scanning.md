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

## Scan on a ticket (parts)

Open the ticket's Money tab. The scan bar sits above the cost-line form.

1. Scan a stock item or a serialized unit.
2. The form fills in: the item's name, a quantity of 1, and its last
   recorded unit cost when your role may read costs. A serialized unit
   carries its serial into the description.
3. Check the numbers, then press **Add line**. The scan fills the form
   and stops there. It never submits on its own.

A location or a device asset does not apply on this surface, and the bar
tells you that instead of filling something wrong.

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
