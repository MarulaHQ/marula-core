---
description: "Record trips against tickets, check the 14-day mileage reconciliation, merge duplicates, and turn travel drafts into time entries and expenses."
---

# How to log travel and reconcile mileage

> **Audience:** staff — you record the trip, the dispatcher checks the numbers,
> and the office turns them into billable time and reimbursable expenses.

The **Travel** tab (`/dispatch/travel`) holds two views of the same trips: the
**Drafts** you can still edit, and the **Reconciliation** that shows whether the
last two weeks add up. A submitted trip becomes a time entry and stays where it
landed.

## Prerequisites

- A signed-in session with **Dispatch** access.
- A mileage rate and a reimbursement decision on the organization. Without them
  the reconciliation still totals distance and time, but the money columns have
  nothing to apply. See [Turn on travel time and mileage reconciliation](admin-routing-travel.md).

## 1. Know what a draft is

A draft is a trip waiting to be confirmed. It carries a ticket, a start and end
time, a distance in kilometres or miles, an optional vehicle, a note, and
whether the client is billed for the drive.

Drafts are not billed and do not appear in approvals. Submitting one turns it
into a time entry and makes it immutable here.

## 2. Add a trip

1. Open **Travel** and stay on the **Drafts** view.
2. Press **New travel draft**.
3. Fill the drawer:

   | Field | What to enter |
   |---|---|
   | **Ticket** | The ticket this drive belongs to. A trip with no ticket does not start a mileage record. |
   | **Started** / **Ended** | The drive, as it happened. Duration and time charged come from these two. |
   | **Distance** | The distance driven, with the unit beside it. Change it when the road disagrees with the estimate. |
   | **Vehicle** | The vehicle used, when the org tracks more than one. |
   | **Notes** | Anything the office needs when it reads the row. |
   | **Billable to the client** | Whether the drive goes on the client's invoice. |

4. Press **Save draft**. The row appears in the table with the **Draft** pill.

## 3. Work the drafts table

The status filter at the top switches between **All**, **Draft** and **Sent**,
with a count on each. The table lists technician, date, route, minutes, status
and note, with a totals row at the bottom for trips, distance and minutes.

Each draft row has three actions:

- **Edit** reopens the drawer with the row's values.
- **Submit trip** turns the draft into a time entry. The row changes to
  **Submitted** and the editing actions go away.
- **Delete** removes a draft. It is not offered once the trip is submitted.

## 4. Handle duplicates before submitting

When two drafts describe the same drive, the row carries a **Possible duplicate**
pill and a **Merge** link. Merging keeps one trip and drops the other, so read
both rows before you press it.

## 5. Turn a submitted trip into an expense

A submitted row with no expense offers **Expense**. Pressing it records the
reimbursable side of the trip and the row changes to **Expensed**. A row that
already has an expense shows the pill instead of the button.

## 6. Read the reconciliation

Switch to **Reconciliation** for the last 14 days. The page compares **planned**
travel from the board against **logged** travel on time entries, and bars above
the table show the two side by side per day. A difference of more than fifteen
minutes is worth a look before the week is billed.

The table has one row per technician:

| Column | Meaning |
|---|---|
| **Trips** | Drafts for that person, with the number submitted beside it when there are any. |
| **Distance** | Total distance, in the org's unit. |
| **Time** | Total drive minutes. |
| **Expenses** | Amount already recorded as expenses. |
| **Reimbursable** | What the technician is owed at the org's mileage rate. |
| **Rate** | The rate applied per unit. The total row says whether technicians are reimbursed at all. |

The total row adds the columns. Read a row with many minutes and almost no
distance as a stop that was never closed, and a row with distance and no time as
a trip entered after the fact.

## What this page does not do

- It does not plan the route. The distance is a number you enter and can correct,
  so the row is only as good as what was recorded.
- It does not edit a submitted trip. Corrections go through the time entry on
  the ticket.
- It does not change the mileage rate. That is an org setting.

## Related

- [Dispatch from the board](staff-dispatch.md) — where drive time shows up as a block on the day.
- [Run a service day from the phone](technician-service-day.md) — the technician logging the same trip from the van.
- [Turn on travel time and mileage reconciliation](admin-routing-travel.md) — routing provider, day start, rate and reimbursement.
