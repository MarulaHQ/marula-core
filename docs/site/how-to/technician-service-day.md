---
description: "Today is the first screen after sign-in. It reads the same dispatch board the office sees, narrowed to your bookings:"
---

# How to run a service day from the phone

> **Audience:** both — technicians work the day from this page. Dispatchers read
> it to know what the phone shows and when the board updates.

## Start on Today

Today is the first screen after sign-in. It reads the same dispatch board the
office sees, narrowed to your bookings:

| Part of the screen | What it tells you |
| --- | --- |
| Time gutter plus booking cards | Your day in order, with the arrival window on the left |
| State pill | `SCHEDULED`, `IN PROGRESS`, `DONE`, or `CANCELLED` |
| Travel line | Minutes the dispatcher's estimate booked for the drive |
| On-call strip | Shows only when a rotation block covers today or the next two weeks |

Unassigned tickets still waiting for an owner are counted under the title. The
dispatcher moves them onto a board. The count is your cue that more work is
coming.

## Accept a booking

1. Find the booking on Today. `SCHEDULED` is the state to watch for.
2. Tap **Accept job**. The pill flips to `IN PROGRESS` before the server
   answers, so the screen never blocks on signal.
3. When the work is done, tap **Finish**. The pill reads `DONE`.

Both taps write the appointment's state through the same API the dispatch
board uses. A dispatcher looking at the board sees the same change. If the
phone is offline, the write queues and replays when signal returns. The banner
under the schedule says it queued rather than sent.

## Clock in and out

The clock is the ticket's timer, not a private stopwatch.

1. Open the ticket from the board, from My work, or from a booking's ticket
   link.
2. Tap **Clock in**. The card shows a running `HH:MM:SS` count, and the timer
   bar appears across the app.
3. Tap **Pause** when you break for parts, **Resume** when you start again,
   and **Stop** at the end.

One timer runs at a time for the whole org. Starting a timer while another
runs is refused, and the card will say the other ticket holds it. Stopping the
clock writes a time entry with the worked minutes, the billable flag, and an
approval state. The Time section on the ticket lists those entries and the
running total.

## Log the drive

Travel drafts capture the miles between jobs.

1. On Today, tap **Log travel**.
2. Pick the ticket the trip belongs to, then set the start and end time. The
   form opens with the last half hour already filled in.
3. Enter the distance, pick `km` or `mi`, and name the vehicle. Leave
   **Billable** on unless the trip is on your own time.
4. Tap **Save draft**. The draft appears under Travel on Today.
5. Tap **Submit** when the numbers look right.

Submitting converts the draft through the time-entry path, so it lands in the
same approvals queue as the clock. The office travel page shows the same
drafts. A dispatcher can reconcile them against the day's plan. Drafts that
are wrong can be deleted before submit.

## Write the record on site

Notes are comments on the ticket, and photos attach to them.

1. Open the ticket and scroll to Notes.
2. Pick a kind: **Internal** stays in the office, **Client** is visible to the
   customer, **Update** reads as an in-house update.
3. Type what happened and tap **Add note**. With a hardware keyboard,
   `Cmd + Enter` posts the note.

For a photo, tap **Photo**, pick from the library, and the upload stages a
chip above the composer. Add the note and the photo travels with it. Uploading
needs a connection — an offline attempt says so and stages nothing. The note
itself always works offline. It queues and syncs later, and the office sees it
the moment it lands.

## On call

The on-call strip sits above the schedule when a rotation block covers you. It
names the rotation (`Primary`, `Secondary`, and so on) and either the time it
ends or the dates of the next block. When the two-week window holds nothing
for you, the strip is absent, which is the normal week.

## Notifications

The inbox collects mentions, assignments, SLA alerts, stock warnings, and
report deliveries. Each row carries its kind and a way in: **Open ticket →**
when the notification points at one. Tapping a row marks it read. **Mark all
read** clears the batch.

## On a tablet

An iPad at 768 points wide or more gets the two-pane layout. The board shows
the ticket list beside the open ticket. Today shows the schedule rail beside
your work. Selecting a row swaps the detail without leaving the screen, and
Stage Manager resizing crosses back to the single-column phone layout.

A hardware keyboard works the app the way iPadOS expects: `Tab` and the arrow
keys move between controls, `Enter` or `Space` activates the focused control,
and `Cmd + Enter` posts a note from the composer. The rest of the web
shortcuts do not carry over — this shell has four tabs, no command palette,
and no rail to cycle.

## What the office sees, and when

Every write on this screen is a server write with a client idempotency key.
Two things follow from that:

- A dispatcher watching the ticket sees your progress as the writes land —
  timer state first, then notes, then time entries.
- An offline phone keeps working. Notes, timer changes, travel drafts, and
  booking states queue on the device and replay in order on the same entities.
  The queue inspector under **More → Developer tools** lists what is waiting.

Photos are the one exception: the upload needs the wire. A note with a photo
becomes two steps — upload while connected, then the note (which can still
queue behind it).
