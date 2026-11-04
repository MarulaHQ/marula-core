# The agent set

> **Audience:** developer. What an automated agent may reach in this product, what it
> may not, and the machine-readable index of both.

An agent here is one of two things. It is an MCP client talking to `mcp-server` under the
session of the person it acts for, or a service key calling the HTTP API directly. Both
arrive at the same authorisation model, and both see the same surface, so this page
describes that surface once.

The MCP adapter holds no credential of its own. It forwards the session token it was
given, on the capability read and on every proxied tool call, so core-api authorizes the
person behind the session and not the adapter. Over HTTP the token is the request's
`Authorization: Bearer`; over stdio it is `MARULA_API_TOKEN`. A namespace the org has not
granted does not appear at all.

Both transports are served by the official `rmcp` SDK. It implements the stable
2026-07-28 MCP revision and stays compatible with 2025-11-25 and earlier, so a client
agrees its era at connect time. The adapter exposes tools only: resources, prompts,
sampling, elicitation and long-running tasks are out of scope, and no write tool exists.

[`manifest.json`](manifest.json) is the data half of this page: the namespaces an agent
can be granted, the surfaces it reaches them through, and the prohibitions with the code
that enforces each one. Read it when you are wiring a client. [`scripts/check-agent-manifest.mjs`](../../scripts/check-agent-manifest.mjs)
keeps it honest against the API spec.

## What an agent may do

- **Read what its grant covers.** The MCP tool catalog is derived from the OpenAPI
  snapshot, so every `GET` route with an `operationId` becomes one tool
  (`crates/mcp-server/src/catalog.rs:32-96`).
- **Call the same routes over HTTP** with a service key, where a client would rather
  speak REST than MCP.
- **Follow the event stream.** The live surface is a read of the outbox, not a second bus
  (`crates/core-api/src/routes/events.rs`, [extension points](../reference/extension-points.md)).
- **Export the audit trail**, walking forward on `(occurred_at, id)` with a stable
  cursor. `observability:read` is the grant, and a plugin capability maps onto it.
- **Read per-org usage and quota settings.** Configured limits are reported and nothing
  in the core refuses a request because of one.

## What an agent may never do

Each rule below is refused in code rather than by convention. The manifest carries the
same list with the file that enforces it.

| Never | Stopped by |
|---|---|
| Write through the MCP catalog | The catalog derives `GET` operations only, and `catalog_derives_get_tools_only` fails if that changes (`crates/mcp-server/src/catalog.rs:201`) |
| Reach a namespace it was not granted | `filter_granted` drops an ungranted namespace from `tools/list` entirely, so the tool is not hidden, it does not exist (`catalog.rs:102-111`) |
| Read sensitive content even with a grant | `EXCLUDED_PATH_PREFIXES` removes attachments, export bundles, webhook delivery logs, API keys, auth, email, internal and portal paths before grants are considered (`crates/mcp-server/src/lib.rs:57-75`) |
| Hold an administrative special | No plugin capability maps onto one, and `admin_specials_should_never_be_grantable` fails the build if one does ([extension points](../reference/extension-points.md)) |
| Write, edit or delete an audit row | The audit surface is export only, org-scoped, with no write path ([extension points](../reference/extension-points.md)) |
| Import a core crate to work around any of this | A connector talks to the public API and carries no path dependency into `crates/` |

## The surfaces

| Surface | Transport | Source | Notes |
|---|---|---|---|
| MCP | stdio JSON-RPC | `crates/mcp-server/src/stdio.rs` | The canonical transport. No arguments runs this; the session token is `MARULA_API_TOKEN` |
| MCP | `POST /mcp` on loopback | `crates/mcp-server/src/http.rs:9-12` | `mcp-server serve-http`, for local development; the token is the request's bearer header |
| REST | HTTPS | `packages/sdk/openapi.json` | 651 paths in the pinned snapshot. A service key scopes what it may call |
| Events | SSE | `crates/core-api/src/routes/events.rs` | A read of the outbox stream, bounded by `MARULA_EVENT_STREAM_MAXLEN` |
| Audit | HTTPS | `crates/core-api/src/routes/audit.rs` | Forward-only cursor export |

## How a tool gets its name

Three rules, all in the adapter and all checked:

1. A route needs an `operationId`, a `GET`, and a path under `/api/`, or it never becomes
   a tool (`catalog.rs:32-96`).
2. The capability namespace is the first segment after the version prefix, so
   `/api/v1/tickets/123` derives `tickets` and the tool holds `tickets:read`
   (`crates/mcp-server/src/lib.rs:80-89`).
3. The org's granted namespaces intersect that list, deny-by-default. No grant, no tool.

That is why the manifest lists namespaces rather than 303 tool names. The names are
derived at runtime from the spec, and a hand-maintained list of them would be a second
copy that drifts the first time somebody adds an endpoint.

## Reading and checking the manifest

```json
{
  "schema_version": 1,
  "namespaces": ["admin", "alerts", "assets", "..."],
  "sensitive": { "admin": "why it is powerful" },
  "surfaces": [ { "id": "mcp-stdio", "source": "crates/mcp-server/src/stdio.rs" } ],
  "prohibitions": [ { "id": "...", "statement": "...", "enforced_by": "file:line" } ]
}
```

```sh
node scripts/check-agent-manifest.mjs
```

The check derives the namespace set from `packages/sdk/openapi.json` with the same rules
the adapter uses, then fails in both directions: a namespace the manifest documents and
the spec no longer has, and a namespace the spec exposes that the manifest does not
describe. A new namespace has to be written down, and a removed one cannot linger.

## Where to go next

[Extension points](../reference/extension-points.md) is the boundary a pack builds on,
[core and packs](../reference/core-and-packs.md) explains what stays in the
core, and [the reliability matrix](../reference/reliability-matrix.md) covers what the
surface promises under load.
