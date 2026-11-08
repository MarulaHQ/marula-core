---
description: "Create the meeting types and availability pools that back ticket scheduling links: duration, buffer, booking window, assignee scope and the auto-email."
---

# How to set up meeting types and scheduling groups

> **Audience:** staff — a dispatcher keeps the bookable templates current so the
> links sent from tickets offer the right length, the right people and the right
> window.

The **Scheduling** tab (`/dispatch/scheduling`) holds two things: **meeting
types**, which are the templates a scheduling link is minted from, and
**scheduling groups**, which are availability pools a group-scoped template can
draw on.

This page does not send links. Sending, resending and cancelling a link happens
in the ticket's scheduling panel.

## Prerequisites

- A signed-in session with **Dispatch** access.
- Staff records for everyone a template can assign to. A template cannot point
  at someone who is not in the directory.

## 1. Read the meeting types list

Each row shows the template's name and description, then its settings as badges:

| Badge | Meaning |
|---|---|
| `45m` | How long the booking occupies the calendar. |
| `buffer 15m` | Time reserved before and after the visit for travel and notes. |
| `window 14d` | How far ahead a client can book. |
| `User · Ana` or `Group · Field` | Who the booking is assigned to. |
| `auto-email` or `link only` | Whether the client is emailed when a link is minted. |
| **Active** / **Paused** | Whether the template can be picked when a link is created. |

Paused templates keep their settings and stop being offered. That is the safe way
to retire one.

## 2. Create a meeting type

1. Press **New meeting type**.
2. Fill the dialog:

   | Field | Range | What it decides |
   |---|---|---|
   | **Name** | free text | What staff pick from when they mint a link. |
   | **Description** | free text | The line shown under the name in this list. |
   | **Duration (minutes)** | 5 to 480 | How long the booking blocks the calendar. |
   | **Buffer (minutes)** | 0 to 120 | Padding before and after, so the next visit is not back to back. |
   | **Booking window (days)** | 1 to 365 | The furthest date a client can choose. |
   | **Assignee scope** | user or group | A single person, or a pool that shares availability. |

3. If the scope is **group**, pick the group. If it is **user**, pick the person.
4. Leave **Auto-send the scheduling email when a link is minted** off to mint
   links you send yourself. Turn it on to send straight away, then write the
   subject and body. Keep the body short: it carries the link, and the client
   supplies the time.
5. On an existing template, **Active (bookable)** decides whether it stays in the
   picker. It appears only when you edit.
6. Save. The template appears in the list, ready to be picked from a ticket.

## 3. Keep scheduling groups current

A group is a pool of people, not a rota. A group-scoped template offers the
first open slot across everyone in it, so membership is what decides how much
choice a client gets.

1. In **Scheduling groups**, pick a group from the picker. The count beside each
   name is its current membership.
2. Read the member chips. Each chip has a `×` that removes that person from the
   pool.
3. Pick a person from **Add member** to put them back.
4. Repeat for each pool. A group with no members cannot fill a booking.

When someone changes team or stops taking client appointments, remove them here.
Leaving them in the pool keeps offering their time.

## 4. Change a template without breaking old links

Editing a template changes future links. A link already sent keeps the settings
it was minted with, so a client holding one from last week still sees last week's
duration and window.

If a template is wrong in a way that hurts, pause it and create a corrected one.
That stops fresh links without changing any appointment already booked.

## What this page does not do

- It does not send, resend or cancel a link. That is the ticket's scheduling panel.
- It does not set availability. Working hours and calendars belong to the calendar
  surface; a group is only a list of people.
- It does not book the time. A client picks a slot from the link, and the booking
  lands on the calendar.

## Related

- [Dispatch from the board](staff-dispatch.md) — the day these bookings land on.
- [Plan the week ahead](staff-dispatch-planning.md) — what the week looks like once bookings are in.
- [Set up the portal](admin-portal-setup.md) — the client account a booking sits beside.
