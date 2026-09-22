# How to author automation rules

> **Audience:** admin — rule authoring, test-runs, and enable boundaries
> are settings-owner work.

## Prerequisites

- Admin access to **Settings→Automations** (and, for alert routing,
  Settings→Alerts).
- A trigger and an action in mind that both exist in the product's typed
  menus — the builder invents nothing you have not picked from a list.
- If the action is an agent: an approved, run-once-verified agent — see
  [the agent tutorial](../tutorials/first-agent-run.md).

## The mental model (why authoring is safe)

Automations are declarative trigger/action rows — the same
customizations-are-data doctrine as boards and fields, which is what makes
them migratable and fork-safe (the full rationale lives in
[customizations are data](../explanation/customizations-are-data.md)).
Authoring is therefore not code: you pick typed triggers and typed
actions, and the row is data the upgrade machinery can move.

## 1. Build the rule

Open Settings→Automations. From the rule list, start a rule and select:
- the **trigger** from the typed event menu, and
- the **action(s)**, again from typed menus — invoking an agent is an
  ordinary action on the same list.

## 2. Test-run before enable (always)

The rule editor carries a **dry-run test** that runs the rule against
seeded/sampled events and returns a **proposal-shaped result**. The editor
renders that result — it never fakes a run. Read what would have happened
before enabling. This is the same dry-run-first philosophy the imports
surface enforces ([dry-run imports](admin-imports.md)).

## 3. Enable, and scope to the org's risk posture

Enable the rule. Disabled rules stay in the list untouched — disabling is
a state change, not deletion. Alerts triggered by rules route as in the
[alerts how-to](#where-actions-meet-alerts) below.

## Where actions meet alerts

**Settings→Alerts** is the alert queue's home: alert triage with
ack/resolve states, and an alert-source configuration link. Acknowledge
and resolve are first-class states on the alert model, so an automation
that fires alerts hands its downstream to a queue with real workflow,
not a notification flood.

## What you learned

- Typing is the safety: both trigger and action are product-known
  constructs, migrated as data.
- Dry-run first, enable second — the product enforces the habit, the
  how-to preserves it.
- If a rule's action involves agents or AI surfaces, the same
  deny-by-default grant model applies (master switch, per-surface
  grants) — [manage AI access](admin-ai-access.md).
