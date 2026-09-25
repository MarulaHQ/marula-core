# How to buy: vendors, purchase orders, receiving, bills

> **Audience:** both — office staff raise and receive orders; admins own
> vendor records and portal links.

Purchasing lives at **Purchasing** (`/purchasing`). A purchase order is the
paper trail from "we need this" to "it is on the shelf and the bill is
posted": vendor, lines, a receipt into stock, and the bill that lands the
cost in the project rollup.

## Prerequisites

- Vendors exist (**Purchasing → Vendors**), because every order names one.
- At least one stock location, because receiving must post the goods
  somewhere.

## 1. Set up vendors

Press **New vendor** on the Vendors page. Beyond the contact block, two
fields matter to the order flow:

- **Default currency** — new orders to this vendor start in it.
- **Portal links** — the vendor's order portal and any tracking pages. The
  links feed the one-click buttons on an order, so the jump to the vendor's
  site is one press and not a search.

Add contacts as a list: name, role, email, phone, and one primary contact.
Saving replaces the list in one write, so edit freely.

## 2. Raise a purchase order

On **Purchasing**, press **New order**. Pick the vendor, confirm the
currency, and add lines. A line carries a description, a quantity at milli
precision, and a unit cost in minor units. Link a line to a stock item when
the goods will be received into stock — that link is what lets the receipt
post ledger movements instead of a loose cost record.

Link a project when the spend belongs to one. You can send the order once
the lines are right: **Send order** moves it from draft to ordered, and the
lines become read-only from then on.

## 3. Receive against the order

Open the order and press **Receive goods**. The dialog asks which stock location received the goods, and how much of
each line arrived. Partial
receipts are normal: enter what actually showed up, and the line records
`partial` until the rest arrives.

For a serialized stock line, type one serial per unit. Each serial becomes
its own ledger row and its own tracked unit, so the order and the shelf
agree unit for unit.

Add the carrier and tracking number while the box is still open. The
tracking number is the value the deeplink panel uses, and it is what the
client-side "where is my order" question resolves to later.

If more arrived than was ordered, the receipt is refused with the line
named. Tick **Allow over-receipt** to acknowledge the overage explicitly;
the line records `over` instead of silently inflating the order.

The dialog also takes scans. Scan a unit's label and its line stages one
unit, or scan a bin code and the put-away location follows. The receipt
waits for you to confirm, so a scan never posts on its own.

A recorded receipt keeps a short strip on the order. Every received stock
line gets a **Dispatch** button. That opens the movement dialog with the
item staged and the ticket picker ready, so goods that arrived for a
specific job reach that ticket in the same sitting.

## 4. Close or cancel an order

- **Close** an order once its receipts are done. Closing with quantity
  still outstanding records a short close, so the gap is a fact on the
  order rather than a missing receipt.
- **Cancel** works while nothing has arrived. After goods arrive, cancel is
  refused — return or adjust the stock instead, so the ledger keeps the
  truth.

## 5. Bills and project costs

The order's **Bill** card records the vendor's invoice: amount, number,
currency. Amounts default to the received total. **Post** the bill to lock
the exchange-rate snapshot (ADR 0002) and make it count.

Posted bills roll up to the project the order is linked to. Open the
project and the cost strip shows purchase cost against budget, with each
posted bill listed and linked back to its order.
