# AI: deny-by-default and the decision harness

> **Audience:** both — admins decide; every staff member experiences the
> posture in daily surfaces (grant required, quiet absent, typed
> unavailable states instead of 500s).

## The default posture: deny-by-default, master switch off

`Settings→AI`'s banner: "Availability, grants, caps, credentials and the
decision harness — deny-by-default across the org". Below it the org
master switch, **AI features**, default **OFF**, with its own guarantee:

> "OFF keeps the exact pre-AI product — every AI surface answers quietly
> with zero ledger rows; flipping ON lets the per-surface grants and
> configs below decide what is actually audible."

Three facts distinguish this from a feature flag:

1. **Per-surface grants decide audibility**, not the org flip. Turning
   the switch ON changes nothing audible until grants exist. Surfaces are
   native (`doc_drafting`, `email_drafting`, `general_chat`,
   `ticket_note_assist`) or plugin (`automation_writing`,
   `dashboard_generation`, `data_analysis`, `transcription`) — both
   default denied.
2. **OFF is the panic switch.** With zero ledger rows and zero side
   effects, disabling the org's AI later returns you to the pre-AI
   product exactly, not to a degraded variant ([the how-to for the
   switch](../how-to/admin-ai-access.md)).
3. **The caps regime is pre-admission**: every configured period binds at
   admission (hard-cap degrade is typed — never a 5xx, never a silent
   raise of your own costs).

## Why the switch exists: the same product with and without AI

Marula ships an AI product that can be **absent** — the same org
running it OFF gets the identical pre-AI behavior: no surface, no ledger
rows, no side effects. Orgs that never planned to use AI keep the exact
same product forever, not a "reduced" tier. This is
[ADR 0010](../../adr/0010-decision-model-harness.md)'s "old-school orgs
keep the pre-AI product behind the org master switch default OFF" — a
deliberate retention point for conservative orgs, and the strongest kind
of guarantee the AI work could ship: absence, provably.

## The decision harness: typed questions, shadow-first, silent failures

Decision surfaces (the routing/triage/verification family — the "Choice
over the granted MCP tool inventory" routing edge of ADR 0011) do not
chat. They send **typed state** into a `DecisionProvider` seam (the first
adapter is the `system_one` provider) and receive **typed answers**:
choice, score, noul (a confidence level), probabilities per question,
model id, confidence — all shadow-cataloged.

Three properties make the harness safe to enable per org:

- **Schema-validated state**: the state template validates against its
  budget before first send, a typed refusal rather than silent
  truncation. No free-form text questions, no arithmetic in question
  criteria, no criteria composed from untrusted state. Admin-owned
  versioned criteria only.
- **Shadow-first posture**: decisions land in a shadow store carrying the
  **full normalized probability distribution per question**, the model
  id, confidence, and mode. Before a decision surface acts on anything,
  its answers are visible — and attributable by model version.
- **Fail-closed when unconfigured**: "no decision model ⇒ the surface is
  silently quiet, with typed not-available states" (ADR 0011 §3). The
  admin UI carries the "decision model not configured" note. No degrade
  by silence, no refusal by HTTP 500.

## The agent-side cousin (and why drafts must verify)

Agent tool execution rides a sibling seam and an agent ledger of its own
([the tutorial](../tutorials/first-agent-run.md) covers the 7-step
ladder). Its guard is the same harness-shaped bar: **verification
threshold** gates dispatching agent-surface drafts — a draft scoring
below threshold halts dispatch and queues for human review. The loop
termination function (ADR 0011 §4) exists to stop agents "early" rather
than to extend the hard iteration cap that harness code enforces
independently. Noul enables exit, never extends the cap.

## What this buys you at every layer

- **Admin**: you grant per surface, caps enforce, and OFF is provably the
  pre-AI product (zero ledger rows is checkable, not claimed).
- **Staff**: granted surfaces appear and ungranted surfaces do not.
  There is no in-between "partially enabled" state.
- **Auditors**: the decision answers are shadow-cataloged with full
  distributions and model version, and their writes are visible as
  `agent` rows in provenance —
  [provenance vs audit](provenance-vs-audit.md).
