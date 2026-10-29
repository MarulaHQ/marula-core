# ADR 0011 — Agentic tool-routing edge: harness-owned Choice gate, not a tool the model asks for

**Status:** Accepted · 2026-09-18
**Scope:** the tool-calling control plane on top of ADR 0010's decision edge,
which owns tool choice. The build-vs-adopt boundary and the no-decision-model
posture complete the set. The binding anti-patterns and restrictions of the
decision-model class are part of it.
**Depends on:** ADR 0008 (`CompletionProvider` edge, `ai_surfaces` registry,
sealed `ModelCredential` slot, `ai_usage_ledger`/`ai_caps`, ChatRunner tool
loop with MCP reads + IF8 proposals) and ADR 0010 (`DecisionProvider`
harness, state templates, confidence architecture, shadow mode)

## Context

ADR 0008 ships the agent harness's tool loop (ChatRunner with
MCP-catalog-derived read tools behind the capability gate, writes via IF8
proposals), and ADR 0010 ships the decision edge beside it. Tool *choice*
was left unassigned: today it is either the model's own uncalibrated
judgment or deterministic rules. The TypeSafe Jev research positions three
control-loop pillars for agentic execution: **MCP Tool Routing** (Choice
over the tool inventory, ~100 ms), **Loop Termination Check** (Noul asking
"has the task satisfied its goal criteria?", which prevents recursive
execution loops), and **Generative Response Verification** (Noul/Score on
drafts before dispatch, tiered to an LLM judge at borderline confidence).
ADR 0010 covers the third. The first two are missing surfaces. The same
research sets hard class boundaries: no text generation, no temporal or
ordinal math, criteria must stay isolated from untrusted state, and Choice
caps at 255 options.

Three questions were raised explicitly, and this ADR answers them. Is the
decision engine its own tool that models call to ask for tools? No, and the
shape is rejected as an anti-pattern. Does the tool-calling logic come from
another open-source project, or is it built? It is built in core, with a
narrow open-source adoption boundary. What happens when the admin has no
decision model configured? Today's behavior, honestly applied.

Three commitments carry over from ADR 0010. Orgs run "old school" with zero
AI side effects when AI is off. The seam is harness-not-integration:
vendor-general, with per-org input as data. The edge degrades honestly and
never 5xxs a human workflow.

## Decision

### 1. Tool choice is a harness-owned pre-flight gate, not a tool

- **The decision edge sits in front of the tool loop, not inside it as a
  callable tool.** The model never sees "ask the decision engine" in its
  tool list. Making routing itself a model-callable tool reintroduces the
  exact uncalibrated judgment the class exists to remove ("when should I
  ask?" is superstition, not calibrated probability). It stacks one extra
  reasoning round-trip of latency in front of every pick. And it turns the
  tool inventory into model-facing prompt surface that middleware was
  supposed to eliminate.
- **Gate flow:** trigger (agent step boundary, ticket/alert event, rule
  evaluation) → compose state per the surface's template → dispatch typed
  questions in ONE speculative fan-out (tool-routing Choice +
  loop-termination Noul + any verification Noul/Score, and extra questions are
  ~free per ADR 0010) → confidence gate. **High** confidence: the harness
  executes the chosen tool directly and feeds the result into the loop.
  **Medium**: execute, and record a pending-review note. **Low**: fall
  through to the model's own tool choice (the ADR 0008 loop exactly as
  shipped) with the decision answer recorded as an audit signal. Writes
  always stay IF8 proposals regardless of who picked the tool.
- **The gate can only narrow, never widen.** The Choice inventory is the
  caller's already-granted tool set (capability gate applied first), so
  routing adds no new privilege path. Tool results and provenance land in the same ledger/audit rows as
  model-picked calls, and so does usage.
- **New surfaces** register in `ai_surfaces` (kind `decision`, internal):
  `decision_tool_routing` (Choice over the granted MCP tool inventory),
  `decision_loop_termination` (Noul at agent step boundaries), and, from
  the same research, `decision_contract_scope` (native, suggestion-grade:
  MSA coverage classification incl. after-hours surcharge Noul, which rides the
  existing `contracts` schema). ADR 0010's nine surfaces become twelve,
  and these are lower priority than the original nine.

### 2. Build the seam in core, with the open-source adoption boundary protocol-level only

- The loop, the grants, the caps, the ledger, and the degrade posture already
  exist in core
  (ADR 0008). The only new code is the vendor-general choice seam, and it
  must speak Marula's `DecisionProvider` shape, not an executor
  abstraction from an agent framework.
- **Protocol-level libraries are fine where already used or needed** (the
  MCP client/server protocol, JSON-Schema tool description, the
  `mcp-server` crate's territory). **Executor/agent frameworks are
  rejected.** LangGraph-style runners and AutoGPT-style autonomous loops
  would own the loop and bypass the gate chain (surface → grant → caps →
  ledger). They also fight the deny-by-default, degrade-never-fail posture.
  The harness is small, and the posture is the product.
- New decision-model vendors plug in as provider kinds (ADR 0010 §1), and
  tool routing changes nothing at call sites.

### 3. No decision model configured, or downed: exactly today's behavior, honestly

- Unconfigured or failed decision edge: the tool loop runs precisely as
  ADR 0008 ships it, with the model picking tools in its own loop.
  `ai_decision` automation conditions evaluate no-match with a warn
  (identical diagnostics to malformed rules). Suggestion surfaces render
  nothing (visibly-absent, never dead buttons). No ledger rows, no cards,
  no shims.
- The admin UI shows a per-surface "decision model not configured" honest
  note stating what activates when configured, same posture as the
  completion edge.
- Caps exhausted or provider 429/529: hard caps bound dispatch, exponential
  backoff with an honest refusal logged. Never a busy-loop, never a 5xx
  into a human workflow. Intake, alerts, billing and chat loops never
  fail because the decision edge is unconfigured or down.

### 4. Class restrictions and anti-patterns (binding on all decision surfaces)

These are validation-time rules for surface configs, state templates, and
question sets. They are enforced by the harness, not advisory:

- **No free-form text through the decision edge.** Typed primitives only,
  and drafting/summarizing/chat stay completion surfaces. Jev-class models
  produce no rationales, so the audit trail is recorded distributions,
  not prose.
- **Temporal/ordinal math is pre-computed in code.** State templates
  compose derived fields (`after_hours`, `within_sla_window`,
  `minutes_since_created`, `overage_minutes_remaining`). Raw timestamps,
  durations and collections that require elapsed/order/math reasoning are
  never passed for the model to interpret (the class treats dates as
  plain strings). Template validation rejects non-derived temporal leaves,
  and question criteria requiring elapsed-time or ordering judgments are
  rejected at config time.
- **No arithmetic or multi-step logic inside questions.** Sums, counts and
  comparisons happen in code and enter state as explicit fields. Criteria
  demanding math are rejected at config time.
- **Atomic composite scoring.** Complex judgments decompose into separate
  primitive questions (technical complexity, user impact, deadline
  urgency) combined deterministically in application code, never one
  synthesized "overall severity" question.
- **Criteria stay isolated from untrusted state.** Criteria and
  instructions are admin-owned versioned config (ADR 0010 §2), never
  composed from, appended to, or echoed into untrusted content (inbound
  email bodies, alert payloads, customer chat). Untrusted text rides state
  only. Injection can only move an answer through vague criteria, so
  criteria quality is a curation problem with per-org thresholds as the
  answer, never a reason to interpolate state into criteria.
- **Choice beyond 255 options is a tiered tree, never a flattened list.**
  Large inventories (tool catalogs, board/tag taxonomies) fan out
  hierarchically: a level-one category Choice, then a level-two item
  Choice. The second pass costs ~nothing per ADR 0010's fan-out economics.
- **Speculative fan-out is the default shape.** All questions for one
  trigger ride one request. Per-question HTTP calls are an anti-pattern
  that reintroduces the latency stack for zero benefit.
- **Full distributions recorded for audit and calibration.** The decision
  answers store records the complete normalized probability vector plus its
  confidence and the pinned model id per question (rationales do not exist).
  Shadow mode (ADR 0010 §5) applies to tool routing: surfaces ship
  shadow-first, and admins flip shadow→live after reviewing their own
  data.
- **Noul exits loops early, and code caps them.** Probabilistic termination is
  thresholded high (≈0.5 routes to review, never acts), and a hard
  iteration cap in harness code remains the actual infinite-loop
  prevention. Noul enables early exit, and it never extends the cap.

## Consequences

- The tool-routing edge lands inside the decision-harness build (the gate,
  the Choice inventory, and the tiered-tree helper). The triage build
  carries `decision_contract_scope`, and the auxiliary build carries
  `decision_loop_termination`. Loop-termination wiring into the ChatRunner
  lands with the first multi-step agent execution work.
- The old-school guarantee extends: with the decision edge unconfigured,
  tool loop behavior is bit-identical to the ADR 0008 ship state, provable
  by the same zero-side-effect flow test (no ledger rows, no cards,
  identical automation behavior).
- No new privilege path: the routing inventory is a filtered view of
  already-granted tools, so the capability-gate test suite applies
  unchanged to gate-routed calls.
- Vendor risk stays bounded to the adapter (ADR 0010), and work is paid in
  input tokens only: routing at ~100 ms replaces a multi-second model
  round-trip per step in the hot loop.
- Review triggers: the first multi-step autonomous agent execution shipping
  (loop termination against real loops), and a decision-model vendor
  shipping single-pass Choice beyond 255 options (the tiering rule
  revisits). A third trigger is TypeSafe shipping rationale/explanation
  outputs, which upgrades the audit posture from distributions to
  rationales.
