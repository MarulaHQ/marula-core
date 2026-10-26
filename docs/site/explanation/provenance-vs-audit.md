---
description: "When an owner or service lead asks \"Who changed this client's billing rate?\" or \"Who viewed that domain admin password yesterday?\", you need a straight…"
---

# Provenance vs audit: two append-only ledgers, different granularity

> **Audience:** both — staff read provenance during disputes; admins own
> both surfaces and the query posture.

When an owner or service lead asks *"Who changed this client's billing rate?"* or *"Who viewed that domain admin password yesterday?"*, you need a straight answer—not a shrug. Marula keeps two permanent, append-only history ledgers that answer those two different questions: **Provenance** (`/provenance`) tracks field-by-field value changes on records, while **Audit** (`/audit`) tracks security and administrative events across your organization.

## 1. Provenance (`/provenance`): Who changed this specific field?

Open `/provenance` whenever you are investigating a specific record (a ticket, client, contract, or asset) and want to see how a field got its current value:

> "field-level provenance query — who changed what, field by field,
> across every applied write — including agent sessions and imports…
> never rewrites history"

Every row in Provenance shows:

- The **Before → After** values for the field that changed
- The monotonic **sequence number** (`Seq`) and timestamp of the edit
- The **actor kind** chip: `human` (a staff member), `agent` (an AI assistant run), or `import` (a CSV/JSON batch or connector sync)

Use **Provenance** when a client disputes when a ticket priority changed, when you want to see whether a person or an import updated a device serial number, or when you need to trace every edit made during a batch import.

## 2. Audit (`/audit`): What security or admin events happened?

Open `/audit` when you are reviewing security, access, or system lifecycle events across the workspace:

> "Append-only ledger of security, administrative and domain lifecycle events for the org."

The Audit screen lets you filter by action category (`auth.*`, `secret.*`, `plugin.*`, `portal.*`, `billing.*`), actor ID, and date range. Each row shows `human` or `system` actor badges and expands to show full event metadata.

Use **Audit** during a security review to check login attempts, password vault reveals (`secret.*`), plugin consent grants (`plugin.*`), API key creations and revocations, or billing period sign-offs.

## Quick comparison

| Question | Use Provenance (`/provenance`) | Use Audit (`/audit`) |
|---|---|---|
| **What does it record?** | Every field-by-field value change (`Before → After`) | Security, login, vault reveal, and admin lifecycle events |
| **Actor badges** | `human`, `agent`, `import` | `human`, `system` |
| **Typical question** | *"Who changed this contract's hourly rate from $150 to $175?"* | *"Who revealed the firewall password on Tuesday?"* |
| **Best for** | Billing or ticket history checks and import troubleshooting | Security audits, compliance reviews, and access checks |

## Both ledgers are append-only

Neither Provenance nor Audit can be edited or cleared from the UI. New changes always append new rows, and actions taken by AI agents or bulk imports are tagged right alongside human actions so nothing ever happens off the books.
