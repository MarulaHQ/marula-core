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
- [Connect an IdP (SSO)](admin-sso.md) — the IdP handoff round-trip
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
- [Get help inside Meridian](get-help.md) — the ? help panel, guide
  search, and turning feature tours off

## Client (portal user) guides

Audience tag: **both** (admin hands them the start; steps are client-side).

- [Portal client-side tasks](client-portal-tasks.md): the four portal
  sections (Requests, Invoices, Knowledge, Schedule) from the client's
  side

Not documented on purpose: mail configuration — Settings→Mail is an
intentional placeholder until the mail administration wave lands
("no configuration here changes behavior today").
