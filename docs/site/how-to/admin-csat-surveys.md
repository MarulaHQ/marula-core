---
description: "Set up satisfaction surveys on ticket resolution, with intervals, link validity and quiet hours."
---

# How to set up CSAT surveys on resolution

> **Audience:** admin — survey policy is an org-wide decision, and the mail
> transport it depends on is an Email-settings decision.

## Prerequisites

- Admin access to **Settings → Surveys**.
- An outbound mail transport configured under **Settings → Email**. With
  transport `none`, nothing below sends — surveys are never queued and the
  CSAT sections on scorecards stay honestly empty.

## One sentence of doctrine

> "One survey per resolved ticket, at most one per client per interval,
> never inside quiet hours, and every link works exactly once."

## 1. Turn surveys on

In **Settings → Surveys**, check *Send a satisfaction survey when a ticket
resolves* and save. From the next resolution on, the worker queues a survey
for the ticket's client.

If mail is not configured, the page says so in plain text and no surveys
are queued. Turn the transport on first; the toggle does not pretend.

## 2. Shape the policy

- **Minimum interval per client** (default 168 hours): a client who already
  received a survey inside the window is skipped. The skip is recorded
  (`min_interval`) so you can see the policy working rather than guessing.
- **Response link validity** (default 14 days): how long the emailed link
  works. Links are single-use regardless. A second open gets an honest
  "invalid, used, or expired" page, never a second ballot.
- **Quiet hours** (optional `HH:MM` window in the org timezone): surveys
  arriving inside the window are deferred to its end, then send. Leave both
  bounds empty to send as soon as a survey is queued.

## 3. Where responses land

- **Scorecards** — each tech's card gains a CSAT block (responses, average
  score, low scores at ≤ 2) once the org has real responses. Before that
  the section stays "not yet available" instead of showing a fake zero.
- **Team scorecards → Client satisfaction** — the per-client rollup table
  for owners and admins.
- **Automations** — the *CSAT response recorded* trigger with the
  *CSAT score below N* condition fires on low scores. Use it to notify a
  manager or add an internal note while the ticket is still fresh.

## 4. The response experience

Clients open the tokenized link from the resolution email and sign in to
the portal — the sign-in is the attribution, so responses are never
anonymous. They pick 1–5 with an optional comment and see a thank-you
state. The link in the email and the in-app route are the same single-use
token.

## Troubleshooting

- **No surveys arriving** — check the transport (Email settings), the
  send-on-resolve toggle, and whether the client is inside the minimum
  interval.
- **A link does not work** — it was already used or its validity has run
  out; there is no override, by design.
