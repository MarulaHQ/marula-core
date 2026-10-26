---
description: "Automation rules let your service desk handle repetitive routing and follow-ups automatically: for example, alerting a manager when a P1 ticket arrives…"
---

# How to author automation rules

> **Audience:** admin — rule authoring, test-runs, and enable boundaries
> are settings-owner work.

Automation rules let your service desk handle repetitive routing and follow-ups automatically: for example, alerting a manager when a `P1` ticket arrives, adding a triage checklist when an RMM alert triggers, or notifying an account lead when a low CSAT score comes in. Every rule can be dry-run tested against sample events before you turn it on.

## Prerequisites

- Admin access to **Settings → Automations** (and **Settings → Alerts** for inbound alert sources).
- If your rule invokes an AI agent, complete [Your first agent run](../tutorials/first-agent-run.md) first so the agent is enrolled and approved.

## Why automation rules don't break on upgrades

Instead of requiring custom scripts that break when an API field changes, automation rules in Marula are stored as structured trigger-and-action data rows ([see Why customizations are data](../explanation/customizations-are-data.md)). When you upgrade Marula, your rules migrate right alongside your boards and custom fields.

## 1. Choose a trigger and actions

Open **Settings → Automations** and create a new rule:

1. Pick the **trigger** event from the menu (such as `ticket.created`, `sla.breached`, `stock.level_low`, `csat.recorded`, or an event contributed by an installed connector).
2. Add optional **conditions** (such as priority, board, or status category). Rules can evaluate ticket events as well as board-scoped events that don't yet have a ticket attached.
3. Pick one or more **actions** to run when the conditions match (for example, post an internal note, assign a board or owner, send a webhook, or invoke an enrolled agent).

## 2. Run a dry-run test before enabling

Before turning a rule loose on live customer tickets, click **Dry-run test** in the rule editor.

The dry run evaluates your rule against sample events and displays a preview of the exact actions that would have fired, without modifying any tickets or sending any notifications. This follows the same preview-first safety model as [dry-run data imports](admin-imports.md).

## 3. Enable or pause the rule

Once the dry-run preview matches what you expect, toggle the rule **Enabled**. If you need to pause a rule later during maintenance, flip it back to **Disabled**; disabling pauses execution immediately without deleting your configuration.

## 4. Triage incoming infrastructure alerts

Open **Settings → Alerts** to manage inbound monitoring alerts (from RMMs, uptime monitors, or signed webhook sources). Alerts support **Acknowledge** and **Resolve** states and collapse alert storms by `dedup_key` so a flapping router doesn't flood your board with 200 identical tickets.

## What you learned

- Rules are built from typed triggers and actions (plus optional conditions) that survive version upgrades safely.
- Always run the built-in **Dry-run test** to verify what a rule will do before enabling it.
- If a rule calls an AI surface or agent, it still respects your organization's master AI switch and role grants ([see Manage AI access](admin-ai-access.md)).
