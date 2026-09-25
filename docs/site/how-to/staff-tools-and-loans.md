# How to check tools out, and how to run client loans

> **Audience:** both — technicians check tools in and out from the van;
> admins and account managers issue loans to clients.

Tools are stock items flagged as tools: serialized, individually tracked,
and always answerable to "who has this unit right now". Loans are the
client-facing sibling: equipment that leaves with a client under an
agreement.

## Prerequisites

- Tools exist as serialized stock items with **This is a tool** ticked, and
  at least one unit is received into a location.
- For loans: the client exists, and the equipment either lives in stock or
  is an asset in the client's fleet.

## 1. Check tools out

Open **Stock → Tools** and press **Check out tools**. Four facts make a
checkout:

- **Holder** — the person responsible for the units. It defaults to you,
  which is the common case when a technician loads a van.
- **From** — the location the unit leaves.
- **Due** — the date the tools are expected back.
- **What** — individual tools, or a kit (toolbox) checked out as one
  transaction.

Picker note: when you add a tool without naming a serial, the server picks
the oldest in-stock unit at that location. Name the serial when the exact
unit matters.

A kit shows its readiness before you submit: each member item lists
required units against available units at the chosen location. A kit that
cannot go out says which member is short.

One tool has one holder at a time. If someone else already holds a unit,
the checkout is refused and the message names the unit.

## 2. See who has what

The **Who has what** table lists every serialized tool and device with its
current state: in stock, checked out, on loan, or out. Each row carries the
holder, the location or client it sits with, its due date, and an overdue
tag once the due date passes. Overdue is derived from the due date at read
time, so it cannot go stale.

Use the chips above the table to narrow to overdue units when chasing
returns.

## 3. Check tools back in

Press **Check in** on a row, or open a checkout and return it in full or in
part. Each returned line takes a condition and a note. Conditions are good, fair,
or damaged. The destination defaults to the checkout's own location; pick another
one when the tools come back somewhere else.

The check-in writes its ledger row, the unit flips back to in stock, and
the checkout closes. Parts of a multi-tool checkout can return on different
days without closing the whole transaction.

## 4. Issue a client loan

Open **Stock → Loans** and press **Issue loan**. Choose the client, the
expected return date, and the items. A stock-sourced item rides the ledger:
choose the serial when the item is serialized. When the issued device does
not come from stock, the loan line records the device from the client's
fleet instead.

Two optional facts sharpen the record:

- **Agreement reference** — the contract or paperwork number.
- **Lease terms** — when the loan is a lease, set the cycle, the amount,
  and the currency. The issued device carries those terms on its lifecycle
  record, and the return closes the rental window at the actual return
  date.

Issuing a stock-sourced loan also creates the device in the client's fleet
with its serial as an identity key. When a later discovery feed reports the
same serial, the record converges onto the same device instead of creating
a twin.

## 5. Return a loan

Press **Return** on a register row. Each line takes a condition and a note,
and any stock-sourced line needs a destination location. The return writes
its ledger leg, re-arms the unit to in stock, and closes the rental window
for leased devices.

The register filters by client, so the per-client "on loan" view is the
client filter plus the open status: everything that client still holds,
with expected-return badges and overdue marks.
