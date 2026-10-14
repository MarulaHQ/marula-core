# Tutorial: your first agent run

> **Audience:** admin — enrollment and grants are admin settings. The
> run itself produces evidence anyone can read.

In many tools, turning on AI means letting a bot rewrite tickets or email clients behind your back. Marula works differently: AI is off by default, and any enrolled agent must lay out a plan, get a human approval click, and record every action on an append-only ledger scoped to a single client.

This tutorial walks you through enrolling an agent, reviewing its plan, running it once under your supervision, and inspecting the audit trail on the seeded demo org.

## Prerequisites

- Admin access so you can open **Settings**.
- A quick read of [Why AI is deny-by-default](../explanation/ai-deny-by-default.md) if you want the background on how master switches and role grants work.

## 1. Enroll the agent

Open **Settings → Agent platform**. The center of this page is the **agent ledger** table (**Agent**, **Session**, **Status**, **Evidence**, **Perspective**, and **Actions**). Above the table, Marula lists the seven-step safety ladder every agent follows:

1. Enroll the agent
2. Scope it to a workbench session
3. Review and approve its plan
4. Trigger a single supervised run
5. Wire it to automation rules
6. Connect external tools
7. Audit the ledger

Paste an enrollment token to complete step 1. Enrolling registers the agent's identity, and nothing executes yet.

## 2. Assign a workbench session and read the plan

Give the agent a ticket or workbench session scoped to one client. Instead of changing records immediately, the agent produces a written **plan** listing the exact steps and tool calls it wants to run.

Agents propose. People approve.

## 3. Approve the plan and run it once

Click **Approve** on the plan (step 3), then trigger a **single supervised run** (step 4).

The agent executes its approved steps for that single client while recording each tool call and its inputs and outputs as an evidence row on the ledger. Requiring a manual single run before you attach an agent to background automations lets you verify what it actually does on real tickets.

## 4. Connect to automations only when you trust the output

Steps 5 and 6 on the ladder let you wire a proven agent into your [automation rules](../how-to/admin-automation-rules.md) or external connectors. Even when triggered by an automation rule, client-visible replies and external side effects remain gated by your policy rules.

## 5. Inspect the audit ledger and field history

Return to **Settings → Agent platform** and click into the ledger row for your run. You can see which session ran, which client it was scoped to, and the exact evidence captured at each step.

Every field an agent updates also carries an `agent` actor badge in the [Provenance view](../explanation/provenance-vs-audit.md) right alongside `human` and `import` changes, so your team can always tell whether a person or an assistant updated a ticket field.

## Companion task: turning on an in-app AI helper for one role

Notice that Marula separates autonomous background agents (**Settings → Agent platform**) from in-app writing and triage helpers (**Settings → AI**):

1. Open **Settings → AI**.
2. Flip the organization-wide **AI features** master switch to **ON** (it ships **OFF**).
3. Grant a specific helper surface (such as `ticket_note_assist` or `doc_drafting`) to the staff role that should see it.

Roles without a grant never even see AI buttons in the interface. For the full walkthrough, see [Manage AI access](../how-to/admin-ai-access.md).

## What you learned

- Every agent follows the seven-step ladder: enroll, plan, human approval, and a supervised single run before any automation wiring.
- Agent runs are locked to one client at a time and leave a permanent, field-level audit trail.
- Leaving the master AI switch **OFF** removes all AI helpers and background runs cleanly.
