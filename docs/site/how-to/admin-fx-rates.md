# How to record FX rates & read reconciliation

> **Audience:** admin — the rate book and its corrections are
> settings-owner work (billing operators may read either surface).

If your MSP bills a client in EUR or CAD while reporting company financials in USD, you need historical invoices and inventory receipts to keep the exact exchange rate from the day they happened. In Marula, **Settings → FX rates** stores an append-only rate book where every invoice and stock receipt locks in its exchange rate snapshot at creation time.

## Prerequisites

- Admin access to **Settings → FX rates**.
- Your organization's **base currency** set under **Settings → General** (the single reporting currency all financial rollups convert into).
- For the design background on why different financial document types are reconciled separately, see [FX additivity](../explanation/fx-additivity.md).

## Why the FX rate book is append-only

> "Rate book (append-only, corrections add rows) — the ONE reporting
> currency all money surfaces reconcile to."

You never edit or overwrite a past exchange rate row, because older invoices and stock receipts have already snapshotted that history ([see Your first invoice](../tutorials/first-invoice.md)). If you entered a wrong rate for a date, you correct it by adding a new rate row with the updated figure.

## 1. Record an exchange rate

Open **Settings → FX rates** and click **Record a rate**:

1. Confirm the **base currency** and select the foreign **quote currency**.
2. Set the **as-of date** (the date the rate applies to, not simply today's date if you are backfilling a month-end rate).
3. Enter the conversion **rate** (how many units of the quote currency `1` unit of your base currency buys) and save.

## 2. Read the FX reconciliation view

Below the rate book, the **FX reconciliation** panel displays converted totals from the `ops_v1_fx_reconciliation` reporting view:

> "totals are per kind and never summed across kinds — the additivity
> contract."

Each financial category (such as issued invoices, credit notes, purchase orders, or stock receipts) shows its own subtotal in your base currency. Marula intentionally keeps each category on its own line rather than combining invoices and purchase costs into a single meaningless grand total. If you query `ops_v1_fx_reconciliation` directly from a BI tool or SQL client, see [How to report against the read model](read-model.md).

## 3. Rules around changing your base currency

- **Base currency locks once snapshots exist:** **Settings → General** refuses changes to your organization's base currency while FX conversion snapshots exist, preventing historical reports from mixing two different base currencies.
- **Historical snapshots never shift:** Recording a new exchange rate today affects new runs and current As-Of lookups without altering any issued invoice from last month.

## What you learned

- Exchange rates are recorded by **as-of date** in an append-only book; corrections add a new row rather than rewriting past entries.
- Reconciliation reports totals per financial document kind in your organization's base currency ([see Why FX snapshots are additive](../explanation/fx-additivity.md)).
