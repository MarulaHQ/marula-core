# ADR 0011 — Agentic tool-routing edge: harness-owned Choice gate, not a tool the model asks for

**Status:** Accepted (user directive + research session) · 2026-09-18
**Scope:** tool-calling control plane on top of ADR 0010's decision edge — who
owns tool choice, build-vs-adopt boundary, the no-decision-model posture, and
the binding anti-patterns/restrictions of the decision-model class
**Depends on:** ADR 0008 (`CompletionProvider` edge, `ai_surfaces` registry,
sealed `ModelCredential` slot, `ai_usage_ledger`/`ai_caps`, ChatRunner tool
loop with MCP reads + IF8 proposals) and ADR 0010 (`DecisionProvider`
harness, state templates, confidence architecture, shadow mode)

## Context

ADR 0008 ships the agent harness's tool loop (ChatRunner: MCP-catalog-derived
read tools behind the capability gate, writes via IF8 proposals) and ADR 0010
ships the decision edge beside it — but tool *choice* was left unassigned:
today it is either the model's own uncalibrated judgment or deterministic
rules. The TypeSafe Jev research (session 2026-09-18) positions three
control-loop pillars for agentic execution: **MCP Tool Routing** (Choice over
the tool inventory, ~100 ms), **Loop Termination Check** (Noul: "has the task
satisfied its goal criteria?" — prevents recursive execution loops), and
**Generative Response Verification** (Noul/Score on drafts before dispatch,
tiered to an LLM judge at borderline confidence). ADR 0010 covers the third;
the first two are missing surfaces. The same research sets hard class
boundaries (no text generation, no temporal/ordinal math, criteria must stay
isolated from untrusted state, Choice ≤255 options).

Three questions were raised explicitly and answered here: (1) is the
decision engine its own tool that models call to ask for tools — **no**, and
that shape is rejected as an anti-pattern; (2) does the tool-calling logic
come from another open-source project or is it built — **built in core, with
a narrow open-source adoption boundary**; (3) what happens when the admin has
no decision model configured — **today's behavior, honestly**.

Binding directives carried from ADR 0010: orgs run "old school" with zero AI
side effects when off; harness-not-integration (vendor-general seam,
per-org input as data); degrade-never-5xx.

## Decision

### 1. Tool choice is a harness-owned pre-flight gate, not a tool

- **The decision edge sits in front of the tool loop, not inside it as a
  callable tool.** The model never sees "ask the decision engine" in its
  tool list. Making routing itself a model-callable tool reintroduces the
  exact uncalibrated judgment the class exists to remove ("when should I
  ask?" is superstition, not calibrated probability), stacks one extra
  reasoning round-trip of latency in front of every pick, and turns the tool
  inventory into model-facing prompt surface that middleware was supposed to
  eliminate.
- **Gate flow:** trigger (agent step boundary, ticket/alert event, rule
  evaluation) → compose state per the surface's template → dispatch typed
  questions in ONE speculative fan-out (tool-routing Choice + loop-termination
  Noul + any verification Noul/Score — extra questions are ~free per ADR
  0010) → confidence gate → **high**: the harness executes the chosen tool
  directly and feeds the result into the loop; **medium**: execute + record a
  pending-review note; **low**: fall through to the model's own tool choice
  (the ADR 0008 loop exactly as shipped) with the decision answer recorded as
  an audit signal. Writes always stay IF8 proposals regardless of who picked
  the tool.
- **The gate can only narrow, never widen.** The Choice inventory is the
  caller's already-granted tool set (capability gate applied first); routing
  adds no new privilege path. Tool results, provenance, and usage land in the
  same ledger/audit rows as model-picked calls.
- **New surfaces** registered in `ai_surfaces` (kind `decision`, internal):
  `decision_tool_routing` (Choice over the granted MCP tool inventory),
  `decision_loop_termination` (Noul at agent step boundaries), and — from the
  same research — `decision_contract_scope` (native, suggestion-grade: MSA
  coverage classification incl. after-hours surcharge Noul; rides the
  existing `contracts` schema). ADR 0010's nine surfaces become twelve; lower
  priority than the original nine.

### 2. Build the seam in core; the open-source adoption boundary is protocol-level only

- The loop, grants, caps, ledger, and degrade posture already exist in core
  (ADR 0008). The only new code is the vendor-general choice seam, and it
  must speak Marula's `DecisionProvider` shape — not an executor
  abstraction from an agent framework.
- **Protocol-level libraries are fine where already used or needed** (MCP
  client/server protocol, JSON-Schema tool description — the `mcp-server`
  crate's territory). **Executor/agent frameworks are rejected**: LangGraph
 -style runners, AutoGPT-style autonomous loops, and similar would own the
  loop, bypass the gate chain (surface → grant → caps → ledger), and fight
  deny-by-default/degrade-never-fail. The harness is small; the posture is
  the product.
- New decision-model vendors plug in as provider kinds (ADR 0010 §1); tool
  routing changes nothing at call sites.

### 3. No decision model configured, or downed → exactly today's behavior, honestly

- Unconfigured/failed decision edge: the tool loop runs precisely as ADR
  0008 ships it — the model picks tools in its own loop; `ai_decision`
  automation conditions evaluate no-match with a warn (identical diagnostics
  to malformed rules); suggestion surfaces render nothing (visibly-absent,
  never dead buttons); no ledger rows, no cards, no shims.
- Admin UI shows a per-surface "decision model not configured" honest note
  and what activates when configured — same posture as the completion edge.
- Caps exhausted or provider 429/529: hard caps bound dispatch, exponential
  backoff + honest refusal logged, never a busy-loop, never a 5xx into a
  human workflow — intake, alerts, billing, and chat loops never fail
  because the decision edge is unconfigured or down.

### 4. Class restrictions and anti-patterns (binding on all decision surfaces)

These are validation-time rules for surface configs, state templates, and
question sets — enforced by the harness, not advisory:

- **No free-form text through the decision edge.** Typed primitives only;
  drafting/summarizing/chat stay completion surfaces. Jev-class models have
  no rationales — the audit trail is recorded distributions, not prose.
- **Temporal/ordinal math is pre-computed in code.** State templates compose
  derived fields (`after_hours`, `within_sla_window`,
  `minutes_since_created`, `overage_minutes_remaining`); raw timestamps,
  durations, and collections that require elapsed/order/math reasoning are
  never passed for the model to interpret (the class treats dates as plain
  strings). Template validation rejects non-derived temporal leaves; question
  criteria requiring elapsed-time or ordering judgments are rejected at
  config time.
- **No arithmetic or multi-step logic inside questions.** Sums, counts, and
  comparisons happen in code and enter state as explicit fields; criteria
  demanding math are rejected at config time.
- **Atomic composite scoring.** Complex judgments decompose into separate
  primitive questions (technical complexity, user impact, deadline urgency)
  combined deterministically in application code — never one synthesized
  "overall severity" question.
- **Criteria stay isolated from untrusted state.** Criteria/instructions are
  admin-owned versioned config (ADR 0010 §2) and are never composed from,
  appended to, or echoed into untrusted content (inbound email bodies, alert
  payloads, customer chat). Untrusted text rides state only; injection can
  only move an answer through vague criteria, so criteria quality is a
  curation problem with per-org thresholds as the answer — not a reason to
  interpolate state into criteria.
- **Choice >255 options is a tiered tree, never a flattened list.** Large
  inventories (tool catalogs, board/tag taxonomies) fan out hierarchically:
  level-one category Choice, level-two item Choice — the second pass costs
  ~nothing per ADR 0010's fan-out economics.
- **Speculative fan-out is the default shape.** All questions for one
  trigger ride one request; per-question HTTP calls are an anti-pattern that
  reintroduces the latency stack for zero benefit.
- **Full distributions recorded for audit and calibration.** The decision
  answers store records the complete normalized probability vector +
  confidence + pinned model id per question (rationales do not exist).
  Shadow mode (ADR 0010 §5) applies to tool routing: surfaces ship
  shadow-first; admins flip shadow→live after reviewing their own data.
- **Noul exits loops early; code caps them.** Probabilistic termination is
  thresholded high (≈0.5 routes to review, never acts) and a hard iteration
  cap in harness code remains the actual infinite-loop prevention — Noul
  enables early exit, never extends the cap.

## Consequences

- Lane scope rides Wave 23 (ADR 0010): `p23-decision-harness` gains the
  tool-routing edge (gate + Choice inventory + tiered-tree helper), the
  triage lane gains `decision_contract_scope`, the auxiliary lane gains
  `decision_loop_termination`; loop-termination wiring into the ChatRunner
  lands with the first multi-step agent execution work.
- The old-school guarantee extends: with the decision edge unconfigured, tool
  loop behavior is bit-identical to the ADR 0008 ship state — provable by the
  same zero-side-effect flow test (no ledger rows, no cards, identical
  automation behavior).
- No new privilege path: the routing inventory is a filtered view of
  already-granted tools, so the capability-gate test suite applies
  unchanged to gate-routed calls.
- Vendor risk stays bounded to the adapter (ADR 0010); work stays paid in
  input tokens only — routing at ~100 ms replaces a multi-second model
  round-trip per step in the hot loop.
- Re-evaluation triggers: first multi-step autonomous agent execution
  shipping (loop-termination against real loops); a decision-model vendor
  shipping single-pass Choice beyond 255 options (tiering rule revisits);
  TypeSafe shipping rationale/explanation outputs (audit posture upgrades
  from distributions to rationales).
