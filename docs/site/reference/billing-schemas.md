# Billing schema reference

The tables behind work-type categories, billing verification, contract pricing
and project milestone billing, and the rules an integrator has to know before
writing to them. Every table is organization-scoped with the same RLS
convention as the rest of the schema: an org's rows are its own, and the
API is the supported write path.

The HTTP surface for all of it is in the
[API reference](api.md) (generated from the committed pin). This page is
the data model behind those routes.

## Time taxonomy (migration 0110)

**`time_categories`** — one row per category, keyed by a permanent slug.

| Column | Notes |
|---|---|
| `slug` | lowercase snake_case. Permanent. Entries and rate cards key onto it. |
| `label` | display text. Free to change. |
| `default_billable` | applied to new entries in this category |
| `requires_unbillable_reason` | turning billing off then demands a typed reason |
| `rate_card_affinity` | documentation that T&M cards are expected to price this slug |
| `position` | admin-managed order, and every capture picker sorts by it |
| `retired` | soft retire. Pickers drop it, entries keep it. |

**`time_category_work_type_map`** — `legacy_value` → `category_slug`, with
`source` of `backfill` (migration 0110) or `manual` (an admin alias).
Capture resolves slug-first, then this table, case-insensitively. An
unmapped value is refused, never guessed.

Seed data lands at three provisioning seams (sign-up, the demo seeder,
the API test-harness org), because migrations run before the demo org
exists on a fresh database.

## Verification (migration 0111)

**`time_entry_adjustments`** — the reduce-only ledger. Each row carries
the entry's original minutes plus a signed delta and a reason. Nothing
rewrites `time_entries.minutes` for billing: effective minutes are
`minutes + Σ delta`, and the run prices that. A delta that would raise
the bill is refused with a 422.

**`time_entry_dispositions`** — the reviewer's ruling per entry
(approved, rejected, non-billable, needs-review) with the approval state
it resolved to, the typed unbillable reason where one applies, and the
actor.

**`invoice_run_signoffs`** — one attestation per contract window:
three checklist claims, the reviewer, the moment, an optional note. The
server re-derives all three claims inside the writing transaction, so a
partial window cannot be signed. `create_run` claims every sign-off its
invoices cover, and a sign-off is evidence rather than a gate: a run with
no sign-off still issues.

An invoiced entry is locked. Every action re-checks
`invoice_time_entries` in the same transaction and refuses with 409.

## Contract pricing (migration 0112)

Six org-scoped child tables plus five new columns on `contracts`.

**`contract_blocks`** — a prepaid hour bank: `total_minutes`,
`sale_price_minor`, an optional discounted `block_rate_minor`,
`rollover_policy` (`expire` / `rollover` / `none`), optional
`expires_on`. Rows are immutable and the API replaces the set.

**`contract_block_draws`** — the additive draw ledger. Each row names the
time entry, the minutes it consumed, and any `spilled_minutes` that
overflowed the balance onto the overage card. Remaining balance is
computed, never stored. Draw-down is deterministic: entries ascending,
blocks oldest first.

**`contract_caps`** — a not-to-exceed bound, unique per contract and
scope, optionally narrowed to one work-type bucket. `scope` is `period`
or `contract`.
`reached_behavior` is typed: `stop` drops the work, `warn` records a
cap warning in the evaluation, `overage_billable` emits a `cap_overage`
line at the cap's own rate. `overage_rate_minor` is nullable, and `None`
bills at the matched card rate.

**`contract_minimums`** — one row per contract: the per-period floor in
minor units. A period that bills short gets a `minimum_trueup` line.

**`contract_escalators`** — `uplift_bps` (1–5000, so 300 is +3.0%) and an
optional `first_uplift_on`. Applied at each billing anniversary,
compounding linearly per anniversary, on its own `escalator` line kind.

**`contract_change_orders`** — lifecycle `draft → approved | rejected |
cancelled`, with `billed` run-owned. Approved orders price as
`change_order` lines inside their approval window. The run flips them to
`billed` with the invoice link, and a manual write to `billed` is
refused.

**`contract_sla_credits`** — `reason` (required, non-blank), a positive
`amount_minor`, `currency`, and `applies_on` (the window the credit
lands in, inclusive-day at evaluation). `ticket_id` is optional.
Pending credits become the only negative line kind. The run marks them
applied with the consuming invoice, and an applied credit is immutable.

New `contracts` columns: `unit_price_minor` + `unit_basis`
(`devices` counts active client-scoped `device_assets`, `users` counts
seats, both read at evaluation time), `incident_price_minor` +
`incident_priority` (`any` or `p1`–`p4`), and `po_number`. `invoices`
gains `po_number`, copied by the run.

PATCH semantics for all of this are tri-state: an absent or null money
field means unchanged. Only two fields can be cleared explicitly,
`clear_minimum` and `clear_escalator`.

New billing line kinds: `per_unit_base`, `per_incident`, `block_sale`,
`block_drawdown`, `cap_overage`, `minimum_trueup`, `change_order`,
`sla_credit`, `escalator`.

## Project billing (migration 0113)

**`projects.billing_contract_id`** — `ON DELETE RESTRICT`. The linked
contract must belong to the project's own client and must not be
cancelled. Its rate card prices approved phase time and its runs carry
the project's releases.

**`project_phase_budgets`** — one row per phase, carrying `budget_minor`
+ `budget_currency` and/or `fee_minor` + `fee_currency`. The burn read
model reports consumed approved minutes, card-priced consumed value, the
budget, and a typed alert (`none` under 80%, `warn` at 80%, `over` at
100%). A phase with no budget reports `none` on purpose: no budget is not
a zero budget.

**`project_billing_releases`** — the durable ledger row a consumer stages
per released milestone (or completed fixed-fee phase). `UNIQUE
(project_id, phase_id, status) WHERE status <> 'cancelled'` is what makes
redelivery inert. Status is `pending` / `billed` / `cancelled`, and the
run flips `billed` with the invoice and line inside its own
transaction.
The release id rides the line description in `[id]` brackets so the flip
does not depend on description text.

**`time_entries.phase_id`** — a CHECK-enforced exactly-one-parent rule
(`ticket_id` XOR `phase_id`), so project time is logged without inventing
a ticket. `ops.v1_time_entries` was restated with a LEFT join, and
`phase_id` appended, so report rows survive.

## What is not here

Sales tax. No tax engine computes anything on an invoice, and totals are
line sums. The boundary and its reasoning are in
[contract relationship types](../explanation/contract-relationship-types.md)
and [billing corrections](../explanation/billing-corrections.md).
