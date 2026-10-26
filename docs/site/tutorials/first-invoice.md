---
description: "Walk the invoice cycle end to end in the demo organization, from billable time to a sent invoice."
---

# Tutorial: your first invoice

> **Audience:** admin — staff contribute the billable time; this lesson
> runs the review and the run that turn it into an invoice.

Turning technician hours into a clean client bill follows five clear steps:

1. A technician logs time with a **work category** (`remote`, `on_site`, `warranty`, etc.).
2. A service lead **approves** the time entry.
3. A manager or bookkeeper **verifies** the billing period to catch uncategorized time or apply courtesy write-downs before the invoice goes out.
4. You run the **billing batch** to generate the invoice snapshot.
5. If anything needs adjusting after an invoice is sent, you issue a **credit note** rather than editing a sent bill.

This tutorial walks through the entire cycle using the preloaded demo organization (`owner@demo.local` / `demo-password-123`).

## Prerequisites

- The seeded demo org, or your own workspace with at least one client, one active contract, and logged time entries sitting in the approvals queue.
- Admin or billing permissions. (Service leads can approve daily time entries, while verifying a billing period and issuing invoices require billing access.)

## 1. Check how work categories control billing

Open **Settings → Time categories** (`/settings/time-categories`). Every time entry a technician logs carries one of these categories, such as `on_site`, `remote`, `incident`, or `warranty`.

Each category has two rules that save you headaches at month-end:

- **Default billable status:** For example, `remote` and `on_site` default to billable, while `warranty` defaults to non-billable.
- **Required non-billable reason:** When a technician or manager marks an entry non-billable, Marula requires choosing a reason from a fixed list (`warranty`, `courtesy`, `in_retainer_block`, `sla_covered`, `internal`, or `mistake`). Nobody can quietly zero out hours without explaining why.

Older time entries imported from legacy tools normalize through a mapping table so nothing gets billed by guesswork. See [Capture and map time categories](../how-to/staff-time-categories.md) for the full setup.

## 2. Approve technician time

Open **Approvals** (`/approvals`). This queue shows submitted time entries waiting for a manager's eyes.

Only **approved** entries can be picked up by an invoice run. Once an approved entry is billed on an invoice, it locks permanently so your historical hours always match the invoices your client received ([see why billing corrections use credit notes](../explanation/billing-corrections.md)).

## 3. Review the billing period before creating the invoice

Before you generate invoices for the month, open **Billing → Billing verification** (`/billing/verification`) and select the client contract and date window you want to check. (Clicking **Verify period** from the main Billing page opens this exact contract and date range automatically.)

At the top of the screen, three counters tell you immediately if the period is ready to bill:

- **Undispositioned:** Time entries that nobody has approved, rejected, or written off yet.
- **Unclassified:** Approved entries that are missing a work category slug.
- **Not approved:** Entries that will be skipped by the invoice run.

Click any row to open the review dialog. From here you can approve or reject the entry, reclassify its work category, or **adjust minutes downward** with a required reason (for example, writing down 30 minutes of learning curve as a client courtesy). Note that verification only ever lowers billed minutes (`422` if you try to increase them), because raising someone's worked time belongs on the time entry itself, not during billing sign-off. Entries already attached to a past invoice show a `locked` badge and refuse edits (`409`).

Below the rows, compare the two totals side by side:

- **Table sum:** The total of the individual time entries listed on screen.
- **Engine preview:** What the full contract invoice will actually charge for this period.

When those two numbers differ, the note underneath explains why: for example, your contract includes a monthly base retainer fee, a minimum commitment true-up, a price cap, or a prepaid block purchase that doesn't come from a single time entry. If the contract uses prepaid block hours, the block panel shows hours used prior to this period, hours drawn during this window, remaining balance, and any overage spilling into billable rates.

## 4. Sign off on the period

At the bottom of the verification page, the close card checks three conditions: every entry has been reviewed, zero entries are unclassified, and the preview has been loaded. When you click **Sign off**, the server verifies all three conditions against live data and records your name and timestamp alongside any note you wrote.

Signing off creates an permanent audit trail showing who reviewed the pre-bill numbers and when.

## 5. Run the invoice

Open **Billing** (`/billing`) and run the invoice for that same contract and date window. Here is what happens under the hood:

- **Draft contracts are skipped.** You can preview a draft contract, but it will never generate a real bill until activated.
- **All contract line types are calculated together:** hourly time from the rate card, prepaid block draw-downs, monthly recurring fees, price caps, minimum commitments, annual escalator uplifts, approved change orders, service credits, and completed project milestones.
- **Prices are snapshotted.** Once generated, invoice lines store the exact calculated amounts rather than recalculating later if you change your rate card next year.
- **Consumed items are marked in the same database transaction.** Block hours drawn, service credits applied, change orders billed, and milestones released are all marked used at the exact moment the invoice is created, so running the batch twice can never double-bill a client.

## 6. Work with the issued invoice

Once issued, an invoice is a read-only legal and accounting document. From the invoice detail view you can record a payment, print or download the PDF, queue an email statement, void the invoice, or issue a **credit note** if a line needs to be refunded or adjusted.

## 7. Multi-currency contracts

If a contract bills in a currency different from your organization's base currency, the billing run records a snapshot of the foreign exchange rate on the invoice. See [Why FX snapshots are additive](../explanation/fx-additivity.md) and [Manage currencies and FX rates](../how-to/admin-fx-rates.md).

## What you learned

- Work categories set default billing behavior and require clear reasons when hours are written off.
- The **Billing verification** screen lets you catch unapproved or uncategorized time and apply courtesy write-downs before a client ever sees a bad bill.
- Running an invoice snapshots every line and marks logged hours, service credits, change orders, and milestones consumed in one step so nothing is ever double-billed.
