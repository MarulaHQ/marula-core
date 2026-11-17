---
description: "Every ledger gives up eventually. This page lists the jobs, emails, webhook deliveries and stream events that did, and the two decisions that are safe to make."
---

# How to recover work that gave up

> **Audience:** admin — the page is limited to roles that can read the admin
> module.

**Settings → Dead work** is the list of work a ledger abandoned after its final
attempt: background jobs, outbound email, signed webhook deliveries and stream
events. Newest abandonment first. It exists so that a failure is a decision you
can make, rather than a row only an engineer with database access can see.

## Prerequisites

- The admin module's read permission. Without it the page says the surface is
  limited to roles that can read the admin module, rather than showing a list
  you cannot act on.
- For a webhook row, the receiving system's owner within reach. A redrive
  re-sends, and only that person can tell you whether the first delivery
  already had an effect.

## 1. Read the consequence before the button

The note at the top of the page is the most important sentence on it:

> A redrive re-sends. Requeueing an item does not ask the other side whether it
> already acted on the first attempt — a webhook receiver may see the same body
> twice. A cancel is terminal: the row will not be requeued by anything on this
> page. Both decisions are written to the audit trail with your name.

Treat a redrive as a new delivery, not a retry of a failed one. For email that
usually means a duplicate in someone's inbox. For a webhook it may mean a
second ticket, a second charge or a second message in a channel. When the
receiver is idempotent, that is harmless. When it is not, the receiver's owner
is the person who decides.

## 2. Narrow the list

Most triage is two filters:

- **Restrict to one ledger** — jobs, email, webhooks or stream events. Start
  with the ledger that matches the symptom you are chasing.
- **Include cancelled** — off by default, because a cancelled row is a closed
  decision. Turn it on when you are reconstructing what happened.

The list returns the newest 200 rows. It is not a bulk console: each row is one
decision, audited on its own.

## 3. Choose a redrive or a cancel

**Redrive** requeues one row. It becomes due now with a fresh attempt budget, so
the ordinary worker path picks it up again. The confirmation says it will
re-send. Read that, then confirm.

Two outcomes are normal:

- **Requeued.** The work is due now. Watch the ledger it came from for the next
  attempt's result.
- **Already moved.** Nothing changed and no second audit row was written. That
  is not an error. The row had already left this list's control.

**Cancel** ends the row. It requires a reason, and the reason stays on the
record. A cancel is terminal: no redrive on this page will revive it. Use it
when the work should not happen at all — a retired receiver, a job for a
customer who has left, a message that has been overtaken by events.

## 4. Resume a destination that paused itself

A webhook destination pauses after repeated failures. The pause is a safety
measure, not a person's decision, and resuming it is a separate act from
redriving a row.

**Resume** gives the destination its whole failure allowance again before it
can pause once more. Check Health first: if the receiver is still down, a
resume only buys another round of failures.

## 5. What this page cannot do

- **No bulk redrive.** One row, one decision, one audit entry.
- **No redrive of a dead stream event.** Those rows are marked un-replayable:
  the payload needs the relay's stream writer, not a requeue.
- **No editing the recorded error.** The ledger's words are evidence. Fix the
  cause, then redrive or cancel.

## What you learned

- Dead work gathers the four ledgers that gave up, newest first.
- A redrive re-sends and can duplicate an effect. The receiver's owner decides
  whether that is safe.
- A cancel is terminal and carries a required reason.
- A paused destination is resumed separately, and gets a fresh failure budget.
- Every decision is audited with your name, and the page offers no bulk path.

## Where to go next

- [Check that records and stored files still agree](operator-object-integrity.md) —
  the other recovery reader in the same settings group.
- [Configure email](admin-email.md) — the delivery states that lead to a dead
  email row.
- [Watch your self-hosted services](admin-self-host-services.md) — the worker
  and relay health behind jobs and stream events.
- [Provenance vs audit](../explanation/provenance-vs-audit.md) — where the
  decision is recorded.
