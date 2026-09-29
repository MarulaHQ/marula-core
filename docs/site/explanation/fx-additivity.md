# FX additivity and the append-only rate book

> **Audience:** both — billing practitioners read reconciliation totals
> constantly; admins own the rate book itself.

Two contracts govern money in multi-currency stacks, and they travel
together. The command-side procedure is the [FX how-to](../how-to/admin-fx-rates.md);
this page explains the **why** they bind this way.

## Contract 1: one base currency, all surfaces reconcile to it

Settings→General carries the single sentence that defines the reporting
stack:

> base currency — "the ONE reporting currency all money surfaces
> reconcile to"

Every currency-aware surface (contracts, invoices, credit notes, the
priced book) keeps its ledger math anchored to that one currency, so a
  report built from two views never argues about which currency its total
  is *in*. The mechanics behind it live in [multi-currency money and FX
  (ADR 0002)](../../adr/0002-multi-currency-money-and-fx.md):

- Money is carried as a typed value with its own currency, never a bare
  number (the classic dollar-and-cents-in-a-float loss class stays out of
  the stack).
- Conversions **snapshot the rate at the moment of conversion**, so a
  later rate change drifts nowhere in history.

## Contract 2: the rate book is append-only, corrections add rows

Settings→FX opens with the assertion: "*Rate book (append-only, corrections
add rows)*" — where a correction is a **new row** with a new as-of
date/rate pair, never an edit to an existing row.

The reason the contract binds rate recording to history:

- Invoices **snapshot the FX rate** used at conversion time, and once
  issued, the invoice's lines fix. That means the rate itself is part of
  the financial record — a corrected rate with a changed as-of, recorded
  as a row, forever available to reconcile against the snapshot.
- Change base currency while snapshots exist? **Refused.** The surface
  says "record rates first" for the inverse case — record rates first,
  only then change the base reporting currency.

## Contract 2's corollary: additivity is per kind, never across kinds

The reconciliation read model (`ops_v1_fx_reconciliation`) carries the
assertion as its own banner: "totals are per kind and never summed
across kinds — the additivity contract".

> "totals are per kind and never summed across kinds"

What it means downstream: each money **kind** (contract period,
invoice line, credit note, dunning entries, and the rest) reconciles
additively **within the kind** — you can sum two invoice lines, but
invoice lines and credit notes are not additive against each other and
neither is a credit note's usage against a paid invoice. Two drives for
no correction surface:

1. **Correctness of the bars:** each kind's total carries the same
   base-currency anchoring as its own kind's ledger. Add across kinds and
   the mixed-currency arithmetic that was guaranteed to stay honest no
   longer has one anchor.
2. **Provenance is audit-grade.** The write state of financial records is
   documented in the [provenance](provenance-vs-audit.md) ledger. If
   corrections cannot rewrite the ledger, kinds are additive exactly
   because the rows live independently of the totals: no correction
   swept across a boundary, no hole where the contract would have to
   guess.

## The consequence for every report you build

- Anything that sums across kinds is **not a Marula total**; compute it
  knowingly from per-kind breakdowns — leave the contract unbent and
  the dashboard's honesty intact.
- The versioned read model (`ops_v1_fx_reconciliation` view) is the
  stable seam for any SQL against reconciliation — the [read model
  guide](../how-to/read-model.md) covers the materialized view layering.
