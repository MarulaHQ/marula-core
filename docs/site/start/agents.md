# Agent and automation tasks

> **Audience:** developer or admin building something that acts on Marula — an
> AI agent, an automation rule, a plugin, or a script over the API.

Marula separates three things that are easy to blur: what an agent **may** do
(grants), what it **did** (provenance and audit), and what the platform
**exposes** (API, read model, plugin surfaces). Each has a page. When something
that used to work stops working, [Troubleshooting](../troubleshooting.md) starts
from the symptom.

## "I need to call the API"

- [Call the API](../how-to/call-the-api.md) — authenticate, page a collection,
  handle the optimistic write envelope, and use `@marula-core/sdk`.
- [API reference](../reference/api.md) — every endpoint, generated from the
  committed OpenAPI specification.
- [The OpenAPI pin](../reference/openapi.md) — how the published spec and the
  fixture stay identical, so you can trust the reference you generated against.
- [Instant-first interaction](../explanation/instant-first.md) — why writes carry
  a version and why undo exists. Read this before you write a client that retries.

## "I need data, not endpoints"

- [Report against the read model](../how-to/read-model.md) — why reports read
  stable `ops.v1_*` views instead of the write tables.
- [Read model view catalogue](../reference/read-model.md) — each view and its
  columns.

## "What is the agent allowed to touch"

- [AI: deny-by-default and the decision harness](../explanation/ai-deny-by-default.md) —
  the master switch, per-role surface grants, and the probability scores the
  triage and routing models record.
- [Manage AI access](../how-to/admin-ai-access.md) — the grant screen itself.
- [Your first agent run](../tutorials/first-agent-run.md) — plan, approve, run
  once, then read what the run left behind.
- [Provenance vs audit](../explanation/provenance-vs-audit.md) — how an `agent`
  actor shows up next to human and import edits, and which ledger answers which
  question.

## "I am writing an automation, not an AI"

- [Author automation rules](../how-to/admin-automation-rules.md) — event triggers,
  actions, and the dry run that shows what a rule would have done.
- [Extend Marula with plugin surfaces](../how-to/admin-plugin-surfaces.md) —
  declarative widgets, triggers, read nodes, action steps and field validators.
- [Workflow node catalogue](../../reference/workflow-node-catalogue.md) — every
  node the builder offers.

## "I am building a plugin"

- [Configure the plugin consent flow](../how-to/admin-plugin-consent.md) — what the
  person installing your plugin sees, and why the capability list is the whole
  negotiation.
- [Plugin runtime](../../reference/plugin-runtime.md) — the host, the capability
  model, and the distribution contract.
- [The agent platform spec](../../10-agent-platform.md) — the normative reference
  the implementation is measured against.

## "I need the MCP surface"

The capability endpoints are in the [API reference](../reference/api.md): `GET`,
`PUT` and `DELETE /api/v1/mcp/capabilities` grant, re-dial and revoke a capability
for the organization. A capability you have not been granted is not visible in the
inventory, which is the deny-by-default rule applied to tool discovery.

## Not documented yet

- Connecting an MCP client to a running instance: the transport it uses, how it
  authenticates, and a worked session. The endpoints exist in the reference; the client path is not
  written.
- A machine-readable docs index for agents. Every page here is prose for a
  person. Nothing publishes a manifest an agent can load and trust.
- The domain event catalogue: which events exist, their payloads, and which ones
  are safe to subscribe to. Automation pages name triggers; no page lists them all.
- Plugin authoring from an empty directory: the SDK package exists, and no page
  walks it to a published plugin.
