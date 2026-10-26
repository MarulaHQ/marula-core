---
description: "Every MSP runs its service desk a little differently: you have your own board column names, custom client fields, SLA rules, and ticket automations."
---

# Customizations are data, not code

Every MSP runs its service desk a little differently: you have your own board column names, custom client fields, SLA rules, and ticket automations. In Marula, every one of those customizations is stored as structured database rows tied to fixed underlying stages rather than custom scripts. That design choice ([`docs/upgrade-compatibility-policy.md`](../../upgrade-compatibility-policy.md) §5) is why you can customize your workspace freely and still upgrade to new versions without things breaking.

## Why traditional PSA customizations make upgrades painful

Heavily customizable platforms usually get a reputation for terrifying upgrades when customizations are built as custom code or brittle string-matching rules. The moment you rename a status column from `"In Progress"` to `"Bench Repair"`, any report or SLA script looking for the text `"In Progress"` silently breaks.

## How Marula separates your labels from system behavior

In Marula, every custom board status maps to one of four core workflow categories (`open`, `in_progress`, `waiting`, or `closed`).

SLA timers and billing rules, along with automations and reports, all bind to those four underlying categories rather than the display label on your board. When a service desk lead renames a column or adds a new `"Waiting on Vendor"` status mapped to `waiting`, the SLA clock knows immediately to pause. Your custom workflow changes never break reporting or billing.

## Two practical tools this gives administrators

Because your customizations are structured data rows:

1. **Customization manifest:** You can export a single versioned file describing everything your organization has customized (boards, custom fields, workflow categories, and automation rules). It serves as both human-readable backup documentation and the input to upgrade checks.
2. **Offline upgrade preflight (`marula upgrade --check`):** Before you upgrade your server, you can run the preflight check against your customization manifest and a database copy. It tells you ahead of time if any of your custom fields or rules touch a construct scheduled for deprecation.

## The trade-off

Storing customizations as structured data means you configure rules from validated building blocks rather than injecting arbitrary scripts into the ticket page. When you do need custom external logic, you build it cleanly through [plugin surfaces](../how-to/admin-plugin-surfaces.md) or the [REST API and webhooks](../how-to/call-the-api.md), keeping your core database easy to upgrade and [yours to leave with any day](why-marula.md).
