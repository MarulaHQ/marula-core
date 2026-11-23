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
- Optionally, your chart of accounts. Every reporting class has a default
  account, so the package is usable before you touch anything.

## Steps

1. Open the accounting settings and check **Class accounts**. Every class has an
   account code and name: inventory, sales, rental income, freight in, duty in,
   brokerage in, insurance in, service cost and other. The defaults are a
   starting point, not a claim about your books.
2. Choose an account for any class your chart names differently. The change
   applies to the next package you read; it does not rewrite documents you have
   already issued.
3. Read the package for the period:

   ```http
   GET /api/v1/accounting/package?from=2026-11-01&to=2026-11-30
   ```

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

5. If a line is questioned, the `stable_ref` is the trail: `invoice:<id>`,
   `credit_note:<id>`, `acquisition_cost:<id>` or `rental_charge:<id>` names the
   record in Marula, and `kind:id` never changes for that record.

## What the package covers

| Kind | Where it comes from | Class |
| --- | --- | --- |
| `invoice` | An issued invoice in the period | sales |
| `credit_note` | A credit note issued in the period (a negative amount) | sales |
| `acquisition_cost` | A capitalised landed-cost layer dated in the period | its reporting class |
| `rental_charge` | A lease's rent charge whose period starts in the period | rental_income |

A non-capitalised layer posts to a ticket instead, so it is not in the package;
it appears on the ticket's costs and in the reporting-class rollup.

## Troubleshooting

- **The period is refused with 422.** The end date is before the start date.
- **An amount differs from the invoice total.** Credit notes are negative rows. A
  base amount is present only where the document locked an FX snapshot.
- **A class is missing from your list.** The package always carries the full
  default set, plus any class you have chosen yourself.
