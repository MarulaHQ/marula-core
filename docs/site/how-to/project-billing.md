---
description: "A ticket traces work to a client. But rollout projects and migrations live in Projects — and until now, project work that skipped tickets had no path to an…"
---

# How to bill project work — contracts, milestones, fees

> **Audience:** admin (billing setup and runs are an admin + bookkeeper
> surface). Project leads benefit from knowing where their phase money
> stands. Clients see only issued invoices.

A ticket traces work to a client. But rollout projects and migrations
live in Projects — and until now, project work that skipped tickets had
no path to an invoice. This lesson covers the four seams that close
that gap: link a project to the contract it bills through, set a phase
budget and watch its burn, release a milestone (or a fixed-fee phase)
into the invoice run, and log project time without a ticket detour.
The seeded demo org already carries a working example: the Acme Network
Refresh project bills through contract CT-1001, its completed Discovery
phase holds a staged $1,200 release, and its Implementation phase burns
against a $1,500 budget.

## 1. Link the project to its contract

A linked project bills through its contract: the contract's rate card
prices approved phase time, and the contract's invoice runs carry the
project's releases. Open a project and set **Billing contract** (or
`PATCH /api/projects/{id}` with `billing_contract_id`). One rule
applies: the contract must belong to the same client as the project.
An Acme project cannot bill through a Northwind agreement, and the API
refuses the link rather than silently mis-billing someone.

Leaving the field empty keeps the project non-billable — internal work,
fixed-price retainers you handle outside the system, anything that
should never appear on an invoice run. The link prices nothing by
itself. Money moves when a release is staged or approved time lands.

## 2. Set a phase budget and read the burn

For time-and-materials phases, set the contracted budget on the phase:
**Create phase** or **Edit phase**, then `budget_minor` +
`budget_currency` (or `fee_minor` + `fee_currency` for a fixed-fee
phase, below). The budget is the denominator for the burn read model.

`GET /api/projects/{id}/phase-burn` returns one row per phase:

- `consumed_minutes` and `consumed_minor` — approved, billable entries
  scoped to the phase, priced at the linked contract's rate card.
- `budget_minor` — what the client agreed the phase costs.
- `alert` — `none` under 80%, `warn` at 80% or more, `over` at 100%.

A phase with no budget always reports `none`, on purpose: no budget is
not a zero budget, and reporting `over` on a
never-agreed number would invent a commitment nobody made. When a phase
crosses into `warn`, the honest responses are a change order, a client
conversation, or an accepted loss — the number tells
you which conversation to have.

## 3. Release milestones into the run

A completed milestone phase with an amount (the existing
`milestone_amount_minor` on the phase) doesn't wait for anyone to
remember it. Completion stages a **release** — one ledger row carrying
the phase's name and amount, dated the day it completed. You can inspect the ledger
any time at `GET /api/projects/{id}/billing-releases`.

The first invoice run on the linked contract whose window covers the
release date bills it as its own `project_release` line and flips the
ledger row to `billed` with the invoice reference. A pending release
bills on the next run. It never bills twice, because the run flips the
ledger row in the same transaction that issues the invoice — a replayed
completion event lands on an already-`billed` row and stays inert.

Corrections have one door: cancel a `pending` release
(`POST .../billing-releases/{id}/cancel`) before it bills. A `billed`
release is immutable, like every issued invoice — fix it through a
credit note.

## 4. Release a fixed-fee phase

Not every phase is a milestone. A phase with `fee_minor` +
`fee_currency` in its billing record releases that fee on completion,
same ledger, same run behavior, same one-time guarantee. Milestone
phases keep using their milestone amount. The fixed fee is for the
phase that just costs what it costs whenever it's done.

## 5. Log project time without a ticket

Time entries normally require a ticket. Project-scoped entries relax
exactly that: `POST /api/time-entries` with `phase_id` instead of
`ticket_id` logs the entry straight onto a phase. Exactly one parent is
allowed — an entry belongs to a ticket or to a phase, never both,
never neither, and both the API and the database check it.

Approval works the same queue you already run. Project entries appear
alongside ticket entries and bill the same way. Billing consumes them
through the phase (the burn read model above), so a project entry pays
without anyone spinning up a
ticket to carry it.

## What this does not do

No per-phase rate cards: phases price through the linked contract's
card, full stop. No automatic phase completion on burn-out: a budget
exhausting is a documented, alerted fact, and a phase completing is a
human decision. And no currency mixing across a run — a project whose
releases are in a different currency than its contract bills in a
separate run window, same as any other mixed-currency situation.
