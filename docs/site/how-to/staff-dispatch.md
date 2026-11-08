---
description: "The Dispatch board (/dispatch) gives dispatchers and technicians a single view of who is booked where, which tickets are waiting for an owner, and how much…"
---

# How to dispatch from the board

> **Audience:** both — dispatch is the dispatcher's surface and every
> technician's morning board read.

The **Dispatch** board (`/dispatch`) gives dispatchers and technicians a single view of who is booked where, which tickets are waiting for an owner, and how much drive time sits between on-site visits.

## Prerequisites

- A signed-in session with **Dispatch** (`/dispatch`) open.
- Team members assigned to your service boards (such as `Service Desk` or `Field Service`).

## 1. Read the dispatch board layout

Reading from left to right:

- **Team Roster:** Lists every technician along with their booked hours for the day (`0:00 / 8h` capacity bar) and utilization percentage.
- **Day Grid (`06:00`–`19:00`):** Shows each technician's timeline with booked appointments, travel time blocks, and ticket SLA deadline markers.
- **Unassigned Queue (right column):** Tickets that still need an owner or a scheduled slot. Click the `P1`–`P4` chips at the top of the queue to filter by priority.
- **Status Footer (bottom bar):** Quick toggles for **Queue**, **Conflicts**, and live team **Presence**, plus the **Auto-assign** indicator (`Off` by default) and the active timezone axis.

## 2. Assign a ticket to a technician's schedule

To schedule a job:

1. Find the ticket in the **Unassigned** queue on the right.
2. Drag the ticket card onto a technician's row at the hour you want them to work it (or use the card's **Move** menu if you prefer working from the keyboard).
3. The booking snaps into place on the timeline alongside any existing appointments and SLA deadline chips for that day.

## 3. Spot and fix scheduling conflicts

The bottom bar helps you catch problems before a technician is double-booked:

- **Queue:** Shows how many unassigned tickets are still waiting.
- **Conflicts:** Highlights any overlapping bookings on a technician's timeline. Click the conflict indicator to jump to the overlap and drag one of the appointments to an open window.
- **Presence:** Shows which teammates are currently active in the workspace.
- **Auto-assign:** Stays **Off** unless your administrator explicitly enables automatic routing for a scheduling group. Marula never moves tickets behind a dispatcher's back.

## 4. Related scheduling and travel views

Under the Dispatch section you can also open:

- **Scheduling (`/dispatch/scheduling`):** Set up meeting types (duration, buffer minutes before/after, booking window, and active status) and **scheduling groups** (the technician pools used when a client books a slot). See [Set up meeting types and scheduling groups](staff-dispatch-scheduling.md).
- **Planning (`/dispatch/planning`):** Read the seven-day horizon: demand by priority against roster hours, utilization per technician, and skill gaps. See [Plan the week ahead](staff-dispatch-planning.md).
- **Travel (`/dispatch/travel`):** Log trips against tickets, read the 14-day reconciliation, and settle mileage drafts into time entries and expenses. See [Log travel and reconcile mileage](staff-dispatch-travel.md).
- **Client booking links:** Clients can pick an open slot using your published scheduling links or from the **Schedule** tab in the client portal. If someone opens an expired or disabled booking link, Marula shows a clear `"This scheduling link is not available."` message.

## 5. Moving from Dispatch to working the ticket

Once a job is on the calendar, the technician opens the ticket in the workbench (or on the mobile app's **Today** screen) to run the timer and post notes. See [How to work a ticket](staff-work-a-ticket.md) and [How to run a service day from the phone](technician-service-day.md).

## What you learned

- The Dispatch board puts technician capacity, the `06:00`–`19:00` timeline, the unassigned queue, and double-booking conflict alerts on one screen.
- You can schedule work by dragging a ticket onto a slot or by using the keyboard **Move** menu.
- Appointment types and technician pools live under `/dispatch/scheduling`.
