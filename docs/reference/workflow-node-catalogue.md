# The workflow node catalogue

**Status:** Proposed for user review · 2026-10-01
**Decision of record:** [ADR 0016](../adr/0016-workflow-builder-foundation.md)
**Built by:** Wave 43 lane 13 `p43-node-registry`

This is the scope of what a Marula workflow can do. Its purpose is to make the
surface auditable: a power user should be able to read this page and know
whether their idea is possible, and a lane should be able to check a proposed
node against it before building anything.

## The rule that shapes everything else

**Do not enumerate two hundred actions.** The domain has 286 published events and at
least that many operations. A flat catalogue that size is unnavigable, and the
failure mode is a power user hunting through a palette while a dispatcher never
finds the one node they needed.

So the catalogue has three tiers:

- **Tier A — the ~40 nodes that carry most real workflows.** Always visible in the
  palette, first in search, and the only tier the simple rule builder exposes.
- **Tier B — the domain long tail.** Reachable through a **generic operation
  node** whose operation selector is schema-derived from the domain, so the
  palette stays small while the capability stays complete. Tier B is not a
  separate implementation; it is a parameterisation of the same executor path.
- **Tier C — plugin-contributed nodes**, arriving through the ADR 0009 consent
  lifecycle as `PluginAction` surface entries that carry their own schema.

A node earns Tier A placement when two or more realistic workflows would
otherwise hand-roll the same thing. Everything else is Tier B. This document
records the rejected candidates and the reason, so the next lane does not
re-litigate them.

## Triggers

The trigger catalogue is the **event-type registry**, not a hand-written list
(Wave 43 lane 2). `crates/core-events/src/contract.rs:523` holds the 286
published events; `crates/core-domain/src/events.rs:14-609` holds the constants.

A trigger node is `event_type` plus an optional board filter. Not every one of
the 286 events should be offered to a workflow author, and that is a decision
lane 13 must make explicitly rather than by exposing the whole registry. The
proposed cut, and the reasoning:

**Offered as Tier A triggers** — these are the moments a user thinks in:

| Trigger | Events |
|---|---|
| Ticket lifecycle | `ticket.created`, `ticket.status_changed`, `ticket.assigned`, `ticket.closed`, `ticket.reverted`, `ticket.board_moved`, `ticket.priority` (derived), `ticket.tags_changed`, `ticket.comment_added`, `ticket.field_set` |
| Client and site | `client.created`, `client.updated`, `site.created`, `contact.created` |
| Time and attendance | `timer.started`, `timer.stopped`, `time_entry.created`, `time_entry.approval_submitted`, `time_entry.approval_approved`, `time_entry.approval_rejected` |
| Money | `quote.accepted`, `quote.converted`, `quote.declined`, `invoice.created`, `invoice.payment_recorded`, `invoice.voided`, `invoice.dunning_changed`, `invoice_run.completed`, `credit_note.issued`, `contract.created` |
| Scheduling | `appointment.created`, `appointment.cancelled`, `appointment_request.created`, `appointment_request.approved`, `appointment_request.declined`, `dispatch.status_updated`, `dispatch.queue_updated`, `on_call.updated` |
| Field service | `alert.ingested`, `alert.acknowledged`, `alert.resolved`, `travel_draft.submitted` |
| Quality | `csat.response_recorded`, `sla.escalation_fired` |
| External | inbound webhook (lane 1), schedule, manual run, email received |

**Deliberately not offered as triggers:** configuration churn (`setting.updated`,
`board.status_added`, `sla.policy_updated`, every `*.created`/`*.updated` pair on
boards, roles and tags). A user who can trigger on `board.status_added` will
eventually build a workflow that breaks when a status is renamed, and it was
never the automation anyone wanted. **This is a proposal and needs the user's
ruling** — the lane must not silently drop 200 events an operator may have
legitimate reason to watch.

## The Decision Engine nodes

Call this out early, because it is where the node vocabulary would otherwise
have been too thin, and this is the engine that makes the whole thing
interesting. It is not one question against one entity. It is **arbitrary
composed context in, a batch of atomic questions, a batch of typed answers out**,
and the answers come back with enough structure to route on.

### What the engine actually is

`crates/core-decision` takes `DecisionRequest { state, questions: Vec<Question> }`
and returns `DecisionResponse { answers: Vec<Answer> }` (`provider.rs:99-114`).
Two properties of that shape are the whole point:

1. **All questions for one trigger ride a single provider call.** ADR 0011 §4 says
   this explicitly, and `validate_config` **refuses** composite questions — a test
   named `composite_overall_severity_is_refused` locks in "decompose into atomic
   questions combined in code". So the engine is built to be asked several things
   at once and never to be asked one compound thing.
2. **Every answer carries a full distribution alongside its verdict.**
   `Answer::Choice { picked, distribution, confidence }`,
   `Answer::Score { level, distribution, confidence }`, and
   `Answer::Noul { probability }` where `probability` is P(yes) and ≈0.5 sends the
   answer to review without acting on it (ADR 0010 §5).

### The node

| Node | What it does |
|---|---|
| `decision.ask` | Send a composed context and a **set of atomic questions** to a configured decision surface in **one call**, and return **every answer** as a separately addressable output. |

**Inputs**

| Input | Shape | Notes |
|---|---|---|
| `surface` | enum of the six user-facing surfaces | Internal surfaces are refused by the registry (below) |
| `questionSet` | reference to a question set | A named set, not inline questions — see below |
| `context` | ordered map of field name → `TemplateField` | The state the model sees, composed from three kinds of source |
| `subject` | `entity_type` + `entity_id` | Recorded on the run for audit; the model never sees a raw id |

`context` accepts exactly the three sources `TemplateField` already defines
(`config.rs:52`):

- **`From { path, max_chars }`** — a dot-path into a **referenced entity's**
  snapshot, truncated. This is how upstream workflow output enters: a
  `domain.query` result, a previous step's return value, a ticket's client.
- **`Derived { name }`** — composed in harness code from the event envelope plus
  the org's business-hours and SLA config (`AfterHours` and its siblings). The
  class never sees raw timestamps, durations or collections.
- **`Literal { value }`** — an admin-owned constant, string/number/bool only.

**The state budget is checked on the node, not at runtime.** The engine enforces
`STATE_BUDGET_TOKENS` on the serialized composed state and refuses with
`StateBudgetExceeded`. A workflow author must see the refusal **in the
properties panel while editing**, with a live character count, rather than
discover it as a failed run.

**Question sets.** Writing six questions inline at every node would make the
canvas unusable, and the surfaces already do it this way — `starters.rs:124-406`
holds six starter configs, each a `questions: Vec<QuestionConfig>`. So the node
references a **question set**, either:

- a **surface's configured set** (the org's existing config, answerable today), or
- a **workflow-scoped set** owned by the workflow, so an author can compose
  their own without an admin changing org config.

Each `QuestionConfig` is one atomic question: `QuestionKind::Noul`, `Score` (with
a `rubric`), or `Choice` with `ChoiceOptions::Fixed`, `FromStatePath` or
`Tiered`. Each carries **its own `act` and `review` thresholds**, so a single set
can say "route this one to review when confidence is low, but act on that one
confidently". `Tiered` is a two-pass tree, which is what lets
`decision_tool_routing` pick a category and then an item.

**Outputs — every answer, addressable**

| Output | Type |
|---|---|
| `answers` | the full ordered array, preserved for the run ledger and for audit |
| `answers.<questionId>.picked` | the selected option index, or the rubric level |
| `answers.<questionId>.label` | the human-readable option or level text |
| `answers.<questionId>.distribution` | the full normalised distribution, for downstream thresholds |
| `answers.<questionId>.confidence` | `f64`, present on Choice and Score |
| `answers.<questionId>.probability` | `f64`, present on Noul (P(yes)) |
| `answers.<questionId>.verdict` | `act` / `review` / `abstain`, computed from that question's own thresholds |

Per-question outputs are what make the node actionable. Without them it
would be interesting and useless. `flow.branch` and every condition node can gate on
`answers.duplicate.verdict`, or on `answers.urgency.distribution[3]`, or on any
combination across questions. One call, several answers, and the workflow routes
on whichever it wants.

### Why this shape, and why it is the interesting part

**Four questions in one call is not four calls.** Chain four single-question
decision nodes instead and you pay four times as much, wait four times as long,
and get four answers from four blind spots, since each call is blind to the
others. Asking together is what lets the model say "this looks urgent *and* a
duplicate of 18432, so reprioritise rather than dispatch". That correlation is the
capability, and it disappears the moment the node is narrowed to one question.

This is also why ADR 0011 §4's rule that questions stay atomic and combine in
code matters at the workflow layer. The model judges each thing on its own terms,
and **the workflow is where the combination happens** — visibly, in the graph,
inspectable by a human who did not write it.

### Idempotence

`decision.ask` is **`idempotent: false`** by default. A second call costs money
and may answer differently. The flag is load-bearing: it is what tells the
executor whether a retry is safe, and lane 13 must record it per node rather than
defaulting the whole registry.

### Surface visibility

**Ten decision surfaces exist today. Four are internal-only** and gated as
`internal` in `decision_edge.rs:341`:

- `decision_search_rerank` · `decision_llm_verify` · `decision_tool_routing` ·
  `decision_loop_termination`

**A workflow must not be able to reach an internal surface.** Encode that as a
registry property so it cannot be forgotten at runtime.

The six proposed as user-facing for `decision.ask`: `decision_triage` (ticket),
`decision_alert_triage`, `decision_dispatch`, `decision_billing_audit`,
`decision_contract_scope`, `decision_record_dedupe`. **The last four are a
proposal and need the user's ruling** — only the first two are obviously
user-facing.

### The relationship to the existing AI condition

`AutomationCondition::AiDecision` already exists (`automations.rs:52`) and gates
on the **newest recorded answers** for a ticket, not a live fan-out, with missing
or stale answers degrading to a no-match-with-warn. That is the right posture for
a *gate*, and it must stay.

The two are different and must not be collapsed:

| | `AutomationCondition::AiDecision` | `decision.ask` node |
|---|---|---|
| When | evaluates recorded answers, no model call | calls the model |
| Question | fixed per surface config | workflow's own question set |
| Context | the ticket snapshot | the composed `context` map, from any source |
| Answers | boolean gate | every answer, with distributions, addressable by id |

The condition is for cheap deterministic gating on something already decided. The
node is for deciding.

### The four internal surfaces, named precisely

They are worth stating because each is a capability a workflow author will ask
for and must not have:

- `decision_tool_routing` — picks which tool an agent should call. Giving a
  workflow this hands it the tool-routing problem, and `tools` is already refused
  (below).
- `decision_loop_termination` — decides when an agent loop should stop. Useless
  without agentic loops, which ADR 0016 rule (a) forbids.
- `decision_search_rerank` — ranks search results. Belongs to the KB surface, and
  the right answer is for a workflow to call `domain.query`, not to re-rank.
- `decision_llm_verify` — verifies an LLM's own output. A workflow that wanted
  this should get a `decision.ask` question instead.

### The other two AI edges, and the third that is refused

All three ride the same deny-by-default admission, `authorize_surface_call`
(`crates/core-api/src/routes/ai_policy.rs:2845`), with typed refusals
`ai_surface_internal`, `ai_surface_disabled` and `ai_surface_not_granted`. The
capability edges are `AiEdge::Completion`, `AiEdge::ToolCall` and
`AiEdge::Decision` (`ai_policy.rs:2807`).

| Node | Verdict |
|---|---|
| `transcription` | **Offered.** Transcribe an attachment (`routes/transcription.rs:83`). A genuinely useful workflow node: an email arrives with a voice note, the workflow transcribes it, then creates the ticket from the text. It is an `AiEdge::Completion` call with a narrow surface, so the blast radius is small. |
| `completion` | **Offered, behind an explicit grant.** The free-text completion edge, used by chat and by transcription. It is the most open-ended thing in the system, so a workflow may only use it where the org has granted the surface, and its output is never auto-actioned without a decision node downstream. |
| `tools` | **Refused.** The tool-calling edge (`AiEdge::ToolCall`). Tool calling inside a workflow reintroduces exactly the agentic loop ADR 0016 forbids, and it bypasses the one-fan-out-per-trigger doctrine. `decision_tool_routing` is refused for the same reason. |

## Conditions and branching

Today `AutomationCondition` carries twelve variants (`automations.rs:135`).
Wave 43 lane 17b added six of them: `PriorityIsNot`, `CategoryIsNot`,
`TitleNotContains`, `PriorityIn`, `CategoryIn`, `AssigneeIsSet`. A rule can now
say "priority is not P1" or "priority is any of P1, P2", which had no way to be
written down before.

Every variant is still **ticket-shaped**, which is the same limitation Wave 43
lane 3 fixes.

The node vocabulary needs conditions over the *whole* subject. Today they only
reach a ticket:

| Group | Nodes |
|---|---|
| Field comparison | `field.equals` / `notEquals` / `contains` / `isEmpty` — **generic over any registry-declared field on any entity**, so it does not need a variant per field |
| Numeric | `number.gt` / `gte` / `lt` / `lte` / `between` — covers SLA remaining, stock level, hours, totals, probability |
| Temporal | `date.isBefore`, `date.isAfter`, `date.withinLast` (`2h`, `3d`), `date.isOverdue` |
| Collection | `list.contains`, `list.length.gte`, `list.isEmpty` |
| Decision | `decision.matches` — gates on a **recorded** answer, reusing the existing `evaluate_ai_decision` semantics. For a live call, branch on `decision.ask` outputs instead |
| Always | `condition.else` — the explicit fallthrough, so a branch never dangles |

**`field.equals` is the load-bearing one.** It is what makes the catalogue finite.
Every entity's fields come from the registry, so a client field added next year
works today without a new condition node.

Conditions answer a question. `flow.branch` and `flow.switch` (below) decide where
that answer sends the run.

## Actions

The existing five are `AssignUser`, `SetPriority`, `AddInternalComment`,
`NotifyUsers` and `PluginAction` (`automations.rs:286`). That is enough for a
rule and nowhere near enough for a workflow. The proposal, by tier.

### Tier A actions

| Domain | Nodes |
|---|---|
| Tickets | `ticket.create`, `ticket.update`, `ticket.transition`, `ticket.assign`, `ticket.comment`, `ticket.internalNote`, `ticket.setPriority`, `ticket.setTags`, `ticket.addChecklistItem`, `ticket.completeChecklistItem`, `ticket.link`, `ticket.merge`, `ticket.moveToBoard`, `ticket.setField` |
| Time | `timeEntry.log`, `timeEntry.update`, `timeEntry.submitForApproval`, `timer.start`, `timer.stop` |
| Clients | `client.create`, `client.update`, `client.addNote`, `contact.create`, `contact.update`, `site.create` |
| Money | `quote.create`, `quote.send`, `quote.convert`, `invoice.create`, `invoice.recordPayment`, `invoice.void`, `creditNote.issue`, `contract.create` |
| Scheduling | `appointment.create`, `appointment.update`, `appointment.cancel`, `dispatch.assign`, `dispatch.updateStatus`, `appointmentRequest.approve`, `appointmentRequest.decline` |
| Communication | `email.send`, `notification.send`, `sms.send` (if the org has a provider), `csat.request`, `portal.notify` |
| Field service | `alert.acknowledge`, `alert.resolve`, `travelDraft.create`, `signOff.capture` (lane 12), `signOff.file` |
| Assets | `stockItem.create`, `stockItem.move` (a real ledger movement, never an UPDATE), `toolCheckout`, `toolCheckin`, `loan.issue`, `loan.return`, `purchaseOrder.create`, `purchaseOrder.receive`, `purchaseOrder.postBill` |
| Records | `document.create`, `document.attach`, `attachment.upload`, `kbArticle.create`, `kbArticle.update`, `secret.record` (write-only; never read a secret value into a workflow) |
| Projects | `project.create`, `projectPhase.complete`, `projectTask.create`, `projectTask.complete` |

### Tier B actions, all reached through one generic node

| Node | What it does |
|---|---|
| `domain.update` | Update fields on any registry-declared entity. The operation selector is schema-derived from the domain, so it covers the long tail without a node each. **This node replaces about 150 hand-written action nodes.** |
| `domain.query` | Read entities matching a schema-derived filter. Output feeds conditions and downstream nodes. Read-only and rate-capped. |

`domain.update` is the single most important decision in this catalogue. Hand-write
Tier A as ~90 discrete node types and their schemas come along too, at which point
the registry stops being a single source of truth. Tier A should instead be *thin
wrappers that validate their arguments and then call the same executor path as
`domain.update`*, which keeps the registry honest and the catalogue navigable.

### Tier C — plugin-contributed

| Node | Notes |
|---|---|
| `plugin.action` | Already exists as `PluginAction { entry_key, params }`. The registry entry's `params_schema` is the input schema, and the entry is **re-read at execution time** so a revoked install degrades to a typed skip instead of a guess. Lane 4 adds `automation_trigger`, so a plugin can contribute *triggers* as well as actions. |

## Control nodes

Two families here, and the second one changes the engine's shape rather than
adding a node.

### Time-based waits

| Node | Inputs | Notes |
|---|---|---|
| `flow.delay` | `duration` (`4h`, `90m`, `2d`) | Continue after a duration. **Never holds a worker.** The run is parked with a resume time and the queue does the waking. |
| `flow.waitUntil` | `at` (absolute instant), optional `calendar` | Continue at a wall-clock instant. With a `calendar` bound it resolves **business-hours-aware**, which is nearly free here: `sla_calendars` is already an org-scoped model with full CRUD (`crates/core-db/src/sla.rs:170-287`) and is already joined into policy evaluation, so "9am on the next business day" reuses it instead of inventing a second calendar. |
| `flow.deadline` | `until`, plus `onExpiry` (`fail` / `continue` / `route`) | **The one that matters most.** Continue when something happens, but not past a time. An escalation is not a wait plus a check. It is one node, because the alternative is two nodes whose failure mode is "nobody ever noticed". A ticket breaching SLA in four hours needs a wake-up whether or not the awaited event arrives. |

### Event-based waits, the parking nodes

| Node | Inputs | Notes |
|---|---|---|
| `flow.waitFor` | `match` (a declared set of event types), `where` (optional conditions), `timeout` | Park the run. Resume on the **first** matching event, or on timeout. This is the "wait for X" primitive. |
| `flow.waitForAll` | same, but every declared event must arrive | A run waiting on three approvals must not proceed on the first one. |
| `flow.awaitApproval` | `approvers` (user or role), `timeout`, `onExpiry` | A human gate, kept separate from `waitFor` because it carries **identity** and rides the existing agent-proposal lifecycle (`agent_proposal.created/approved/declined` already exists). A wait-for-event has no approver. An approval always does. |

**These three require engine changes that lanes 14 and 15 must not miss.**

1. **A parked run is a new execution state.** The ledger needs `waiting`, with a
   resume cursor and a wake-at, alongside `running` / `succeeded` / `failed`. A
   run waiting three days holds no connection and no transaction.
2. **A new job kind.** `settle_job` (`crates/worker/src/queue_runner.rs:174`)
   matches a **closed list**: `sla.verdict`, `pm.generate`, `secrets.rotation`,
   `automation.effects`, `import.batch.run`, and `webhook.deliver`. Anything else
   dead-letters with "unknown job kind". `workflow.resume` has to be added there
   explicitly, and a lane that adds the node without touching that match ships a
   run that sleeps forever.
3. **A third event consumer.** Today every event fans out to automations
   (`automations.rs:98`) and outbound webhooks (`webhook_out.rs:220`).
   `waitFor` adds a third: parked runs. The event router needs a waiters index,
   and that is the piece which makes loop protection mandatory rather than
   advisory (below).
4. **`schedule_retry` is not a wait.** It carries `max_attempts` and dead-letters
   on exhaustion. A three-day wait modelled as a retry is a run that dies
   overnight.

### Joins and fan-out

| Node | Notes |
|---|---|
| `flow.parallel` | Fan out to several branches. **Bounded**, with the cap an org can raise and the author cannot. |
| `flow.join` | Converge N branches. Mode is `all`, `any`, or `n-of-m`. Needs its own concurrency cap. |

### Iteration

| Node | Notes |
|---|---|
| `flow.foreach` | Iterate a collection, binding each item as `item`. **Hard cap, org-raisable, refused at author time above it.** See "Iteration is the dangerous one" below. |

There is deliberately **no `flow.while`**, **no `flow.until`**, and **no
`flow.filter`**. Filtering is `domain.query`'s job, since it already takes a
filter. An unbounded loop is a run that never finishes.

### Branching

| Node | Notes |
|---|---|
| `flow.branch` | If/else, two labelled outgoing edges. |
| `flow.switch` | Multi-way on one value, one edge per case, plus a default. |

### Error handling

| Node | Notes |
|---|---|
| `flow.onError` | A **property on every node**, not a separate node: `halt`, `continue`, or `route` to an error branch. A policy living on the node is one an author cannot forget to attach. |
| `flow.errorHandler` | Where error branches converge. Its outputs carry the failing node's id, the error class, and the step's inputs. |
| `flow.retry` | In-graph retry with backoff. Distinct from the operator retry in lane 14. **Must refuse to wrap any node declared `idempotent: false`**, `decision.ask` among them, because it spends money and may answer differently. Must also avoid multiplying against the queue's own attempt budget, which is how a transient blip turns into twelve calls. |

### Termination

| Node | Notes |
|---|---|
| `flow.stop` | End successfully. |
| `flow.fail` | End with an error the operator sees in the ledger. |
| `flow.abandon` | End deliberately, having chosen not to act. **Distinct from `fail` on purpose**, so the ledger can say "we decided not to dispatch this" differently from "the dispatch broke". A queue of identical failures is unreadable. A queue that mixes the two is triage. |

### Iteration is the dangerous one

`flow.foreach` is the node that turns a workflow engine into an outage generator.
A loop over a collection with no cap, inside a run that also writes, is how you
get ten thousand tickets updated at three in the morning.

Three defences, all three required:

1. **A hard cap per run**, org-raisable and surfaced at author time, so the
   palette shows "12 items" beside a collection of 4,000.
2. **Batched iterations**, one run with 12 steps, rather than 12 runs.
3. **`loop.maxDepth` on the whole graph**, not per-node.

### Loop protection becomes mandatory once waits exist

Without `waitFor` a workflow is a straight line, and the existing
`is_automation_echo` guard (`automations.rs:26`) is enough. With it, a workflow
can resume itself:

> `flow.waitFor(ticket.status_changed)` → later `ticket.transition` → the wait
> matches the workflow's own write → resume → transition → resume, forever.

A depth counter is the weak answer. The strong one is free, because the registry
already holds the information: **each node declares the event types it can
produce** (the emit seam from Wave 43 lane 4C). The engine can therefore
**statically refuse** a `flow.waitFor` whose matched event type is producible by
a node in the same workflow. A validation error at author time, rather than a
runtime surprise.

Three answers to "what started this run, and from where":

- **Every run records its triggering event id and its triggering run id**, so
  ancestry is a column and the chain is walkable.
- **`loop.maxDepth`** on the graph, default 1. A workflow that wants to resume
  itself says so at author time.
- **A resumed run is a new run row with a `resumed_from` pointer**, never a
  mutation of the parked one. The ledger is an audit record, and rewriting it is
  how an audit turns into fiction.

### Value nodes

These are not control flow, but a workflow cannot be written without them.

| Node | Notes |
|---|---|
| `value.literal` | A constant. Every other node takes inputs, and a workflow that cannot name a fixed string is not writable. |
| `value.reference` | Point at a field of a previous step's output. **A typed reference, not an expression string** — ADR 0016 rule (a). |
| `value.math` | Arithmetic on numeric outputs, for "remaining = due − used". Closed operator set. |
| `value.template` | Interpolate a named field into text, e.g. an email body. Closed substitution set. **The one place string building is allowed, and it stays a template, never an expression.** |

### Deliberately absent from control flow

- **No sub-workflow invocation.** A workflow calling a workflow turns every
  failure into a stack trace nobody can read.
- **No unbounded loop of any kind.** `foreach` with a cap, or nothing.
- **No cron or schedule node.** Scheduling is a **trigger**, and Wave 43 lane 5
  already has the typed schedule trigger with its delivery record.
- **No catch-all.** `flow.awaitApproval` requires a `timeout` and an `onExpiry`.
  A gate with no expiry is a way to lose a run permanently.

## How plugins interact with the workflow engine

**Decision of record: [ADR 0017](../adr/0017-plugin-node-surface.md) — accepted 2026-10-01.** Equal capability for first-party and marketplace connectors, with the org withholding per install through the existing `granted_capabilities`. No authorship class distinction.

### Where plugins stand today

Three wasm plugins ship (`plugins/wasm/asset-tag-validator`, `escalation-widget`,
`triage-note-action`) and all three declare a `surface_profile`. The
`automation_action` surface is real and executing: the effects path handles it
at `crates/core-api/src/routes/internal.rs:1018`, the executor re-reads the
registry entry at execution time, and the automations page already lists plugin
actions from `GET /api/v1/plugins/surface/automation-actions`
(`crates/core-api/src/routes/plugin_surface.rs:84`).

**Zero connectors declare any surface, and today they cannot.**
`PluginManifestV2::validate` refuses it outright:

> `"http_connector manifests must not declare a surface_profile"`
> — `crates/core-domain/src/entities/plugins.rs:975`

### The rule's stated reason does not cover the kinds we need

The code comment gives the reason: *"§4.6 kind-coherence: the surface profile is a
WASM-manifest declaration — a connector has no in-product UI surface to govern."*

That reason is sound for `widget` and `report_render`, which really are UI. But
`automation_action` is not a UI surface. It is an **executor** surface, and so
are the two kinds this wave adds, `automation_trigger` and `read`. So the rule's
*rationale* covers two of the six kinds in the vocabulary and not the other four,
and the *code* refuses all six.

The amendment should be **per-kind, not a blanket relaxation**:

| Kind | Connector allowed? | Why |
|---|---|---|
| `widget`, `report_render` | **No** — keep refusing | The original UI-coherence reason stands exactly as written |
| `automation_action` (write) | Yes, with connector-specific consent | The connector already holds credentials and network egress by design |
| `automation_trigger` | Yes | Non-UI |
| `read` | Yes | Non-UI |

The load-bearing argument for allowing a connector to declare an action is that
**it grants the connector nothing new.** An `http_connector` already holds OAuth
credentials and outbound network access as its reason for existing; a workflow
action only makes an existing capability addressable from a workflow. A wasm
module is the opposite case, which is presumably why it started there: a wasm
sandbox is capability-restricted, so granting it a network action *is* an
escalation, and it earns that through explicit consent.

### The surface contract needs six fields, not one

`plugin_surface_entries` stores `surface`, `entry_key`, `label`, `definition`
(JSONB) and `enabled` (`crates/core-db/src/plugin_surface.rs:16`). Everything
about the node lives inside `definition`, and today it carries an input schema
alone. A workflow node needs six:

| Field | Status | Why the workflow needs it |
|---|---|---|
| `inputSchema` | exists (`params_schema`) | Validated before the effect runs |
| `kind` | **new** — `read` / `action` | `read` is the missing piece; see below |
| `outputSchema` | **new** | Downstream `value.reference` and every condition gate on a returned field. Without it a workflow can call a plugin but cannot use what it returned |
| `idempotent` | **new** | Load-bearing. `flow.retry` **must refuse** to wrap any node declared false, and the queue's own retry needs to know the same thing |
| `emits[]` | **new** | Load-bearing. Lane 15's **static loop refusal** works by checking whether the awaited event type is producible by a node in the same workflow. A plugin that emits `ticket.status_changed` and then waits for it would deadlock a workflow |
| `audience` | **new** — `human` / `ai` / `both` | Decides palette visibility against the LLM tool catalogue. A power user and the authoring model should not see the same list |
| `agentDescription` | **new** | Model-facing prose for the same node, alongside the human label. Cheap, and it is what makes LLM authoring cheap |

`emits[]` is the one with teeth. It is the same mechanism that makes a
first-party NMS plugin safe inside a workflow, and it is invisible until you
build it: the failure is a workflow that resumes itself forever, which is exactly
the class of bug nobody finds before production.

### The three directions, and which needs what

This is the shape of "a workflow interacts with my RMM", and the three directions
have very different costs.

**Inbound — RMM tells Marula something.** *"A managed device went offline."*

This is the majority of RMM value and it needs **no surface declaration at all**.
Wave 43 lane 1's generic signed ingest already accepts a signed POST that resolves
an org and declares an emitted event type. The connector HMACs with its own
secret and posts `rmm.device_offline` with a payload carrying the device, site,
failing check and severity. The event-type registry (lane 2) makes it
subscribable.

**Prefer this path by default.** It is the cheapest, it needs no relaxation of
the surface rules, and it reuses machinery lane 1 already builds.

**Outbound write — Marula tells the RMM something.** *"Push this ticket to the
vendor portal."*

This needs a connector `automation_action`, so it needs the per-kind amendment
above.

**Read — Marula asks the RMM a question.** *"Is this device in a maintenance
window? What is its site?"*

**This is the missing one, and it is the one the example needs.** Today
`automation_action` is write-shaped. A workflow cannot fetch anything from an
external system, which means it cannot assemble Decision Engine context from an
RMM, which means the most valuable thing a workflow can do with a connector is
inaccessible.

A `read` surface needs its own guardrails, and they are stricter than a write's:

- **Rate-capped** per run and per org. A loop over devices becomes a denial of
  service against the customer's own RMM.
- **Never returns a secret.** No vault value may appear in a `read` output.
  `secret.record` writes; nothing reads, and that holds here too.
- **Output must be bounded**, or a device list comes back with ten thousand
  objects and the run ledger grows by megabytes.
- **Distinct consent line**, because "this plugin can read your RMM" is a
  different promise from "this plugin can act in your RMM".

**Poll — the RMM has no push API.** Use the typed schedule trigger (lane 5) plus
a `read`. No new node, and worth stating because "our RMM does not do webhooks"
is the most common real-world connector constraint.

### The RMM workflow, end to end

> *"When a managed device goes offline, open a ticket at the right site, assign
> the on-call technician, and if nobody acknowledges it in 30 minutes, tell the
> client."*

| Step | Node | Notes |
|---|---|---|
| 1 | trigger `rmm.device_offline` | Declared by the connector via signed ingest, or by lane 1's generic source registration |
| 2 | `plugin.read` — device → site mapping | The RMM knows which site a device is at; Marula does not until this runs |
| 3 | `domain.query` — the site's open tickets | Is this already being worked? |
| 4 | `decision.ask` | **One call, several questions.** Is this inside a maintenance window? Is the site a single-tenant or shared office? Is a spare device available at the site? Impact given the device's role. All four ride one call, and the answers correlate |
| 5 | `flow.branch` on `answers.impact.verdict` | Route by the answer, not by a fixed rule |
| 6 | `ticket.create` at the mapped site, `ticket.assign` from the on-call roster | Native domain actions. No HTTP in the document |
| 7 | `flow.deadline` 30m, `onExpiry: route` | The escalation is one node, so nobody has to remember to check |
| 8 | `notification.send` / `sms.send` | The client hears about it |

Every step is Marula's own vocabulary. The RMM appears twice, at the trigger and
at the one read. **No connector credential ever enters the document**, because
the host resolves it from the install at execution time.

### The trust posture for a plugin node

Binding, and all of it already exists for plugin actions except the parts marked
new:

- **Credentials are resolved by the host** from the install, at execution time.
  They are never values in the workflow document, never in the run ledger, and
  never returned in a node output.
- **Workflow capability is a per-install grant**, deny-by-default, riding
  `plugin_installs.granted_capabilities`. Withdrawing it removes the nodes from
  the palette and degrades a running call to a typed skip. **The canvas must
  render a withdrawn grant as a visible, explained placeholder**, because an
  author whose node vanished needs to know why.
- **The entry is re-read at execution time** (`get_entry_by_key`
  also requires `i.status = 'enabled' AND i.revoked_at IS NULL`,
  `plugin_surface.rs:170`), so a revoked install degrades to a typed skip rather
  than a guess. **A revoked plugin must never leave a workflow silently
  half-finished**: the run goes to `flow.fail` with the revoked entry named.
- **Consent shows every kind**, and an emitted event type is as much a promise as
  an endpoint. Lane 4's consent screen gains the same treatment as `webhook_endpoints`.
- **Digest pinning already governs the artifact**, so a third-party connector's
  action cannot change behaviour after consent without a re-consent.
- **Uninstall tears down what it seeded** — outbound subscriptions (lane 4A) and
  now any registered event source.

### Open questions specific to plugins

1. ~~**Should first-party and third-party connectors get the same surface
   capability?**~~ **RESOLVED 2026-10-01: yes, with a per-install grant the org can
   withhold.** See [ADR 0017](../adr/0017-plugin-node-surface.md) decisions 1 and 2.
   The grant rides `plugin_installs.granted_capabilities`, which already exists,
   so no new table is needed.
2. ~~**Does a workflow-visible plugin action need its own grant?**~~ **RESOLVED:
   yes, and it is the same grant.** Consenting to a connector and letting
   workflows call it are two separate decisions on one capability set. A separate
   "allow workflows to use this" toggle would have been a third place to answer the
   same question.
3. **Should plugin actions be exposed on the MCP server?** `crates/mcp-server`
   exists, and exposing the same node catalogue there would let a user's own LLM
   build workflows against the same registry the palette reads. Tempting and
   genuinely useful, and out of scope for this wave.

## What is deliberately absent

Recorded here so a later lane does not add them by accident.

- **No code step, no script, no arbitrary expression language.** ADR 0016 rule
  (a). Arbitrary logic is a plugin action with a schema. A plugin *executable*
  is a different thing from code in the document, and this rule is about the
  document.
- **No tool-calling edge.** It reintroduces an agentic loop and bypasses the
  one-fan-out-per-trigger doctrine.
- **No internal decision surfaces.** `decision_search_rerank`,
  `decision_llm_verify`, `decision_tool_routing` and `decision_loop_termination`
  are internal, and the registry must refuse them.
- **No raw SQL, and no generic HTTP node in the core.** Outbound HTTP is
  already a first-class product feature — the signed webhook with its delivery
  ledger and retries. A workflow that wants to "call an API" should subscribe to
  an event, not smuggle an HTTP call through an automation. **Talking to an
  external system is the plugin's job**, which is the section above.
- **No secret read.** `secret.record` writes. Nothing reads a secret's value into
  a workflow's variables, and no plugin `read` output may carry one, because the
  run ledger is a place operators read.
- **No sub-workflow invocation, no unbounded loop, no schedule node.** Those three
  are covered with reasons in the control-nodes section above.

## Open questions for the user

These are the points where the catalogue cannot be settled from the code alone.

1. **Configuration events.** Should `board.status_added`, `setting.updated`,
   `role.forked` and their neighbours be offered as triggers? Proposed answer:
   no. A workflow that depends on a status label breaks when someone renames it.
2. **Which decision surfaces are user-facing?** Six of the ten are proposed for
   `decision.ask`. `decision_triage` and `decision_alert_triage` are obviously
   user-facing; `decision_dispatch`, `decision_billing_audit`,
   `decision_contract_scope` and `decision_record_dedupe` should be confirmed.
3. **Is `sms.send` real?** It is listed as conditional on an org provider. If
   the product has no SMS surface, drop the node rather than ship a dead one.
4. **Can a workflow be triggered by a portal action?** A portal user approving a
   quote is an obvious trigger. It needs a decision on whether portal identity can
   act as a workflow subject.
5. **Retention and blast radius.** If a fan-out node can touch 5,000 tickets, is
   that allowed, or does the executor cap at a number an org can raise? The run
   ledger needs an answer before fan-out ships.
6. **Does `domain.update` need an approval gate?** A workflow that updates 200
   records unattended may need the same approval posture as the agent-proposal
   path (`agent_proposal.created/approved/declined` already exists for this).
7. **How long may a run stay parked?** A three-day wait is plausible for an
   escalation. A ninety-day wait on a quote nobody has answered is a row nobody
   will ever look at. Propose a default max parked duration with an org-raisable
   ceiling, because `waiting` runs are the one thing in this engine that can
   accumulate without any human involvement.
8. **Should `flow.waitFor` be able to match an external signal?** An inbound
   webhook is an event, so yes in principle. But the honest version is that a
   workflow whose next step depends on an external system should subscribe and
   branch, not park. Worth a ruling before `waitFor` is offered.