# ADR 0017 — The plugin node surface: equal capability, a per-install grant, and six declared fields

**Status:** Accepted (user directive) · 2026-10-01
**Scope:** which plugin kinds may contribute workflow nodes, how an org withholds that capability, and what a node must declare before the engine will run it.

## Context

ADR 0016 decides the workflow foundation and, in passing, notes that a plugin
action is the only sanctioned route for arbitrary logic. It does not answer who
may contribute a node, which is the question a first-party RMM, NMS or identity
connector asks immediately.

**The current answer is "no one but wasm".** `PluginManifestV2::validate`
refuses a `surface_profile` on any connector:

> `"http_connector manifests must not declare a surface_profile"`
> — `crates/core-domain/src/entities/plugins.rs:975`

Three wasm plugins ship (`plugins/wasm/asset-tag-validator`,
`escalation-widget`, `triage-note-action`) and all three declare surfaces. **No
connector declares any surface**, because the rule refuses them, and the
`automation_action` surface is otherwise live: the effects path executes it at
`crates/core-api/src/routes/internal.rs:1018` and the automations page already
lists entries from `GET /api/v1/plugins/surface/automation-actions`.

The code comment gives the reason, and the reason is narrower than the rule:

> *"§4.6 kind-coherence: the surface profile is a WASM-manifest declaration — a
> connector has no in-product UI surface to govern."*

That reasoning holds for `widget` and `report_render`, which are UI. It does not
reach `automation_action`, which is an executor surface, nor the two kinds this
wave adds. So the rationale covers two of six kinds in the vocabulary and the
code refuses all six.

**The grant hook already exists.** `plugin_installs.granted_capabilities`
(`crates/core-db/src/plugins.rs:42-44`, written by `create_install:261-279`) is
already the per-install capability set an org narrows at consent time. There is
also a deny-by-default grant precedent to copy: `ai_surface_grants` with
`grantee_kind` of `user` or `role`, `user_has_surface_grant:461-489`, and
`authorize_surface_call` returning typed refusals
(`ai_surface_internal`, `ai_surface_disabled`, `ai_surface_not_granted`).

**The RMM case needs three directions, and only two need a surface.** A device
going offline is inbound, and inbound is a signed POST to the generic ingest
that ADR 0016's Wave 43 lane 1 already builds. Fetching a site's device list is a
read, and no surface kind can express a read today, because
`automation_action` is write-shaped. That is the specific gap: a workflow cannot
assemble Decision Engine context from an external system at all.

## Decision

1. **First-party and marketplace connectors get the same capability. There is no
   authorship class distinction.** The case for equal treatment is that trust
   comes from consent plus digest pinning plus the grant, none of which depend on
   who wrote the artifact. The case against is that an unbounded `read` is a
   liability we would be handing to strangers. Decision 2 answers that at the
   point where it matters, which is the org that owns the consequence.
2. **The org may withhold, per install.** Workflow capability rides the existing
   `plugin_installs.granted_capabilities`, deny-by-default, so consenting to a
   connector and letting workflows call it are two separate decisions. An org
   can install an RMM connector for its ticket board and refuse the workflow
   grant. Withdrawing the grant removes the nodes from the palette and degrades
   a running workflow's call to a typed skip, exactly as a revoked install does.
   **No new grant table.**
3. **Amend the surface rule per kind, not blanket.** `widget` and `report_render`
   stay refused on connectors, on the original UI-coherence reasoning, and the
   manifest comment is corrected to say so precisely. `automation_action`,
   `automation_trigger` and `read` become available to connectors. The argument
   that carries it: an `http_connector` already holds credentials and network
   egress as its reason for existing, so an action grants it nothing new. The
   wasm case is the reverse, because a sandbox is capability-restricted and
   granting it a network action *is* an escalation, which is presumably why the
   rule started there.
4. **A node declares six fields, not one.** `plugin_surface_entries.definition`
   carries an input schema today; a workflow node needs `kind`
   (`read`/`action`/`trigger`), `inputSchema`, `outputSchema`, `idempotent`,
   `emits[]` and `audience`, plus a model-facing `agentDescription`. Two of these
   are load-bearing rather than cosmetic. `outputSchema` is what makes a plugin
   node usable downstream, since without it a workflow can call a plugin and
   cannot use what came back. `emits[]` is the input to ADR 0016's static loop
   refusal, and it is what stops a plugin that emits `ticket.status_changed` from
   deadlocking a workflow that then waits for it.
5. **`read` carries stricter guardrails than a write,** because reading is where
   the volume and the blast radius live. Rate-capped per run and per org, since a
   loop over devices is a denial of service aimed at the customer's own RMM.
   Bounded output, since a device list of ten thousand must not grow the run
   ledger by megabytes. It may never return a vault secret, because the run ledger
   is a place operators read. And it gets its own consent line, because "can read
   your RMM" is a different promise from "can act in your RMM".
6. **Inbound is the documented default and needs no surface at all.** A connector
   HMACs a POST to the generic signed ingest declaring the event type it emits,
   and the event-type registry makes it subscribable. Most of a first-party
   connector's workflow value is inbound, so the authoring guide must say plainly
   that a connector should not reach for a surface to announce something.
7. **Credentials are resolved by the host, never by the document.** They are
   resolved from the install at execution time and are never workflow values,
   never ledger contents, and never a node output. The registry entry is
   re-read at execution time, and `get_entry_by_key`
   (`crates/core-db/src/plugin_surface.rs:170`) already requires
   `i.status = 'enabled' AND i.revoked_at IS NULL`, so a revoked install cannot
   run. A run whose node is revoked goes to `flow.fail` naming the entry, because
   a silently half-finished workflow is the worst outcome available.

## Consequences

**Good.** The RMM, NMS and identity cases become reachable, and the third-party
case costs the same code as the first-party one because authorship is not part of
the decision. The grant mechanism is the column that already exists rather than
a new table, so decision 2 is close to free. A connector that emits its own
workflow's trigger event becomes impossible to author, by validation, rather than
by a depth counter.

**Costs, stated plainly.** `read` is a genuine new capability with a new abuse
shape, and the rate cap is a product setting that needs a default. Grant
withdrawal is a new state an author will hit: a workflow authored last month may
have nodes the palette no longer shows, so the canvas must render a removed grant
as a visible, explained placeholder rather than a broken node. The six-field
contract is a breaking change to every existing `automation_action` manifest,
which is three wasm plugins and one marketplace surface, so the cost is bounded
but real and must ship with defaults for the two new required fields.

**Rejected alternatives.** Keeping the blanket wasm-only rule would have made the
RMM case unreachable and pushed users toward generic HTTP, which is strictly
worse. A first-party versus third-party capability split would have created a
class of code we cannot test and a second thing to reason about at every consent
screen. A new grant table would have duplicated `granted_capabilities` and split
the truth about what an install may do.

## Dependencies

Wave 43 lane 4 (`p43-plugin-webhook-surface`) implements decisions 3 through 7.
Lane 13 (`p43-node-registry`) defines the six-field contract and derives Tier C
from it. Lane 15 (`p43-workflow-executor`) consumes `idempotent` for retry
safety and `emits[]` for the static loop refusal, and fails a run whose entry was
revoked mid-flight. ADR 0016's inbound path remains the default for connectors.

**Remaining open question.** Whether plugin nodes should also be exposed on
`crates/mcp-server`, so a user's own LLM builds workflows against the same
registry the palette reads. Genuinely useful, out of scope here, and recorded in
the node catalogue.