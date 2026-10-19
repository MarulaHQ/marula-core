# How to customize without code

> **Audience:** both — admins declare the customization; staff experience the result.

Every MSP names its ticket columns, client fields, and routing rules a little differently. In Marula, you can add custom fields, rename board columns, and build automation rules directly in **Settings** without writing code or worrying that the next software update will break your setup.

## Prerequisites

- An administrator account with **Settings** access (or an API token with customization scopes).

## 1. Add custom fields to clients, contacts, tickets, or assets

You can add custom fields to core record types using a fixed set of validated field types:

- `text`, `number`, `date`, `bool`, `select`, `url`, and `email`

Create custom fields in **Settings** (or via `/api/v1/…custom-fields` in the [API reference](../reference/api.md)). If you install a plugin that provides a custom field validator, you can also attach JSON Schema validation to board fields.

## 2. Rename and reorganize board columns safely

Every status column on a ticket board maps to one of four core workflow stages (`open`, `in_progress`, `waiting`, or `closed`).

Because SLA timers and billing rules, along with automations and reports, bind to those four underlying stages rather than your display labels, you can rename columns or boards anytime without breaking a single report or SLA clock. When a technician moves a ticket to a different board (`POST /tickets/{id}/move-board`), Marula maps the status across the underlying stages automatically and records the move in the audit log. See [How to set workflow categories](admin-workflow-categories.md).

## 3. Build declarative automation rules

Automation rules are stored as structured trigger-and-action rows rather than brittle custom scripts. Configure and dry-run test them in **Settings → Automations** ([see How to author automation rules](admin-automation-rules.md)).

## 4. Why your customizations survive upgrades cleanly

Because custom fields, board workflows, and automation rules are stored as structured data rows ([see Why customizations are data](../explanation/customizations-are-data.md)):

1. Database migrations automatically carry your custom rows forward during version upgrades.
2. Running `marula upgrade --check` before an upgrade scans your customizations and warns you ahead of time if any rule or field references a feature scheduled for deprecation ([see How to upgrade](upgrade.md)).

## 5. What stays internal

Marula guarantees stability across four public boundaries:

- The REST and SSE APIs (`/api/v1/…`, `/api/events/stream`)
- Versioned domain events and webhooks
- The plugin manifest contract
- Declared customizations and `ops.v1_*` reporting views

Internal PostgreSQL table layouts and internal Rust crate modules are not customization surfaces and may change between releases.
