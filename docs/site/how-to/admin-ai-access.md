---
description: "Marula treats AI as an optional tool that you control down to each role and screen."
---

# How to manage AI access

> **Audience:** admin — the master switch and per-surface grants are
> settings-owner decisions.

Marula treats AI as an optional tool that you control down to each role and screen. Out of the box, the organization-wide **AI features** master switch is **OFF**. When you turn it on, every individual AI helper (such as ticket note drafting or voice transcription) still starts disabled until you grant it to specific roles under **Settings → AI**.

## Prerequisites

- Admin access to **Settings → AI**.
- A quick read of [Why AI is deny-by-default](../explanation/ai-deny-by-default.md) if you want the full background on how permissions, rate limits, and model credentials fit together.

## 1. Check the organization-wide master switch

At the top of **Settings → AI**, the **AI features** master switch defaults to **OFF**:

> "OFF keeps the exact pre-AI product — every AI surface answers quietly
> with zero ledger rows; flipping ON lets the per-surface grants and
> configs below decide what is actually audible."

When the master switch is **OFF**, no AI buttons appear in the workspace, no prompts are sent to any model, and zero AI ledger rows are written. Flipping this switch **OFF** at any time acts as an instant kill switch across the entire organization.

## 2. Grant specific AI helpers to roles or users

Once the master switch is **ON**, use the **Surfaces** table to choose which tools are active and who can use them. Every surface defaults to denied until you add a role or user grant:

- **Built-in writing and chat helpers:** `doc_drafting`, `email_drafting`, `general_chat`, and `ticket_note_assist`.
- **Plugin and workflow helpers:** `automation_writing`, `dashboard_generation`, `data_analysis`, and `transcription`.

To turn a helper on for your service desk:

1. Find the surface row (for example, `ticket_note_assist`).
2. Add the role or user grant under **Role/User grants**.
3. Save. Users without a grant for that surface will never see its button in the UI.

## 3. Configure decision models (routing and triage)

Lower on **Settings → AI**, the **Decision harness** section configures structured evaluation models used for ticket triage and routing. If no decision model is configured, those features stay cleanly off and report `"decision model not configured"` rather than failing with an error.

## 4. Set usage caps and model credentials

- **Usage caps (`ai_caps`):** Set token or request limits per time window so a busy week never runs up an unexpected model bill. When a cap is reached, the surface pauses gracefully with a clear status message.
- **Usage ledger:** Every AI call records its surface and actor alongside the token count in the usage ledger.
- **Model credentials:** Store your API keys in the encrypted credential slot (at the organization level or per user) or point Marula at your own local hardware using [How to run transcription and embeddings on your own hardware](admin-local-ai-models.md).

## What you learned

- Leaving the master switch **OFF** gives you a zero-AI workspace with no hidden background calls.
- Turning the master switch **ON** still requires granting each individual AI helper (`ticket_note_assist`, `transcription`, etc.) to the roles that should use it.
