---
description: "Hand your accountant one period of money rows, each mapped to an account and traceable to its Marula record."
---

# How to hand your accountant a period

> **Audience:** admin — reading or choosing the account a class posts to is
> settings-owner work, and the package is read-only.

A month-end handoff usually means a spreadsheet, a folder of PDFs and a list of
questions. The accountant package answers the questions in one document: every
money row in a period, the class it carries, the account it posts to, and the
stable reference that points back at the one Marula record it came from.

## Prerequisites

- The period you want to hand over.
- Financial read access (the package includes money).
- A class mapping for every class you post under. Every class ships with a
  default account except cost of goods sold: its account number varies by chart,
  so the package names it and holds until you choose one.

## Steps

1. Open the accounting settings and check **Class accounts**. Every class has an
   account code and name: inventory, sales, rental income, freight in, duty,
   duty in, brokerage in, insurance in, service cost, rental expense, e-waste
   cost, overhead and other. The defaults are a starting point, not a claim
   about your books. Cost of goods sold is the one class with no default. See
   step 5.
2. Choose an account for any class your chart names differently. The change
   applies to the next package you read; it does not rewrite documents you have
   already issued.
3. Read the package for the period:

   ```http
   GET /api/v1/accounting/package?from=2026-11-01&to=2026-11-30
   ```

   Add `.csv` to the path (`/api/v1/accounting/package.csv`) for the same
   period as a CSV file, with one line per record and the same fields and account codes
   as the JSON form. An organisation with no accounting
   connection can hand that file over directly.

   Text that begins with `=`, `+`, `@`, a tab or a non-numeric `-` is written
   with a leading apostrophe, so a spreadsheet keeps it as text instead of
   evaluating it. The JSON form keeps the exact value.

4. Hand the accountant the response. It carries:

   - `period_from` and `period_to`, plus the organisation's `base_currency`.
   - `class_accounts`: the class-to-account map in force.
   - `records`: one row per money fact, each with a `stable_ref` of the form
     `kind:id`, its document reference where one exists, the currency, the
     amount, the base amount where a snapshot exists, its reporting class and
     the account it posts to.
   - `totals_by_account` and `totals_by_kind`. These add a row only when it is
     in the base currency or carries a locked exchange snapshot, so a total
     never mixes currencies.
   - `unconverted_rows`: foreign rows with no snapshot. They stay in `records`
     and are left out of the totals.

5. If the package answers 409 with `account_mapping_missing`, one of the period's
   rows carries a class with no account. The response names every such class in
   `classes`. Choose an account for each and read the package again. The rows
   were never rewritten, so nothing is lost by the hold.
6. If a line is questioned, the `stable_ref` is the trail: `invoice:<id>`,
   `credit_note:<id>`, `purchase_bill:<id>`, `acquisition_cost:<id>`,
   `rental_charge:<id>`, `stock_item:<id>` or `ewaste_disposition:<id>` names the
   record in Marula, and `kind:id` never changes for that record.

## What the package covers

| Kind | Where it comes from | Class |
| --- | --- | --- |
| `invoice` | An issued invoice in the period | sales |
| `credit_note` | A credit note issued in the period (a negative amount) | sales |
| `vendor_bill` | A purchase bill posted in the period | inventory for goods, rental expense or e-waste cost for those intents, cost of goods sold for a project consumable |
| `acquisition_cost` | A capitalised landed-cost layer dated in the period | its reporting class |
| `rental_charge` | A lease's rent charge whose period starts in the period | rental_income |
| `stock_item` | Every item with quantity on hand at period end | inventory |
| `ewaste_disposition` | A disposition recorded in the period (a zero amount) | ewaste_cost |

A non-capitalised layer posts to a ticket instead, so it is not in the package;
it appears on the ticket's costs and in the reporting-class rollup.

A stock valuation row is one per item, valued at the item's last receipt cost at
or before the period end, over the quantity the ledger shows at that date. Both
halves are dated, so re-reading a closed period gives the same value even after
later receipts. It is not a FIFO layer: Marula prices stock at its last unit
cost everywhere, and the package follows the same rule. An item that has
quantity but no receipt cost at or before that date appears at zero, with the
reason in its memo. It never borrows the cost of a later receipt.

An e-waste disposition row has no amount because the disposition itself records
no money. The memo carries the pathway, the weight, the partner and the
certificate, so the accountant can tie the custody fact to the paperwork.

## Troubleshooting

- **The period is refused with 422.** The end date is before the start date.
- **The package is refused with 409 `account_mapping_missing`.** The response's
  `classes` lists every reporting class in the period that has no account.
  Choose one for each in **Class accounts**, and the package then reads normally.
  This is deliberate: posting real money to a guessed 5900 account would be
  worse than asking.
- **An amount differs from the invoice total.** Credit notes are negative rows. A
  base amount is present only where the document locked an FX snapshot.
- **A class is missing from your list.** The package always carries the full
  default set, plus any class you have chosen yourself.
