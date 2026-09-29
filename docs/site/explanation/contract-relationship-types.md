# Contracts: one shape per client relationship

> **Audience:** both. Admins own contract setup. Staff see the type on
> tickets and invoices, and knowing what each band means helps when
> work gets logged.

A Marula contract is the commercial answer to one question: *how does
this client pay for this work?* The type you pick decides what the
billing engine evaluates, what the admin verifies, and what lands on the
invoice. Every type here is real practice at working MSP shops: it
tracks the agreement types ConnectWise and Autotask model. Wave 39 grew
the list from four to six shapes plus the attached configuration the
labor types share.

## The six contract types

**Recurring** bills a flat price per period (weekly through annual,
anchored to a date). Nothing is metered: the invoice is the period.

**Retainer** bills the same per-period base, but the price buys a block
of included minutes. Approved work inside the period draws the block,
and minutes past it bill at the overage rate. The base line and the overage
line are separate documents lines, so a verification pass can see both.

**Time & materials** prices approved labor from a rate card keyed on
work type, falling back to the contract's default hourly rate. Anything
attachable attaches here: prepaid blocks (below), not-to-exceed caps
(below), and the rate card itself.

**Fixed fee** bills dated schedule entries — a migration fee due March
1, an assessment fee due whenever. No metering at all.

**Per-unit** bills quantity × unit price per period, where the quantity
is read from your inventories at each evaluation: active client devices
(per-device billing) or org users (per-seat billing). Sell 25 seats,
the invoice says 25 × the unit price without anyone counting.

**Per-incident** bills a fixed price per closed ticket, optionally
banded to one priority (P1 incidents at a different price than P3
requests, for example). The price is per resolved ticket inside the
billing window, and nothing else is metered.

## Attached configuration (any labor-bearing contract)

These ride contracts of any of the six types, and the engine applies
them at evaluation:

- **Prepaid blocks** (hour banks). Sold up front as a `block sale`
  line, drawn down per approved entry from a ledger that survives runs.
  A block can carry a discounted draw rate (the hours cost the block
  rate, not the card rate) and an expiry with a typed policy: `expire`
  lapses unused minutes, and the run reports them so nothing vanishes
  silently. `rollover` reports the minutes a successor block should
  mint. `none` never expires. Depletion and expiry surface in the
  evaluation's block-consumption payload.
- **Not-to-exceed caps**. A labor ceiling per period or per contract
  with a typed cap-reached behavior. `stop` drops beyond-cap work.
  `warn` bills through and records a visible warning.
  `overage_billable` prices beyond-cap work as separate overage lines.
  A cap is never a silent cutoff: the behavior is a field on the cap,
  not an accident.
- **Minimum commitment**. A per-period floor. When a period's base and
  labor lines fall short, one true-up line tops the period up.
- **Escalator**. An uplift in basis points applied at each billing
  anniversary (300 bps = +3%), compounding linearly per anniversary,
  emitted as its own line so the base price keeps its identity.
- **Change orders**. Approved scope amendments with their own
  lifecycle (draft → approved → billed). Only approved orders price,
  and the run that consumes one flips it to billed with the invoice
  link.
- **SLA service credits**. Records of a missed SLA that become
  NEGATIVE lines on the invoice covering their window. Credits are
  never silent: the record names the ticket and the missed target, and
  an applied credit is immutable (corrections issue new credits).

## Purchase-order numbers, and where tax lives

Contracts carry an optional `po_number` that copies onto every invoice
the contract produces — the reference the client's AP system matches
against.

Tax is deliberately absent. The invoicing migration (0036) drew the
boundary: no tax engine on invoices, totals are line sums, and tax is
an accounting-export concern configured in QuickBooks or Xero where the
operator's accountant controls it. That boundary still stands. Quotes
carry a tax estimate for the sales moment only. Invoices do not.

## Why the engine is one pure function

Every one of these shapes evaluates through the same read-only rules
function. Previews and invoice runs call it with the same inputs, so a
preview can never disagree with the run that follows it. Block
draw-downs, cap behavior, and escalators are unit-tested as pure math.
Persistence happens at run time, additively, and an issued document is
never rewritten.

Projects plug into the same function: a project linked to a contract
bills through it, released milestones and fixed-fee phases ride the run
as their own line kind, and phase burn is read against the same rate
card. The task-side walk-through is [How to bill project
work](../how-to/project-billing.md).
