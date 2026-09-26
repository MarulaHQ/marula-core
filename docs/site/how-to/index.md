# Admin how-to guides

A how-to guide is task-oriented: it assumes you know what you want and
shows the safest, supported path to one goal. Concepts live in
[explanation](../explanation/index.md); system facts live in
[reference](../reference/index.md).

Every admin guide is tagged **Audience: admin** — you host or administer
Meridian and own its settings. Where staff or clients get the same page,
the tag is **both** or points to the staff/client catalog below.

## Admin (settings-owner) tasks

- [Bootstrap your first organization](admin-org-bootstrap.md) — sign-in and
  create-org live on the same page
- [Manage users and invites](admin-users-invites.md) — team invites,
  setup links, verification
- [Configure SLA calendars & escalation ladders](admin-sla-calendars.md)
- [Set workflow categories](admin-workflow-categories.md) — behavior binds
  to the semantic category, never the label
- [Author automation rules](admin-automation-rules.md)
- [Manage API keys](admin-api-keys.md) — show once → rotate by
  replacement → revoke
- [Set up the portal](admin-portal-setup.md) — users + board exposure
- [Run a dry-run import](admin-imports.md) — dry-run first, saved mappings
- [Record FX rates & read reconciliation](admin-fx-rates.md)
- [Configure the plugin consent flow](admin-plugin-consent.md)
- [Manage AI access](admin-ai-access.md) — org master switch + per-surface
  grants
- [Manage dashboards & report schedules](admin-dashboards-reports.md)
- [Turn on travel time & mileage reconciliation](admin-routing-travel.md) —
  provider config, day start points, warnings, reimbursements
- [Set up labels and label templates](admin-labels.md) — typed templates,
  Zebra/DYMO stock, QR deep links, and the print targets in browsers or ZPL
- [Set up toolboxes and dispatch defaults](admin-toolboxes.md) — kits,
  membership, box tags, and the mappings the day pack reads
- [Configure email](admin-email.md) — routing addresses, the signed
  webhook, SPF/DKIM/DMARC, transports, notifications, and the self-host
  runbook
- [Connect an IdP (SSO)](admin-sso.md) — the IdP handoff round-trip
- [Buy: vendors, purchase orders, receiving, bills](admin-purchasing.md)
  — the order lifecycle, receipts into stock, posted bills
- [Upgrade to the latest release](upgrade.md) — including the N−2 rule
- [Customize without code](customize.md) — custom fields, boards,
  automations
- Also relevant: the [self-host reference spine](../reference/index.md)
  and its runbooks.

## Staff (MSP practitioner) guides

Audience tag: **both** unless a guide is staff-only by nature.

- [Work a ticket](staff-work-a-ticket.md) — notes → approval → bill
- [Dispatch from the board](staff-dispatch.md)
- [Use the keyboard shortcuts](staff-keyboard-shortcuts.md) — the Alt
  command registry, the hold-Alt overlay, and the ? cheat sheet
- [Scan barcodes and QR codes](staff-scanning.md) — what a scan means on
  each surface, and why it never hijacks a form
- [Run stock and parts](staff-stock-and-parts.md) — receive,
  use parts on a ticket, thresholds, transfers
- [Check tools out, and run client loans](staff-tools-and-loans.md) —
  who-has-what, check-in, loan issue and return
- [Work a day pack](staff-day-pack.md) — trips, the grab list, one-tap
  checkout, and tomorrow's list tonight
- [Get help inside Meridian](get-help.md) — the ? help panel, guide
  search, and turning feature tours off

## Client (portal user) guides

Audience tag: **both** (admin hands them the start; steps are client-side).

- [Portal client-side tasks](client-portal-tasks.md): the four portal
  sections (Requests, Invoices, Knowledge, Schedule) from the client's
  side
