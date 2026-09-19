# ADR 0010 — Decision-model harness: provider-general structured-decision edge, Jev as first adapter

**Status:** Accepted (user directive + research session) · 2026-09-18
**Scope:** new decision-model provider edge beside the ADR 0008 completion edge,
nine decision surfaces, org-level AI on/off posture, automation-rule integration,
and the BYOK-style credential path for decision models
**Companion research:** TypeSafe Jev docs (docs.typesafe.ai), System One model
launch analysis, cost-benefit per surface (session record 2026-09-18)
**Depends on:** ADR 0008 (`ai_surfaces` registry, sealed `ModelCredential` slot,
`ai_usage_ledger`/`ai_caps`, deny-by-default grants)

## Context

Jev (TypeSafe AI, released 2026-09-15) is the first of a new model class —
"System One" models — that do not generate text. The interface is
`POST /v1/systemone` with `{ state, model, questions }`: state is a string or
JSON object of the content to judge; questions are typed (Choice over ≤255
options with full probability distribution + computed confidence; Score over an
org-defined ordered rubric; Noul yes/no probability). Every question evaluates
in parallel and in isolation against the same state — adding questions barely
changes latency, so fan-out is effectively free. Latency is 70–500 ms and price
is $42/Btok input ($0.042/Mtok; output-free) — one to two orders of magnitude
below frontier-LLM structured output for the same judgment, with no parsing and
no hallucination surface. Limits of the class (design-relevant): text-only
state (no images), ~32k-token state+longest-question budget, confidence is
provider-defined but probabilities are returned raw, non-English accuracy is
lower, output is constrained to the supplied answer space (no free-form JSON).

This is exactly the shape of many decisions inside a PSA: ticket triage and
routing, alert actionability, urgency/severity/friction scoring, KB-answer
relevance, LLM-output verification, billing-description audit, record dedupe.
Today those are either deterministic-only (automation rules match conditions on
a snapshot) or would burn an LLM call per judgment.

Two binding user directives shape the decision:

1. **Meridian must still work for orgs who run "old school."** Orgs can turn AI
   features off entirely, or control which features are active and visible.
2. **Harness, not integration.** Jev is the test-case model; the seam must be
   provider-general so future decision models swap in without breaking
   call sites, and input structure must be customizable per org.

## Decision

### 1. A second AI edge: the `DecisionProvider` harness (mirror of ADR 0008's completion edge, not an extension of it)

- One **decision edge** in core: resolve caller/org → decision surface →
  enabled? granted? provider configured? caps OK? → compose state per the
  surface's template → dispatch typed questions → typed answers + thresholds →
  side effects (suggestions / automation actions) — with every call in
  `ai_usage_ledger` and confidences/answers auditable. Same posture as ADR
  0008: degrade-never-5xx, deny-by-default, audit everything.
- **Provider abstraction is a trait + provider-kind descriptor, not a TypeSafe
  client.** `DecisionProvider` speaks the *shape*: JSON-able state in, typed
  question map in, typed answers (choice/score/noul with probabilities +
  confidence) out. Kind `system_one` ships first — the TypeSafe adapter
  (`POST /v1/systemone`, `jev-*` models, 429/529 exponential backoff) is a
  **test case for the seam**. Future decision models (or self-hosted/
  LLM-backed judges constrained to the same shape) plug in as new kinds or via
  the generic adapter; call sites never see a vendor type.
- **Credentials ride the existing sealed slot.** A Jev/system-one key is a
  sealed `ModelCredential` (api_key kind) via the existing BYOK paths — org or
  personal — with resolve-personal-first semantics unchanged. Provider name
  `typesafe` joins the provider options; the model catalog gains a
  `decision` capability marking which models answer question maps instead of
  generating text.
- **Model pinning is explicit.** Decision surfaces pin a versioned model id
  (`jev-1.13.0`, not only `jev-latest`); TypeSafe's own docs recommend pinning
  when thresholds are tuned. The catalog stores the pinned id; answer
  provenance records which model answered.

### 2. Input structure is customizable per org — state templates as data

- Each decision surface config carries a **`state_template`** (JSON template
  composing entity fields into the vendor's state shape — the same
  `shape_fields` mapping discipline the connectors use) plus atomic
  **questions** (`type`, `instructions`, `criteria`, paths into the template),
  per-answer **thresholds** (act / review / ignore) and a **decision model
  pick**. Admins edit these as data; no code per org. Dot-path references into
  the template (`ticket.messages[0].text`) work because the template composes
  the snapshot, mirroring the documented Jev state pattern.
- Templates are validated against a size guard (~32k state budget): refusal
  with a typed error, never silent truncation.
- **Starter question-set schemas ship per surface as versioned seed data.**
  Each surface keys a curated default question map — the Choice options, Score
  rubric levels, Noul criteria, instructions, and default thresholds Meridian
  sends when an org hasn't customized. Versions improve over time (curated
  rubric updates ride migrations/settings bumps) while org-customized sets
  stay pinned to the version their admin accepted; lanes 2–3 build their
  fan-outs ON these schemas, never ad-hoc.
- **Customization is fork-on-first-edit, stored as org-owned versioned
  config.** An untouched surface's config *references* the starter set
  (`starter@v1`); the first org edit copies the set into an org-owned
  versioned config and marks it customized, pinning provenance. If Meridian
  later curates `starter@v2` (better rubric wording), the org sees "v2
  available" with an honest diff — adoption is opt-in, and a customized schema
  is NEVER silently overwritten by an upstream bump (the fork is what keeps
  their thresholds calibrated against wording they accepted). The worker and
  automation engine read the org's active config at decision time; the org's
  custom question map is exactly the caller-supplied `questions` input — the
  starter is only the default sent when an org hasn't customized.

### 3. Nine decision surfaces ship in wave 1 of the harness (priority-ordered)

Surfaces are registered in `ai_surfaces` (kind `decision` — new kind value,
internal vs grant-targeted per row) with keys:

| Key | Surface (research #) | Consumer | Posture |
|---|---|---|---|
| `decision_ticket_triage` | #1 inbound-email + ticket triage (urgency, actionability, board/queue routing, tags, priority, frustration) | native | suggestions-by-default; state-template-driven; apply-threshold gated, else pending suggestion |
| `decision_automation_conditions` | #2 semantic conditions inside automation rules — new `ai_decision` condition kind referencing (surface, question, operator, threshold, confidence) | native | answers feed existing condition/action engine; malformed rule config skips-with-warn like today |
| `decision_workbench` | #3 workbench/queue suggestion card (one fan-out on ticket.created/updated) | native | inline card, 70–500 ms budget, optimistic UI; pending state when below threshold |
| `decision_alert_triage` | #4 RMM/alert actionability + severity (every alert — LLMs are uneconomic here) | native | auto-ack/ignore only above high thresholds; else leaves the alert untouched |
| `decision_search_rerank` | #5 rerank top-k retrieval candidates (Noul per candidate: "does this answer/relate?") | internal | rides existing Postgres retrieval; never replaces it |
| `decision_llm_verify` | #6 cheap verification of LLM-surface outputs (note/email/doc drafts vs source; injection/policy checks) | internal | tiered: Jev Noul first, escalate borderline to an LLM judge — exactly the confidence-gated pattern |
| `decision_billing_audit` | #7 time-entry/invoice-line description audit (billable? matches work?) | native | flags at creation; feeds existing approvals |
| `decision_record_dedupe` | #8 entity dedupe residual-matching after deterministic blocks | native | suggestion-grade only (merges stay human-approved) |
| `decision_dispatch` | #9 dispatch queue assignment + escalation urgency | native | suggestion + optional automation application |

Not decision surfaces (stay completion/LLM): general chat, note/doc/email
drafting — Jev-class models cannot generate text at all.

### 4. Old-school orgs: AI visibility is an explicit, layered off-switch

- **Org master switch** `ai.features_enabled` (org setting, default **OFF**).
  When OFF: no decision calls, no completion calls, no suggestion cards, no AI
  entry points — the org sees exactly the pre-AI product. This switch gates the
  completion edge as well (one switch for all AI, per the directive).
- **Per-surface control** on top: surface `enabled` flag + existing
  deny-by-default grants decide which features are active and visible;
  disabled/granted-to-nobody surfaces render nothing (visibly-absent doctrine,
  never dead buttons).
- Failure posture when the provider errors/keys are missing: suggestion
  surfaces go quiet (workflow unaffected), automation `ai_decision` conditions
  evaluate as no-match with a warn — identical diagnostics to malformed rules
  today. A downed model must never block ticket intake, alerts, or billing.

### 5. Confidence architecture is code-owned

- Gate order (docs' confidence-gated routing): high confidence → act (subject
  to per-answer thresholds); medium → hold as suggestion/pending for review;
  low → ignore except as a recorded signal. Thresholds live in surface config,
  editable per org. Noul has no separate confidence — its probability *is* the
  signal; a Noul ≈ 0.5 routes to review, never acts.
- **Shadow mode** is a first-class mode per surface: run, record answers +
  model + confidences into a decision-answers audit store, take no action.
  Landing behavior: surfaces ship shadow-first; the admin flips
  shadow→live after reviewing their own data (thresholds are per-org because
  model calibration is per-domain).

### 6. Caps, ledger, and abuse posture ride ADR 0008 unchanged

Every decision call records (org, user/bot principal, surface, provider, model,
input tokens) in `ai_usage_ledger`; `ai_caps` (hard) bound dispatch with the
same degrade-never-fail semantics; per-org rate-limit handling is exponential
backoff + honest refusal logged, never a busy-loop.

## Consequences

- Lane scope (**Wave 23** — fires after Wave 18 lanes merge; the harness sits
  on top of the 0081 matrix/credential/ledger schemas):
  `p23-decision-harness` (edge + data model + system_one/Jev adapter + kill
  switch + shadow store), `p23-decision-triage` (surfaces #1–#4 + #9 + the
  `ai_decision` automation condition), `p23-decision-auxiliary` (surfaces
  #5–#8), `p23-ai-closeout` (settings "AI features" controls, per-surface
  decision config UI + preview, suggestion cards, cap/usage views, vision
  review). Migration numbers head-ordered after Wave 18's 0082 (coordinator
  keeps order).
- The old-school guarantee is testable: a default-org flow test proves zero AI
  side effects with the switch OFF (no ledger rows, no cards in captured UI,
  automations behave identically).
- Vendor risk is contained: any future System One-class model (or an
  LLM-constrained judge) is a provider-kind + catalog entry; surface configs
  and call sites do not change. If TypeSafe pricing/limits move (early-access
  "= dynamic"), the seam's vendor knowledge is one adapter crate.
- Non-English-heavy orgs get a calibration note in the admin UI (Jev-class
  accuracy is lower on CJK per vendor docs); thresholds are per-org, so local
  tuning is the answer, not a code fork.
- Re-evaluation triggers: second decision-model vendor shipping (proves the
  abstraction), first self-hosted judge deployment, TypeSafe pricing/limits
  stabilization.
