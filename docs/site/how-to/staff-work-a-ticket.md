---
description: "Working a ticket in Marula happens on one screen so you don't lose your place switching between notes, your stopwatch, and client documentation."
---

# How to work a ticket

> **Audience:** both — this is the daily loop for MSP staff; admins run it
> too and see it from the settings side.

Working a ticket in Marula happens on one screen so you don't lose your place switching between notes, your stopwatch, and client documentation. You pick a ticket from your queue, start the timer on the bottom work bar, write your customer reply or internal tech note, and stop the timer to log the time entry automatically.

## Prerequisites

- A signed-in session. If you are practicing on a fresh install, see [Your first ticket](../tutorials/first-ticket.md) to sign in to the demo org (`owner@demo.local` / `demo-password-123`).
- Membership on the ticket board you want to work from (`Service Desk`, `Field Service`, etc.).
- Optional: if your admin has granted your role AI assistance, the right-hand Assistant panel will show suggested next steps. If not, the ticket workbench runs cleanly without it.

## 1. Pick your next ticket

You have two ways to grab work:

- **Today (`/today`):** Your morning dashboard showing summary counters (**Overdue**, **Due today**, and **Approvals**) above the active ticket queue with `P1`–`P4` priority badges, assigned tech initials, board columns, and SLA status pills.
- **Tickets (`/tickets`):** The full searchable ticket ledger with stage tabs (**Open**, **In progress**, **Waiting**, **Closed**, and **All**) when you need to filter across the whole desk.

## 2. Read the three-pane ticket workbench

Click a ticket row to open the workbench:

- **Left pane — Ticket Context:** Client name, site, board, current status, live SLA countdown ring, and linked Knowledge Base articles that match the ticket.
- **Center pane — Activity:** The chronological timeline of emails, internal notes, parts used, and time entries, with the reply composer at the top. You can also switch tabs along the top of the feed (**Activity**, **Details**, **Related**, **Time Entries**, **Files**, and **Notes**).
- **Right pane — Assistant:** Relevant documentation sources and suggested next steps (when AI assistance is enabled for your role).

In the top header you can change the status column, check the SLA ring, click **Close**, **Escalate**, or **Link**, pop the ticket into its own window (`⧉`), or flip the **Classic** toggle if you prefer the multi-pod layout (**Tickets**, **Detail**, and **Timeline** pods with custom pod arrangement).

## 3. Write a client reply or an internal note

Click into the composer at the top of **Activity**:

- **Replying to the customer:** Leave the mode on **Client reply**. What you send goes out by email (when outbound email is enabled) and appears on the client's portal request page.
- **Writing a private note for your team:** Flip the selector to **Internal note** before posting. Internal notes are highlighted with an internal tint and are never visible to the client in emails or on the portal.
- **Speeding up common replies:** Use **Attach** to add screenshots or logs, **Template** / **Snippets** to insert a canned response, **Dictate** for voice transcription (if enabled), or **AI Assist** to draft a summary.

## 4. Run the timer to record your hours

Look at the dark **work bar** along the bottom of your window:

1. Click **Start timer** on the ticket (or on the work bar). Only one timer can run for you at a time across the entire organization.
2. Do the work and write your notes.
3. Click **Stop** on the work bar when you finish or switch tasks.

Stopping the timer opens the wrap-up prompt with the elapsed minutes and work category prefilled. Confirm it, and your time entry lands directly on the ticket's **Time Entries** tab and in your daily time log.

## 5. Send your hours for approval and billing

Submitted time entries move to **Approvals** (`/approvals`), where a service lead or manager reviews them. Once approved, those hours flow into the next billing verification and invoice run ([see Your first invoice](../tutorials/first-invoice.md)). Once an entry is billed on an issued invoice, it locks permanently.

## What you learned

- Everything for a ticket lives in one three-pane view: **Ticket Context** on the left, **Activity** and composer in the center, and **Assistant** on the right, with the active timer pinned in the bottom work bar.
- Switching the composer to **Internal note** keeps technical notes strictly inside your team.
- Stopping the timer creates the billable time entry right on the ticket.
