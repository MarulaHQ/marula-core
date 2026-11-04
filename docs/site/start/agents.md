---
description: "Marula separates three things that are easy to blur: what an agent may do (grants), what it did (provenance and audit), and what the platform exposes (API…"
---

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

## Connecting an MCP client

The adapter is `mcp-server`, built on the official `rmcp` SDK. It speaks the stable
2026-07-28 revision and stays compatible with 2025-11-25 and earlier, so the client
picks an era at connect time.

Over stdio, the canonical transport:

```sh
MARULA_API_BASE=http://127.0.0.1:8080 MARULA_API_TOKEN=<access token> mcp-server
```

Over HTTP, for local development:

```sh
MARULA_API_BASE=http://127.0.0.1:8080 mcp-server serve-http
```

The endpoint is `POST /mcp`, and each request carries `Authorization: Bearer <access
token>` plus the standard MCP headers its revision requires. Behind a real hostname,
set `MARULA_MCP_ALLOWED_HOSTS` to the names the adapter should accept.

The token is the caller's own session, not an adapter key. The tool list is that
organization's granted reads, and every call runs as the person behind the token, so
core-api's permissions and row scoping decide what comes back.

## What the surface offers

Three surfaces share one rule: the organization's capability grant decides what
exists, and every read runs as the caller's session.

- **Tools**: one per safe read operation in the committed OpenAPI snapshot.
- **Resources**: `marula://tickets/{id}`, `marula://clients/{id}` and
  `marula://documents/{id}`. A resource is not a second permission. It resolves
  through the same catalog entry as the equivalent tool call, so the path, the
  capability and the API's own check are identical. The adapter publishes the
  templates, not an enumeration of your records; use a list tool when you need to
  page a collection.
- **Prompts**: `triage_ticket`, `client_qbr_pack` and `document_review`. Each takes
  one record id, and each is listed only when your session can perform the first
  read the workflow needs.

The last segment of a resource URI is taken literally: ASCII letters, digits, `-`
and `_`. Anything else, a percent escape or a second segment included, is refused
rather than decoded, so a URI cannot point at a different endpoint than the one it
names.

Tasks are not part of this surface yet. Long-running work stays with the API's own
async operations, and the server answers a task request with method-not-found
instead of advertising something it does not run.

## Acting as an agent, not as a person

A person's session is the right credential while a person is watching. For an agent
that runs unattended, an admin creates a service account holding the permission keys
the agent needs, then mints an API key bound to it:

```sh
curl -X POST "$MARULA_API_BASE/api/v1/api-keys" \
  -H "authorization: Bearer $ADMIN_TOKEN" \
  -H "content-type: application/json" \
  -d '{"name": "triage-agent", "scopes": [], "service_account_id": "<account id>"}'
```

Creating a key needs the organization's key-management permission. The key carries the
account: its permissions, its expiry, its revocation. Leave `scopes` empty, because a
bound key takes the account's own keys and naming both is refused, so the two
permission models never mix on one credential. The response is the only time the secret
appears.

Present the key the way any other MCP caller presents a token: `Authorization:
Bearer <key>` over HTTP, or `MARULA_API_TOKEN` over stdio. The adapter decides
the header by the token's shape when it forwards to core-api: a session token
rides `Authorization`, a key rides `x-api-key`, which is where each one belongs. `GET /api/v1/me` answers `"subject": "agent"` for such a
caller, with the account's permissions and the organization's grants, and the
surface filters on exactly that.

## Not documented yet

- A machine-readable docs index for agents. Every page here is prose for a
  person. Nothing publishes a manifest an agent can load and trust.
- The domain event catalogue: which events exist, their payloads, and which ones
  are safe to subscribe to. Automation pages name triggers; no page lists them all.
- Plugin authoring from an empty directory: the SDK package exists, and no page
  walks it to a published plugin.
