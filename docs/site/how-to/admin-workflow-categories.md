---
description: "In many ticketing tools, renaming a board column from \"Waiting on Vendor\" to \"Parts Ordered\" breaks every SLA rule, report filter, and billing script that…"
---

# How to set workflow categories

> **Audience:** admin — reshaping a board's statuses is a settings-owner
> change; staff experience the result as labels only.

In many ticketing tools, renaming a board column from `"Waiting on Vendor"` to `"Parts Ordered"` breaks every SLA rule, report filter, and billing script that looked for the old text. Marula prevents that by tying every board column to one of four fixed **workflow categories** underneath. You can name your columns whatever makes sense to your technicians, and the underlying category keeps your SLA clocks and reports working reliably.

## Prerequisites

- Admin access to **Settings** (under **Delivery**: **Boards**, **Workflow**, **Taxonomy**, and **Tags**).

## The four workflow categories

**Settings → Workflow** opens with the core rule:

> "Behavior binds to a status's semantic category, never its label."

Every status column on a board maps to one of four categories:

- **Open:** New or triaged work waiting to be picked up.
- **In progress:** Active work where billable hours happen and the SLA clock ticks.
- **Waiting:** Waiting on a customer reply or third-party part; **SLA clocks pause automatically**.
- **Closed:** Completed work; SLA clocks and billing stop.

## 1. Create or organize your boards

Open **Settings → Boards** to create ticket boards (for example, `Service Desk`, `Field Service`, or `Internal IT`), assign team members, and attach default checklist templates.

Board membership and checklists live on **Boards**, while status columns, allowed transitions, and custom fields live on **Workflow**.

## 2. Customize status columns per board

Open **Settings → Workflow** and pick the board you want to edit:

1. Rename any existing status label freely, reorder columns with the arrow buttons, or add a new status column.
2. Choose the matching **semantic category** (`Open`, `In progress`, `Waiting`, or `Closed`) from the dropdown on each row.
3. Mark which status is the default entry column for new tickets.

Because behavior follows the category:

- Renaming `"In Progress"` to `"Bench Repair"` changes only the display label.
- Changing a column's category from `In progress` to `Waiting` tells Marula to pause SLA timers whenever a ticket sits in that column.

## 3. Manage ticket types and the category tree

Open **Settings → Taxonomy** to manage your high-level ticket types (`Incident`, `Service Request`, `Problem`, `Change`) and your nested issue category/subcategory tree (such as `Network → Wi-Fi` or `Hardware → Printer`).

You can rename, move, or retire taxonomy nodes at any time without affecting SLA timers, billing rules, or workflow transitions. Retiring a node hides it from new ticket dropdowns while keeping historical tickets intact.

## 4. Manage organization-wide tags

Open **Settings → Tags** to create color-coded labels that can be applied across any board. The table shows how many tickets carry each tag so you can clean up unused tags easily. When staff filter the ticket list by multiple tags, Marula matches tickets carrying **every** selected tag (`AND` filtering).

## What you learned

- You can rename board columns and issue categories to match how your team talks without breaking SLA timers, automations, or reports.
- Moving a ticket from one board to another automatically maps its status through the four core categories and logs the move in the audit trail.
- For adding custom data fields to boards or clients, see [How to customize without code](customize.md).
