# Tutorial: your first ticket (in the UI)

> **Audience:** both — MSP staff learning the workbench and admins learning
> the product. No API knowledge required.

## Prerequisites

- A running stack with the console reachable (see
  [self-host reference](../reference/self-host-local-stack.md) if you
  brought up the compose stack yourself) — or the desktop test app, which
  seeds the demo org on first run ([ADR
  0005](../../adr/0005-local-desktop-test-app.md)).
- The **seeded demo org**: sign in as `owner@demo.local` with password
  `demo-password-123`. It comes with 16 open tickets, seeded boards
  (Service Desk, Field Service), clients, and time entries — you start in
  a real service desk, not an empty one.

Working a ticket is the center of the product. The console is built so a
ticket is one screen, not three:

> "A ticket should take one screen, not three. Notes, timer, conversation,
> calendar live in the same view, because they are the same job."
> — [MANIFESTO.md](../../../MANIFESTO.md), quoted verbatim

## 1. Land on the work queue

After sign-in you are on `/today` — the shared working dashboard. It shows
the org's live posture as stat widgets (overdue, due-today, approvals),
"16 open", and today's rows with P1–P4 priority chips, assignee initials,
board column, and escalation/progress status chips. A hint at the bottom
of the page points at the work bar: "no active timer — the work bar
tracks it". This queue is your morning, and everything below starts
here.

## 2. Open a ticket

Click any row (or filter by priority chip first). The ticket opens in the
workbench: three panes. Left is **Ticket Context** (client, board, status,
SLA ring, contextual documentation links to relevant KB articles). Center
is **Activity** — the conversation and the composer. Right is the **Agent**
panel with relevant knowledge sources and a recommended next step.

Header controls: status/SLA ring, Close, Escalate, Link, and the Classic
toggle (the legacy workspace is kept side-by-side during the transition).

## 3. Work it: note, timer, time entry

Type an update in the composer — flip on the **Internal note** selector for
anything the client should not read. Attach files, or use AI Assist if your
org has granted it. Timer and composer live together: the timer you started
from the work bar lands as a proper time entry event on the ticket's
Activity feed (see the **Time Entries** tab) the moment you stop it — no
separate time-entry screen, no post-hoc retype of what you did.

That is the whole loop: context in, note in, timer out to a time entry.

## 4. See it become billable

Open **Approvals** (/approvals). Your ticket's time entry awaits in the
queue. Approved time entries are the ones billing consumes — invoiced
entries lock permanently. That is the path from "I worked this" to "the
client is invoiced for it".

## What you learned

- The work queue (`/today`) and the ticket list (`/tickets`) are the two
  launch points. The workbench is where the work happens.
- Notes, timer, time entries, files, and (if granted) AI assistance share
  one composer-centered pane.
- The board's semantic status categories govern every downstream clock
  ([more on why](../explanation/customizations-are-data.md)).

## Where to next

- Prefer the API? This same lesson exists as an [API edition
  (first-ticket-api.md)](first-ticket-api.md) — every step worked with
  `curl` against the real endpoints.
- How a board move works for a dispatcher: [Dispatch from the
  board](../how-to/staff-dispatch.md).
- The full task view for staff: [Work a ticket](../how-to/staff-work-a-ticket.md).
