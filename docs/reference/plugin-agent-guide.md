# Plugins and agents

**Related:** [plugin-capabilities.md](plugin-capabilities.md), [plugin-host-functions.md](plugin-host-functions.md),
[workflow-node-catalogue.md](workflow-node-catalogue.md)

An agent reaches a plugin through the same workflow engine a human rule uses. There is no plugin
API for agents, no second authentication, and no grant an agent can hold that an install does not.
This page states what an agent sees, what the author owes it, and where the limits sit.

## What an agent sees

A plugin's surface entries become tier-C nodes in the workflow catalogue. The catalogue is a
read-only declaration, and the data a node returns is gated by the capability it registered under.

`GET /api/workflows/nodes?audience=ai` returns the nodes an agent may use, filtered three ways:

| Filter | Values | Meaning |
| --- | --- | --- |
| `audience` | `human`, `ai`, `both` | who the node is written for |
| `tier` | `A`, `B`, `C` | `C` is a plugin contribution |
| `family` | the node families | action, trigger, condition, and so on |

Each returned node carries the fields an authoring model reads: `id`, `label`,
`agent_description`, `input_schema`, `output_schema`, `idempotent`, `emits`, and `executable`. The
catalogue also carries a version and a digest, so a cached copy can be compared against the live
one.

An agent's view of a plugin is therefore exactly what the plugin's manifest declared, in the words
the author wrote. Nothing about the plugin's internals is visible, and nothing about the org's data
is in the catalogue.

## What an agent may invoke

Only nodes whose `executable` flag is true. For a plugin contribution that means an
`automation_action`: its node rides the `plugin.action` arm of the workflow engine. A plugin *read*
declares its kind and schemas but is not advertised as runnable, because the engine has no arm of
its own for it yet.

When an agent's run reaches a `plugin.action` node, the engine does what a human rule's run does:

1. resolves the entry key to a surface row, which exists only while the install is enabled and the
   grant stands,
2. runs `authorize_invocation` against the install's own grants and the permission matrix;
3. validates the node's params against the entry's `params_schema`;
4. runs the guest under the sandbox limits,
5. applies the consented effect in the run's transaction, or declines it,
6. writes one audit row naming the install as the actor.

The agent's own permissions do not widen any of that. An org that grants a plugin
`automation.actions` has decided what the plugin may do; the agent deciding *when* to call it is a
separate question, answered by the org's automation permissions and the run's approval path.

## What the author owes an agent

An agent reads declarations in a prompt, with no room to ask a follow-up question. Four fields do
that work, and an author who leaves them vague produces a node an agent uses wrongly.

| Field | What an agent does with it |
| --- | --- |
| `agent_description` | the sentence an authoring model reads to decide whether this node fits a request. It is inlined into the prompt, so it must say what the node does and when to use it. |
| `audience` | whether the node appears in an agent's catalogue at all (`ai` or `both`) |
| `input_schema` / `output_schema` | the shapes an agent composes params from and reads a result with. An absent schema defaults to `{"type":"object"}`, which tells an agent nothing. |
| `idempotent` | whether a retry is safe. Defaults to false, because an undeclared plugin's retry safety is unknown, and a false makes the retry path refuse rather than guess. |

Two consequences worth planning for.

- **A node with no `agent_description` is described by its label.** That is the fallback in the
  registry, and it is a poor prompt.
- **A node whose `emits` lists an event the run then waits for is a deadlock the engine refuses.**
  The static loop refusal reads `emits` when it is declared. An action that declares an event it
  does not publish is not caught by that check, so declare only what the plugin really emits.

## What an agent may read

A plugin's model reads, when they ship, go through the same four query ids every guest uses, each
gated by a capability:

| Id | Query | Capability |
| --- | --- | --- |
| 1 | `ticket.by_id` | `tickets.read` |
| 2 | `board.list` | `tickets.read` |
| 3 | `client.by_id` | `clients.read` |
| 4 | `contact.by_client` | `clients.read` |

The fetch behind each id is not implemented in this build, so a plugin that asks today receives
`plugin_not_implemented`. The ids, the table and the gate are fixed, which is what makes a later
implementation a body change rather than an ABI change.

An agent never receives a plugin's KV, its logs or its egress credentials. Those are the install's
own scope: a guest reads its own namespace, and a log line is forwarded to the deployment's log
stream rather than to the caller.

## What an agent must not ask a plugin to do

- **Do not design an agent flow around `emit_event` or `read_model_query`.** Both refuse today, and
  a run that depends on either will skip or fail at that node.
- **Do not use an agent to work around a grant.** An agent's task text cannot add a capability. The
  grant set is the install row's, and a payload that names capabilities is bytes.
- **Do not let an agent approve its own widening.** Granting a capability, enabling an install and
  clearing a quarantine are human admin actions. They are deliberately outside the agent platform's
  reach.
- **Do not route a write through a plugin read.** A read node declares a payload shape and returns
  data. A write is an automation action with a consented effect, and that effect is
  `add_internal_comment`.

## Skills a plugin contributes

A plugin may also ship an agent skill bundle rather than a surface. Skills are staged, approved and
enabled per org through the same distribution path (`/api/v1/plugins/distribution/agent-skills`),
and they appear on the Plugins page's **Agent skills** tab. A skill carries instructions, not code:
it cannot import a host function and it runs no guest, so the limits on this page do not apply to
it. What does apply is the same rule as everywhere else — an admin decides, and the decision is
audited.

## Where to look when something is wrong

| Symptom | Meaning |
| --- | --- |
| a plugin node is absent from the `audience=ai` catalogue | the surface declares `audience: human`, or the install is not enabled |
| the node is present but `executable` is false | it is a read, and the engine has no arm for it yet |
| the node runs and skips with `plugin_entry_unavailable` | the install was disabled or revoked after the run was authored |
| the node fails with `plugin_worker_failed` | the worker died; the run fails rather than skipping |
| a refusal the agent did not expect | read the audit row: the reason string there is the same one the run reports |
