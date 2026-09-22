# How to manage AI access

> **Audience:** admin — the master switch and per-surface grants are
> settings-owner decisions.

## Prerequisites

- Admin access to **Settings→AI**.
- The postures of the org's roles scoped before flipping anything (grant
  after the switch, never before — see step 1).

## One sentence of doctrine, then four quadrants

> "Availability, grants, caps, credentials and the decision harness —
> deny-by-default across the org."

The page lives in four layers, and this how-to covers two of them
(availability and grants): the [decision harness
rationale](../explanation/ai-deny-by-default.md) explains all four.

## 1. Master switch: OFF is the org's pre-AI baseline

**AI features** (org-wide master switch) defaults **OFF**. Its own copy
states the guarantee:

> "OFF keeps the exact pre-AI product — every AI surface answers quietly
> with zero ledger rows; flipping ON lets the per-surface grants and
> configs below decide what is actually audible."

"Zero ledger rows and zero side effects" is the load-bearing clause —
turning the org OFF later is an honest panic switch, not a soft dim. Old
school orgs can run forever this way, and nothing else changes.

## 2. Turn surfaces on per grant, not per org_flip

With the switch ON, the surfaces table governs what is actually audible —
per surface, **Role/User grants** and **per-surface Configure**. Two
kinds of surfaces ship (both default denied):

- **Native surfaces**: `doc_drafting`, `email_drafting`, `general_chat`,
  `ticket_note_assist`.
- **Plugin surfaces**: `automation_writing`, `dashboard_generation`,
  `data_analysis`, `transcription`.

To switch one surface on for one role: locate the surface row, add the
role grant, save. Nothing becomes audible until the row's grant exists —
the switch alone enables the decision, grants decide audibility.

## 3. Decision surfaces: harness section below fold

Decision surfaces (the routing/triage/verification family behind labels
like Choice/Score/Noul) configure in the **harness section** lower on the
page, with its own note: "Decision surfaces configure below in the
harness section". Their posture when unconfigured is honest-typed:
"decision model not configured" — the surface stays quietly unavailable
rather than degrading. See the
[deny-by-default explanation](../explanation/ai-deny-by-default.md).

## 4. Feedback loop: usage, caps, ledger

AI request metrics ride caps (`ai_caps`) whose periods enforce at
admission over each configured window — degrade is typed, never a 500.
All usage lines land in the usage ledger. If a surface is granted to a
role and the ledger stays empty, something upstream (mail transport,
model credential) is worth checking — no silent refund of the grant's
cost, no fake silence either.

Credentials live in the sealed model-credential slot (org or personal,
personal resolved first) on the same page family.

## What you learned

- Master switch OFF is the pre-AI product exactly. ON makes grants
  decide.
- Deny-by-default on every surface. Each grant is per surface, never
  a deference to a global "apply to all".
- Unconfigured or undeployed decision surfaces fail closed and honest.
