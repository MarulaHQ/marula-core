# How to set up labels and label templates

> **Audience:** admin — you own the org's label stock, template library,
> and the QR codes your team scans. Staff print labels from records and
> scan them; they do not edit templates.

## Prerequisites

- A signed-in session on a deployed stack, with an owner or admin role
  (`admin:edit`).
- At least one label template per kind of label the team prints. You can
  create them before any stock exists — the designer previews with sample
  values.
- For direct Zebra printing: the desktop app on the workstation that sits
  next to the printer, and the printer's network address.

## Where label templates live

Settings → Systems → **Labels & scanning**.

A template is typed by PURPOSE, and the purpose decides which fields it
can print and what its QR code points at. Four purposes ship:

| Purpose | Prints | QR points at |
|---|---|---|
| Stock item | Name, SKU, part number, org | the stock item |
| Location bin | Name, location code, org | the location |
| Serialized tool / unit | Name, SKU, serial, current location, org | that physical unit |
| Loan device | Name, asset tag, serial, org | the device asset |

The QR never carries free text. It always carries the record's deep link
(`meridian://…`), which is what the scan bar resolves back to the record —
so a label and the thing it names cannot drift.

## Create a template

1. Open Settings → Labels & scanning and press **New template**.
2. Name it for the shelf, not the software: "Serialized tool — 2×1".
3. Pick the purpose. The printed-lines list below narrows to the fields
   that purpose can actually print.
4. Pick the label stock. Common Zebra thermal sizes and DYMO LabelWriter
   sizes are presets; **Custom size** takes width and height in
   millimetres. The preview is drawn at the real physical size.
5. Choose the printer resolution — 203 dpi for most Zebra thermal
   printers, 300 or 600 for the models that advertise it.
6. Under **Code128 encodes**, pick which token the barcode carries
   (SKU, part number, serial, location code, or asset tag), or choose
   **No barcode**. The QR toggle adds the deep-link QR.
7. Add up to six printed lines and order them with the arrow buttons. The
   first line is the largest. If the stock is too short for the lines you
   picked, the preview says how many will not print — shorten the list or
   move to a taller label.
8. Press **Create template**. Mark one template per purpose as
   **Default** — that is the one the print button on records uses.

The preview updates as you edit and renders through the same engine that
produces the printed label, so what you see is what the printer gets.

## Print a test label

- **Print test label** opens the browser print dialog with the sample
  label at its exact size. Use it to confirm the dialog's page size and
  orientation before a roll of stock is at stake.
- **Send ZPL to printer** sends the raw ZPL the core rendered to a
  networked Zebra printer. It needs the desktop app; a plain browser says
  so instead of pretending, and the print dialog remains the fallback.

The printer address (`10.0.0.9` or `host:port`, default port 9100) is
stored on the workstation, not in the org. A printer is desk hardware —
one office's address is noise in another's settings.

## Print a label from a record

Records that have a label carry a **Print label** button (for example a
device asset, a stock item, a location, or a serialized tool). It uses the
default template for that record's purpose, fills in the record's real
values, and offers the same two targets. With no default template set, it
links back here instead of guessing at a layout.

## What your team will scan

Every printed QR resolves to exactly one record — scan it in Settings →
Labels & scanning and the bar names what it found. That same resolution
rules the operational surfaces: a scan on a ticket fills the parts line,
a scan during receiving matches the purchase-order line, a scan of a tool
unit picks it for check-out. The details are in
[How to scan barcodes and QR codes](staff-scanning.md).

## When a label does not scan

- Print the test label to plain paper first: the barcode band and the QR
  must both be crisp. A blurry Code128 from a tired thermal head is the
  usual reason a scan "doesn't work".
- If the QR scans but resolves to the wrong record, the label was printed
  from a stale template or another entity — re-render it from the record
  rather than reprinting the old label.

## Where to go from here

- Staff-side scanning and what each surface does with a scan:
  [How to scan barcodes and QR codes](staff-scanning.md).
- The inventory quantities and movement ledger behind every label:
  the [self-host reference spine](../reference/index.md) points at the
  read models that answer where stock is.
