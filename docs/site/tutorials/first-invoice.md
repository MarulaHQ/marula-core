# Tutorial: your first invoice

> **Audience:** admin — staff contribute the billable time; this lesson
> runs the review and the run that turn it into an invoice.

Money follows one arrow: time entry → approval → verification → billing
run → invoice. Wave 39 added the middle step, and the middle step is
where a bill gets looked at before anyone sends it. This lesson walks
the whole arrow on the seeded demo org (`owner@demo.local` /
`demo-password-123`).

## Prerequisites

- The seeded demo org, or your own stack with one client, one active
  contract, and time entries sitting in the approvals queue.
- Billing scope (admin or bookkeeper). Approving time is staff scope;
  verifying and invoicing is not.

## 1. The time entry carries a category

Open **Time categories** (/settings/time-categories) and skim the list.
Each row is a slug the capture surfaces write onto an entry: `on_site`,
`remote`, `incident`, `warranty`. Two flags per row matter downstream —
whether the category bills by default, and whether flipping an entry
non-billable demands a typed reason. Reasons are a fixed set (warranty,
courtesy, in_retainer_block, sla_covered, internal, mistake), so a
non-billable entry with no reason is refused rather than stored blank.

Entries logged before the taxonomy existed still work: the free text
normalizes through the legacy map, and anything unmapped is refused
instead of guessed. [The how-to](../how-to/staff-time-categories.md)
covers capture and the map.

## 2. Approve the time

**Approvals** (/approvals) lists submitted entries. Billing consumes
approved entries only. Once an entry is on an invoice it locks for
good — there is no un-invoicing, only correction documents
([corrections](../explanation/billing-corrections.md)).

## 3. Verify the period

Open **Billing → Billing verification** (/billing/verification). Pick the
contract and the window; the Billing page links straight into the same
window it was about to run, and a `contract_id` in the URL is honoured
so a link into one contract never lands on somebody else's.

Read the counters first. They are the gate:

- **Undispositioned** — entries nobody has ruled on yet.
- **Unclassified** — approved entries with no work-type slug.
- **Not approved** — time that will not bill at all.

Every entry row opens an action dialog with six actions. Approve and
reject move the entry through the same queue the Approvals page uses, so
the two surfaces cannot disagree about what "approved" means.
Adjust-minutes is the one that surprises people: it requires a reason
and it only ever lowers a bill. Asking it to add minutes is refused with
a 422, because adding minutes is an edit to the entry, not a review
decision. Entries already on an invoice are locked — visible, labelled
locked, and refusing every action with a 409.

Two numbers sit next to each other on purpose: the sum of what the
table prices, and the engine's own preview for the same window. When
they agree, the invoice will agree with the screen. When they differ,
the gap is a charge the entry table cannot itemise. A base fee, a
minimum commitment, a cap, a block sale — the note under the table says
which.

If the contract has prepaid blocks, the block panel shows the balance
honestly: minutes drawn before this window, minutes drawn in it, what is
left, and any overage that spilled onto the overage card.

## 4. Sign off the period

The close card lists three claims: every entry dispositioned, zero
unclassified, preview read. The server re-derives all three against the
live window before it stores anything, so a partial period cannot be
signed. Your name, the moment, and any note you wrote are kept with it.

A sign-off is evidence, not a lock. A run over an unverified window still
issues — the run claims whatever sign-offs cover it, and that is how
"who checked this period, and when" stays a single lookup later.

## 5. Run the invoice

Open **Billing** (/billing) and start a run against the same contract
and dates. What the engine does at that moment:

- **Draft contracts are skipped.** A draft previews; it never bills.
- The run evaluates every line kind the contract has configured: time
  from the rate card, block sales, cap behaviour, minimum true-ups,
  escalator uplifts, approved change orders, service credits, and
  project milestone releases. So do the metered kinds: block
  draw-down, a per-unit line, a per-incident line.
- Lines are **fixed at run**. The invoice is a snapshot of the numbers,
  not a live re-render.
- Everything the run consumed is marked consumed inside that run's own
  transaction: the block balance it drew down, the service credit it
  applied, the change order it billed, the milestone release it settled.
  A replay cannot bill any of them twice.

## 6. The issued invoice

The invoice is read-only once issued. What remains are corrections:
issue a credit note, void it, record a payment, print, send a statement.
Email in the demo mail mode is honest about being queued rather than
sent.

## 7. Multi-currency stacks

If a contract is priced in a currency other than the org's base
currency, the run snapshots FX rates. The rationale is in
[FX additivity](../explanation/fx-additivity.md) and the mechanics in
[the FX how-to](../how-to/admin-fx-rates.md). Mixed currencies in one
run are refused rather than converted behind your back.

## What you learned

- Categories decide billing semantics before anybody approves anything.
- Verification lowers a bill, never raises one, and every action is
  audited.
- A run is a snapshot engine. It consumes what it bills, in one
  transaction, and drafts never bill.
- Corrections are additive documents, never edits to an issued invoice.
