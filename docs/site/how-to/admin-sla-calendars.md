# How to configure SLA calendars & escalation ladders

> **Audience:** admin — SLA policy is a settings-owner responsibility;
> staff consume the result but cannot bend it.

When a client has an agreement for a 2-hour response on urgent tickets, you don't want the SLA clock ticking at 3 AM on a Sunday unless they pay for 24/7 coverage. In Marula, **business-hours calendars** control when the SLA stopwatch runs, and **escalation ladders** automatically notify leads when a ticket approaches or breaches its deadline.

## Prerequisites

- Admin access to **Settings → SLA** (under **Delivery**).
- The IANA timezone name for each support office or region you cover (for example, `America/New_York` or `Europe/London`).
- Your target response and resolution times for each priority level (`P1` through `P4`).

## 1. How calendars and escalation ladders fit together

The **Settings → SLA** page is built around two rules:

> "Business-hours calendars govern every SLA clock; escalation ladders
> fire once per ticket when consumed budget crosses an offset."

In practice:

- **Calendars** define *when the clock ticks*: your open hours for each day of the week, observed holidays, and timezone. Outside those hours (or when a ticket sits in a `Waiting` status column), the SLA timer pauses automatically.
- **Escalation ladders** define *who gets alerted and when*: threshold offsets against the ticket's SLA budget, firing once per threshold on a ticket.

## 2. Create a business-hours calendar

Click **New calendar** and enter:

1. A descriptive name (for example, `Standard Business Hours — Eastern` or `24x7 Critical Coverage`).
2. The **IANA timezone** (`America/New_York`, `UTC`, etc.). Because the SLA engine calculates deadlines directly from this timezone, double-check that it matches your contractual hours.
3. The open start and end times for each day of the week, plus any company holidays.

## 3. Set priority targets and escalation steps

In the policy section below the calendar, set your response and resolution targets for `P1` through `P4` tickets and configure the escalation offsets that fire as time runs low.

## 4. Test on a live ticket

Open a test ticket on your board and check the **SLA ring** in the ticket header. Move the ticket into a `Waiting` column to verify the countdown pauses, then move it back to `In progress`. When a ticket crosses an escalation threshold, it surfaces at the top of `/today` and breaks through staff notification quiet hours automatically.

## What you learned

- SLA countdowns run only during the open windows on their assigned business-hours calendar and pause automatically when a ticket is waiting on the client.
- Escalation steps fire once per threshold and route into your alert and automation queues ([see Author automation rules](admin-automation-rules.md)).
