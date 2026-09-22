# How to record FX rates & read reconciliation

> **Audience:** admin — the rate book and its corrections are
> settings-owner work (billing operators may read either surface).

## Prerequisites

- Admin access to **Settings→FX rates**.
- Awareness of your org's **base currency** (Settings→General): "the ONE
  reporting currency all money surfaces reconcile to".
- Understanding of the two-line contract this page enforces — read
  [FX additivity](../explanation/fx-additivity.md) if that contract is
  unfamiliar. This page shows the mechanics.

## The rate book, and why it is append-only

> "Rate book (append-only, corrections add rows) — the ONE reporting
> currency all money surfaces reconcile to."

So: correction is an action you take by **adding a row**, not by editing
the past. Each row records base/quote pair, as-of date, and rate ("1 USD
buys this many quote units"). History is kept because invoices snapshot
specific rates — see [first invoice](../tutorials/first-invoice.md) for
where that snapshot binds to a billing run.

## 1. Record a rate

Open **Settings→FX rates** and **Record a rate**: base, quote, as-of,
rate. As-of dates are what reconciliation reads against — record each
rate with the date the conversion actually happens, not the date you
remember it.

## 2. Read reconciliation without breaking its bars

The **FX reconciliation** panel reads the `ops_v1_fx_reconciliation`
view. Its rule is load-bearing:

> "totals are per kind and never summed across kinds — the additivity
> contract."

Read each kind's total on its own. Never sum across kinds — the view is
shaped so each kind's numbers stand apart, and cross-kind sums are not a
meaningful quantity. If an external check needs cross-kind totals, compute
them from per-kind values knowingly, at variance with the contract, never
from any "reconciliation grand total".

Request the read-model detail (the surface quirks of
`ops_v1_fx_reconciliation`) from the [read model guide](read-model.md)
when scripting against it via SQL.

## 3. Base currency side-effects to know first

Two consequences of Settings→General's base currency guard:

- **Changing the base currency is refused while FX conversion snapshots
  exist**. Record rates first; reset snapshots deliberately if you
  really mean to change reporting currency.
- Existing rows never rewrite. Append-only means corrections add rows,
  and by extension nothing in history silently changes value.

## What you learned

- Record per as-of. Corrections add rows, and nothing edits history.
- Reconciliation is per-kind, additive within kind, never summed
  across — [the rationale](../explanation/fx-additivity.md).
