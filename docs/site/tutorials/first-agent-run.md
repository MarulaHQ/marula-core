# Tutorial: your first agent run

> **Audience:** admin — enrollment and grants are admin settings. The
> run itself produces evidence anyone can read.

An agent in Meridian is not a chat toy left loose in your tenant. Its tool
runs are always scoped to a client, cataloged on its ledger, and stay
reviewable after they finish. This lesson takes one agent from enrollment
to a reviewed ledger entry, on the seeded demo org.

## Prerequisites

- Admin access (Settings is yours).
- The org's master AI switch awareness — see step 1 of the access part or
  the [deny-by-default explanation](../explanation/ai-deny-by-default.md).

## 1. Enroll the agent

Open **Settings→Agent platform**. The page centers on the **agent ledger**
table (Agent, Session, Status, Evidence, Perspective, Actions) and states
its lifecycle in seven steps, which is exactly the path of this tutorial:
enroll → show the workbench → plan & approve → run once → wire to
automations → connectors → review the ledger.

Step 1: enroll by pasting an enrollment token. Nothing runs yet — enroll
places the agent on the ladder. It cannot act until a plan exists.

## 2. Show the workbench and plan

Give the agent a session and a workbench. The agent produces a **plan**: the steps it intends to take, laid out.
Agents plan. Humans approve.

## 3. Approve, then run once

Approve the plan (step 3), then manually trigger one run (step 4). The run
executes against its scoped client, and every tool call lands as evidence
rows on the agent ledger. This run-once gate exists so you can see exactly
what an agent's automation wiring would do later, with a human behind the
trigger.

## 4. Wire to automations and connectors (later, deliberately)

Steps 5–6 of the ladder are where the run-once discipline pays off: once
you trust the evidence, attach the agent to automations and connectors.
Automation rules can invoke agents just like any other action — see
[automation rules](../how-to/admin-automation-rules.md).

## 5. Read the ledger

Open **Settings→Agent platform** and inspect the ledger rows: agent,
session, status, evidence, perspective. Provenance is field-level, so any
record an agent touched shows `agent` in the actor chips of
[/provenance](../explanation/provenance-vs-audit.md) — alongside
`human` and `import`. Agents never blur into history, and their writes
stay attributed and reviewable after completion.

## Turning an AI surface on for one role (companion task)

While you are in Settings, note the split: Settings→Agent platform governs
agents. Settings→AI governs the org's AI surfaces. To switch one surface on
for one role: open Settings→AI, confirm the org-wide **AI features**
master switch is ON (default OFF), then grant the surface
(doc_drafting, ticket_note_assist, …) to the role. Only then does it
become audible for those users. Full procedure: [manage AI access](../how-to/admin-ai-access.md).

## What you learned

- The 7-step ladder is the canonical agent sequence. Nothing executes
  without a plan, an approval, and a run-once observation.
- Agent tool runs are client-scoped and ledger-cataloged, forever.
- Master switch OFF means zero ledger rows and zero side effects — the
  pre-AI product, exact.
