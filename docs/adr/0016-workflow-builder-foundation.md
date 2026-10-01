# ADR 0016 — The workflow foundation: our own document and executor, React Flow for the canvas

**Status:** Accepted (user directive) · 2026-10-01
**Scope:** which workflow builder to build on, what the workflow document is, who owns execution, how an LLM authors a workflow, and which surfaces let an operator see runs, edit workflows and read webhook activity.

## Context

Marula wants workflows that let parts of the app drive other parts: a ticket closing billing a retainer, an alert becoming a dispatched job, a portal approval starting work. None of that requires an n8n clone. Two constraints are binding and they pull against each other:

- **Non-technical users must be able to start.** A dispatcher configuring "when a P1 ticket arrives, notify the on-call and raise it" cannot be asked to read a graph.
- **Power users must be able to go deep**, and an LLM must be able to build a workflow from a sentence the user typed.

Six candidates were researched (2026-10-01). They are not four comparable products and two libraries; they are four different categories, and the category decides most of the question.

| Option | Category | Licence |
|---|---|---|
| React Flow (`@xyflow/react`) | canvas library — nodes, edges, pan/zoom, no workflow concept | MIT · 38.6k stars · 17.33M weekly installs |
| Workflow Builder (synergycodes) | editor SDK — canvas + palette + schema-driven properties panel + undo/redo + validation; execution explicitly swappable behind three ports | Apache-2.0 community · 385 stars |
| Vercel `workflow-builder-template` | a demo application (React Flow + Workflow DevKit), not a library | Apache-2.0 · 1.2k stars |
| Activepieces | a platform — own Postgres/Redis/BullMQ fleet, 280+ SaaS pieces | MIT core + commercial EE · 24.8k stars |
| Windmill | a platform — Rust backend, nsjail sandbox, Python/TS/Go/Bash runners | AGPLv3 core + commercial · 18.1k stars |

**The platforms are excluded by our own constraint.** "Natively interact with our data model, without becoming an n8n clone" rules out Windmill first: its execution model *is* the product, so every node would have to become a workspace script running on their workers with credentials in their resource store, while tickets and time entries live in our Postgres behind our RLS. We would write thin proxy scripts forever. Activepieces is the same shape with a different tax: its `props` are a map of Property *objects* with async option functions rather than JSON Schema, its `Property.Custom` injects arbitrary JavaScript into the frontend, and its expression syntax carries a `['output']` wrapper that is a v21→v22 migration artifact which already forced an AST rewriter over stored flows.

Both embed code in the workflow document, which is disqualifying on its own — see "No code in the document" below.

**What we already have is better than any of them for the authoring path.** `AutomationRule` (`crates/core-domain/src/automations.rs:334`) is `trigger_event: String` plus `conditions: Vec<AutomationCondition>` and `actions: Vec<AutomationAction>`, both **typed Rust enums**. Through `utoipa` that already reaches the OpenAPI pin and out to a generated TypeScript client. We already emit a machine-generated schema for our workflow format, with a closed vocabulary of legal actions, and we already ship it.

**Workflow CRUD and webhook activity already exist.** `AutomationsPage` (`apps/web/app/automations/page.tsx:211`) is a full trigger/condition/action builder with a dry-run. `webhook_admin::deliveries` (`crates/core-api/src/routes/webhook_admin.rs:333`) is a cursor-paged per-webhook delivery ledger. **Run history does not.** The queue carries `ClaimedJob { attempts, max_attempts }` with `mark_dead` and `schedule_retry` (`crates/core-db/src/queue.rs:20`), but nothing an operator can browse and nothing that retries one failed run on demand.

## Decision

1. **React Flow is the canvas. The document, the node registry, the executor and the run ledger are ours.**
   React Flow is MIT, has no opinions about our components, and is the substrate under two of the six candidates anyway. What we write is the part we should own — roughly the remaining fifth of the work, not the whole thing.
2. **Workflow Builder was the close second and is rejected on a design-system collision, not on capability.** Its SDK is genuinely good and it would have saved frontend time. It ships `@workflowbuilder/ui` on Base UI plus a white-label "Design System Kit", and adopting a second component system in the same app as the one `p41-ui-chrome` exists to unify reproduces that lane's exact failure — each lane building its own chrome. Three further costs: the serialized envelope carries **no version field**, its `api` persistence strategy sends **no auth headers** (the `props` strategy is mandatory), and ELK layout plus smart edge routing are Enterprise (EUR 6,990). Reconsider only if the canvas work proves materially harder than estimated; `elkjs` is open source and swappable either way.
3. **One document, two editors, plus a third authoring path.** The existing rule builder is the simple tier and stays first-class. The canvas is the power tier and edits **the same JSON** — opening a rule in the canvas is a view toggle, never a migration. The LLM path emits that same JSON. Three surfaces, one artefact.
4. **No code in the document.** A workflow is declarative data: a trigger, conditions, actions, and a branch structure. Never an embedded script, expression string, or eval. Arbitrary logic is a plugin action whose `params` validate against its `params_schema`, which `AutomationAction::PluginAction` (`crates/core-domain/src/automations.rs:286`) already enforces at execution time. This is the security boundary *and* the correctness boundary for LLM authoring: models hallucinate inside code strings and never inside a closed enum.
5. **The node registry is the schema source of truth, in Rust, and it derives everything.** Each node type declares its id, label, an agent-oriented description, an `idempotent` flag, a JSON Schema for its inputs, and an output schema. Those declarations generate the canvas palette, the properties panel, the API types, the plugin-action picker, and the LLM tool catalogue from one source. No second registry in TypeScript.
6. **Node schemas are exposed to the LLM on demand, not inlined into the prompt.** A tool returns one node's schema when asked. This is Windmill's `get_runnable_details` pattern and it is the difference between cheap authoring and a prompt that grows with the catalogue. Each node also carries a short agent-oriented description and an `idempotent` flag so the model can decide whether repeating a step is safe — Activepieces' `aiMetadata`, which is what makes its MCP catalogue work and is cheap to replicate.
7. **Every run is recorded, step by step, and any failed run is retryable from the UI.** A `workflow_runs` and `workflow_run_steps` ledger with per-step status, timing, input and error. This is the one genuinely new backend piece, and it is also what makes decision 3's AI tier trustworthy: a user cannot accept an LLM-authored workflow they cannot inspect afterwards.
8. **Non-goals, binding.** No n8n clone. No DAG editor for its own sake. No script/code step. No third-party marketplace of nodes in the core — plugin-contributed actions only, under the existing ADR 0009 consent lifecycle.

## Consequences

**Good.** The canvas, the hard and undifferentiated part, is MIT and already solved. The workflow format is already machine-generated, so LLM authoring targets a real schema with a closed action vocabulary rather than a hand-rolled envelope. Node types are our entities by construction — a `time_entry.log` node *is* our time-entry schema. The simple tier already exists and is not rebuilt. One design system, because nodes are built from `packages/ui` primitives and obey the same token law as every other surface.

**Costs and risks, stated plainly.** Roughly 5–7 weeks total against building the canvas ourselves, with the backend (~2–3 weeks) ahead of the frontend (~3–4) because the run ledger and executor are what everything else displays. Three estimates carry real uncertainty: the schema-driven properties panel is more work than the canvas looks, ELK auto-layout must be integrated directly rather than inherited, and the LLM authoring quality is unknown until it is measured against our real catalogue — the lane that builds it must capture before/after examples in its landing record, because "the model can emit valid JSON" is a claim that needs evidence like any other.

**Rejected alternatives.** Adopting Activepieces or Windmill wholesale would buy a mature LLM flow builder and cost us the coupling to our own data model, a second runtime, and a path to arbitrary code execution in a self-hosted product whose non-technical users author workflows. Forking Workflow Builder would buy the properties panel and cost a second design system plus an envelope with no version field.

## What this depends on

Wave 43 track 1 supplies the trigger half: lane 1 makes external systems emit real envelopes and lane 2 makes those types subscribable, so the trigger node's catalogue is the event-type registry rather than a hardcoded list. Lane 3 makes ticket-less events able to fire rules, which is what lets a workflow start from something other than a ticket. Lane 4's `automation_trigger` surface kind is how a plugin contributes its own trigger.

Lane 12 (`p43-signature-pad`) and lane 13 (`p43-sign-off-actions`) are the first multi-step consumer of this foundation: a signature is captured, filed against the time entry, and the ticket closed, with the customer optionally notified. That flow is the honest test of whether three steps and a branch are enough, or whether the node vocabulary was designed too thin.