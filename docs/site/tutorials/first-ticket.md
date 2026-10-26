---
description: "When a client calls or emails with a problem, you shouldn't have to bounce between a ticket tab, a separate stopwatch popup, and a timecard sheet at 5 PM."
---

# Tutorial: your first ticket (in the UI)

> **Audience:** both — MSP staff learning the workbench and admins learning
> the product. No API knowledge required.

When a client calls or emails with a problem, you shouldn't have to bounce between a ticket tab, a separate stopwatch popup, and a timecard sheet at 5 PM. In Marula, you open the ticket, start the clock at the bottom of your screen, type what you did, and stop the clock. Stopping the timer logs your billable time entry on the spot.

> "A ticket should take one screen, not three. Notes, timer, conversation,
> calendar live in the same view, because they are the same job."
> — [MANIFESTO.md](../../../MANIFESTO.md), quoted verbatim

## Prerequisites

- A running Marula workspace in your browser (see [self-host reference](../reference/self-host-local-stack.md) if you started Docker Compose yourself), or the desktop test app ([ADR 0005](../../adr/0005-local-desktop-test-app.md)).
- The **seeded demo org**: sign in as `owner@demo.local` with password `demo-password-123`. It comes preloaded with 16 open tickets, two service boards (`Service Desk` and `Field Service`), sample clients, and existing time entries so you can practice on realistic work right away.

## 1. Check your morning queue on Today

After you sign in, you land on **Today** (`/today`). Think of this page as your morning standup sheet:

- Top summary counters show what needs attention first: overdue tickets, tickets due today, and items waiting for approval.
- Below the counters sits the active queue (`16 open` in the demo org), showing each ticket's `P1`–`P4` priority badge, assigned technician initials, board column, and SLA countdown pill.
- Along the bottom edge of the window sits the dark **work bar**. Right now it reads `no active timer — the work bar tracks it`. Whenever you start timing a ticket, that bar stays visible even if you click over to another page to look up a password or check stock.

## 2. Open a ticket in the workbench

Click any ticket row in the queue (or click a priority filter chip like `P1` first). The ticket opens in a three-column workbench so you can read the background and reply without scrolling away:

- **Left column — Ticket Context:** Who the client is, which board and status column the ticket sits in, the live SLA countdown ring, and links to matching Knowledge Base runbooks.
- **Center column — Activity:** The full chronological thread of client emails, internal technician notes, and logged time entries, with the reply and note composer pinned at the top.
- **Right column — Assistant & Related:** Relevant documentation sources and, if your admin has enabled AI assistance for your role, a suggested next step that waits for your review.

Along the top header you have quick action buttons to change status, **Close**, **Escalate**, **Link** related records, or flip the **Classic** layout toggle.

## 3. Start the timer, write your note, and stop the clock

1. Start the ticket timer from the bottom work bar (or press `t`). Only one timer can run for you at a time; if you were already timing another ticket, Marula pauses that one automatically so your hours never overlap.
2. Click into the **Activity** composer. If you are writing a reply the customer should read, leave the mode on **Client reply**. If you are recording troubleshooting steps or serial numbers for your teammates only, switch the selector to **Internal note**.
3. Type your update and post it.
4. When you finish the work, click **Stop** on the bottom work bar.

The moment you stop the timer, Marula creates a time entry attached to this ticket and lists it under the **Time Entries** tab in the Activity feed. There is no separate timesheet screen to fill out later.

## 4. See where your hours go for billing

Open **Approvals** (`/approvals`) from the left navigation. The time entry you just stopped is waiting in the approval queue for a service lead or manager to review.

Once a manager approves a time entry, the billing engine picks it up on the next invoice run. Once that invoice is issued to the client, the time entry locks permanently so historical bills never shift under your feet.

## What you learned

- **Today** (`/today`) and **Tickets** (`/tickets`) show what needs work next, while the ticket workbench keeps client details, notes, and the timer on a single screen.
- Stopping the work-bar timer logs your time entry directly on the ticket timeline.
- Every board column maps to a core stage (`open`, `in_progress`, `waiting`, or `closed`) that drives the SLA clock automatically ([see why customizations are data](../explanation/customizations-are-data.md)).

## Where to next

- Want to run the exact same workflow from a terminal or script? Try the [API edition (`first-ticket-api.md`)](first-ticket-api.md) using `curl`.
- Scheduling technicians onto the calendar? Read [Dispatch from the board](../how-to/staff-dispatch.md).
- Looking for a quick daily checklist for technicians? Open [Work a ticket](../how-to/staff-work-a-ticket.md).
