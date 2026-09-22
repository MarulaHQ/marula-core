# How to configure SLA calendars & escalation ladders

> **Audience:** admin — SLA policy is a settings-owner responsibility;
> staff consume the result but cannot bend it.

## Prerequisites

- Admin access to **Settings→SLA** (DELIVERY group).
- An IANA timezone string for each calendar you plan to author.
- Outcome you want first (e.g. "P1 must have first response inside two
  business hours"), because each policy level's offsets are expressed from
  it.

## 1. Understand what the page binds to

The surface opens with the two governing sentences:

> "Business-hours calendars govern every SLA clock; escalation ladders
> fire once per ticket when consumed budget crosses an offset."

Two halves that matter day to day:

- **Calendars** answer *when does the clock run* — per-day open windows,
  holidays, timezone.
- **Escalation ladders** answer *who is told and when* — offsets against
  consumed budget, one fire per ticket per offset.

## 2. Create a business-hours calendar

**New calendar**: name, IANA timezone, per-day open windows, holidays.
Everything the clock consumes lives here — the semantics, not just
display, so a wrong timezone silently moves every SLA ring in the product.

## 3. Attach policies & escalation ladders

The lower section embeds a default calendar with per-priority (P1–P4)
offset ladders. Set the offsets you aim for. **Verified rough edge in the
current UI**: P1–P4 offset rows render as `[object Object]m` — a real UI
render bug being tracked for a product lane. Do not try to document or
parse names from that broken render. Set values and verify behavior on
a fresh ticket instead.

## 4. Verify against a real ticket

Open a fresh ticket, watch the SLA ring in the ticket header, and confirm
the escalation fires once, when consumed budget crosses your offset.
Escalations are also surfaced in `/today` and break through notification
quiet hours by design.

## What you learned

- Clocks are calendar-driven, while ladders fire on budget offsets,
  one shot each.
- The ladders feed the alert queue — see [automation and
  alerts](admin-automation-rules.md) for where escalated outcomes route.
