# How to dispatch from the board

> **Audience:** both — dispatch is the dispatcher's surface and every
> technician's morning board read.

## Prerequisites

- A signed-in session with **Dispatch** (/dispatch) open.
- Board membership and the coached-out, deployed org (boards like Field
  Service show as a technician world only when the org has people to be
  on it).
- Slots you will be moving into: appointments and ticket-deadline chips
  from the calendar side today (the calendar surface renders both).

## 1. Read the dispatch board

Left to right: the **day-grid per technician** (each row's booked
percentage, a 06:00–19:00 axis). The **TEAM roster** shows each
technician's load (0:00 / 8h markers), and the right **UNASSIGNED**
queue carries ticket work narrowed by the P1–P4 filter chips. Footer: Queue /
Conflicts / Presence, Auto-assign (Off by default), Time zone: UTC axis.

## 2. Move a request onto a person

Drag on the grid (or the Move menu for keyboard-dispatched actions) —
unassigned work onto a technician's slot on the day grid. Appointments
and ticket-deadline chips render per day, so what you move snaps
visibly into whatever the day already holds.

## 3. Conflicts, queue, presence

- **Queue** — the unassigned backlog in its own footer slot, filterable
  by priority chip.
- **Conflicts** — where moves collided (two bookings over one slot).
  Resolve by moving one side.
- **Presence** — realtime presence of the team on the same footer.
- **Auto-assign** remains **Off** unless and until deliberately
  enabled. Nothing moves itself silently.

## 4. The scheduling surfaces beneath

- **/dispatch/scheduling** — meeting types you configure (duration,
  buffer, window, user scope, link-only, Active flag) and scheduling
  groups (the pools auto-assignment reads from).
- **Planning and travel surfaces** (/dispatch/planning, /dispatch/travel)
  carry the day's thin planning views.
- **Public scheduling links** are a calendar-side booking mechanism: the
  standalone logged-out render ("This scheduling link is not available.")
  covers the invalid case. Valid links book normally, and the same
  connect card in Calendar shows where two-way sync connects.

## 5. When the board is where the day starts

Queue → pick the highest priority → move it onto the right tech → next
shuffle. The dispatch board does not replace the ticket workbench. You
still arrive at [the ticket loop](staff-work-a-ticket.md) for the
actual work: notes, timers, approvals.

## What you learned

- The dispatch board is the whole workforce in one day-grid: booked %,
  unassigned queue, conflicts, presence, auto-assign off by default.
- Scheduling groups and meeting types are the config surfaces beneath.
