# How to verify a billing period before you invoice it

> **Audience:** admin — the verification workspace is the gate between
> "hours got logged" and "hours got billed", so it is billing-owner work.

## Prerequisites

- A billable contract (time & materials or retainer) with at least one
  logged entry in the window you want to close.
- If the org uses a **prepaid block**, read [contract relationship
  types](../explanation/contract-relationship-types.md) — the
  consumption panel on this page reports that ledger, and the
  vocabulary there is the vocabulary here.
- If the org bills **project work**, read
  [project billing](../how-to/project-billing.md) — phase-scoped entries
  appear in this window as their own group, with no ticket number.

## What this page is for

An invoice run prices what it finds. Verification is where a human reads
that same window first, decides what each entry really is, lowers the ones
that were logged wrong, and signs the period off. Nothing on this page
invoices anything — the run composer does that, and it points here.

Open **Billing→Verification**, pick a contract and the two dates, and
read. The window you pick here carries into the run composer, so a
reviewer can verify first and invoice from the same dates.

## 1. Read the window before you touch it

The header band is the summary; the body is the evidence.

- **Counters** (top): *undispositioned* and *unclassified* are the two
  that block a close. *Not approved* counts drafts and work still sitting
  in the approval queue. *Adjusted* and *Invoiced* are history.
- **Running total**: the money this window will actually bill, so the
  rows, the per-ticket cards, and the headline figure all foot together.
  Draft work and rejected work show their minutes with a $0.00 cost,
  because a table whose rows do not sum to the total above it teaches
  you to trust neither. If this total still disagrees with the preview
  figure the engine computed (shown under the close card), the gap is a
  contract line the entry table cannot show: base fees, block sales,
  caps. Read the preview before you run.
- **Block consumption** (only on contracts with prepaid blocks): the
  balance, what this window's entries drew, what spilled to card rates
  because the block ran out, and what earlier runs already consumed. A
  `depleted` badge means the money is gone and the rest bills at card
  rates; `expiring` means the balance is real but the block ends inside
  this window.
- **Groups**: one card per ticket (or per project phase — project work
  has no ticket number). Each card totals its own minutes and cost, and
  each row shows the badge that decides whether it bills: *Ready to
  bill*, *Approved*, *Draft*, *Awaiting approval*, *Non-billable*,
  *Unclassified*, *Rejected*, *Invoiced*.

A row marked **Invoiced** is locked. It stays visible, so you can always
see what was billed, and every action on it is refused. That is
deliberate, and a locked row is not a dead row. A correction after
invoicing is a credit note, not a verification edit.

## 2. Decide an entry (the badge you are setting)

Press **Review** on a row. One dialog, every in-context decision:

| Action | What it does | What it demands |
|---|---|---|
| **Approve** | Moves submitted work to approved through the normal approval queue | — |
| **Reject** | Rejects the entry with a typed reason code | Reason code + reason |
| **Flip billable** | Marks the work non-billable (or billable again) | Reason + a typed non-billable reason (warranty, courtesy, retainer, SLA, internal, mistake) |
| **Adjust minutes** | Lowers the billable minutes for this entry | A reason |
| **Recategorize** | Moves the entry onto another category from your taxonomy | A category that exists |
| **Add note** | Annotates the entry | The note |

Two rules are worth knowing before you are surprised by them:

- **Verification lowers a bill, never raises one.** The logged minutes
  stay on the record, and the difference is kept as its own signed row
  with your reason on it. If an entry was under-logged, fix the entry —
  do not try to bill more than the clock says.
- **Every action writes an audit row** with your name, the moment, and
  what changed. An adjustment without a reason is refused rather than
  stored quietly, because six weeks later "why is this 30 minutes?"
  needs an answer.

Recategorizing a billable entry with no category is the quiet failure
this screen exists to prevent: with no category, the entry prices at the
contract's default rate, which is rarely the rate you meant. The
*Unclassified* counter names every one of them.

## 3. Close the period

The close card restates the checklist, and it will not let you sign
until the counters are clean:

1. **Every entry dispositioned** — nothing left unreviewed.
2. **Zero unclassified** — no billable entry priced at the default rate.
3. **Preview read** — the engine's number for the window has been seen.

**Sign off the period** records all three, plus your note and the timestamp.
The server re-checks the first two against the live window as it writes
the record, so a sign-off cannot go stale or contradict the data.

If an issued invoice run already covers the same dates, a warning lists
the overlapping runs on the page and in the sign-off dialog. It is a
warning, not a block: re-running an overlapping window is a real
thing to do after a mistake. Read the list before you decide.

Once a run invoices a window that carries a sign-off, the sign-off
points at that run — so "was this period verified, and by whom?" is one
lookup, not an archaeology project.

## What this page does not do

- It does not create or edit contracts, rates, or blocks — the contract
  editor owns those.
- It does not invoice. The run composer issues invoices, and it is the
  place to go once the period is signed.
- It does not free invoiced work for editing. A correction after invoicing is a
  credit note; see [the first-invoice tutorial](../tutorials/first-invoice.md) for
  where those live.
