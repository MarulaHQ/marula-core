# ADR 0002 — Multi-currency: transaction currency is the fact, conversion is a snapshot

**Status:** Accepted · 2026-08-30
**Scope:** money handling across all modules (time costs, expenses, inventory,
quotes/invoices, payments) and FX rate data

## Context

MSPs transact across currencies at different moments: a USD-based org pays a
ZAR contractor monthly, buys inventory in GBP on date X, and reconciles
everything against the books on a third date. The foundation already exists:
`Money` (core-domain) is integer minor units + an ISO 4217 code, refuses
cross-currency arithmetic (`CurrencyMismatch`), forbids floats, and rounds
percentages half-even; contracts already carry a `currency` column (migration
0029). The open question was where **conversion** lives.

Options considered:

1. Single base-currency columns everywhere; original currency dropped at entry.
2. Convert lazily on read, using a live rate service.
3. Store the transaction currency plus an **immutable conversion snapshot**
   taken at the transaction's lock moment; rates themselves are append-only data.

Constraints from doctrine and reality:

- Reconciliation with the books (QBO/Xero sync) needs reproducible history:
  "what USD was this GBP receipt" must give the same answer forever, even if
  rate data is corrected later.
- Standalone-first: the core cannot require a rate service. Manual entry is
  the "none" mode (mirrors `CompletionProvider` vs Relay's managed router).
- The PSA is not an accounting program: no general ledger, no FX revaluation
  (realized/unrealized gain-loss belongs to the books), no tax engine
  (gap-analysis C-list).
- Facts vs judgment is a first-class split: the core records deterministic
  facts; intelligence layers explain on top.

## Decision

Option 3.

- **Org base currency.** Each org has exactly one base (reporting) currency,
  set at creation and changeable only with a full re-snapshot.
- **Transaction currency is stored, always.** Every money-bearing column uses
  `Money`; nothing is stored "in base only".
- **`fx_rates` is append-only data**: (base, quote, as_of_date, rate, source,
  captured_at). Corrections add new rows; history is never rewritten.
- **Conversion snapshots at five named lock moments** — expense/purchase
  entry, invoice issue, time-entry costing, inventory receipt, payment
  settlement — store `fx_rate_used`, `fx_rate_date`, `fx_source`, and the
  base-currency amount. Snapshots are immutable once written. Reports sum
  base amounts from snapshots; they never re-convert, and summing mixed
  original currencies is a type error, not a warning.
- **`FxRateProvider` is a core adapter trait.** The built-in provider is
  manual entry; managed daily feeds are commercial or community bundles on
  the trait (ECB/OFX/OpenExchangeRates-class sources).
- **Conversion is explicit, never implicit.** One core-domain conversion
  operation, direction-stated, rounding pinned to half-even (consistent with
  `Money::percent`); no implicit conversion in the type system, API, or UI.
- **The FX deliverable to the books is a reconciliation read model** —
  original amount, rate, source/date, base equivalent, sync target.
  Realized/unrealized gain-loss stays in the accounting system, and the
  product states this boundary so nobody expects revaluation from a PSA.

## Consequences

- The currency-aware schema is a **MER-P0** deliverable (gap-analysis B8):
  retrofitting after invoices ship would be brutal.
- Multi-currency invoicing, the managed daily FX feed, and landed-cost
  allocation are **Relay features (REL-S4, B8 v1.x)** riding this schema;
  self-hosters keep manual rates and still get reproducible snapshots.
- Landed cost v1 allocates cost components across units by value only;
  weight/volume/duty-recovery allocation is out of scope until there is a
  design partner that needs it.
