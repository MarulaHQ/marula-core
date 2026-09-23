# How to call the API

> **Audience:** both — developers and self-host operators calling the API directly.

The REST API is the single write surface for Meridian: all mutations flow
through it and emit domain events.

## Prerequisites

- A reachable core-api and a bearer token (the [API tutorial](../tutorials/first-ticket-api.md) obtains one); machine scopes come from [API keys](admin-api-keys.md).


## Authenticate

`POST /api/v1/auth/login` with email + password returns a token pair. Send
the access token as `Authorization: Bearer …`. MFA-enrolled orgs gate login
per org policy; federated (OIDC / edge-assertion) identities authenticate
through their configured sources.

## Read

Reads are plain resource-oriented REST under `/api/v1/…`. Collections are
paged (`?limit=&offset=`).

## Mutate through the instant-first envelope

Every mutation returns the instant-first `mutation` envelope: a sequence
number plus a version and an updated timestamp. Clients render optimistically and
reconcile against the authoritative response — never poll to "check if it
worked".

On conflict you get a typed `version_conflict`, not a generic error:
reconcile the copy, don't toast it.

## Handle errors

Failures return the typed envelope (IF3): a machine-readable reason code
plus a human-readable message. Never parse prose. Example reasons:
`org_slug_taken` (with a verified-available `suggestion`),
`already_seeded`, `board_move_status_conflict`, `slot no longer free`-style
409s for booking conflicts.

## Use the typed SDK

[`@meridian/sdk`](../../../packages/sdk/) is generated from the committed
OpenAPI snapshot — the same one the [API reference](../reference/api.md) is
generated from. If a route exists in the docs, it exists in the client.

```ts
import { createClient } from "@meridian/sdk";

const client = createClient({ baseUrl: "https://your-host" });
```

## Integrate

Connectors are ordinary web applications over the versioned API and signed
webhooks — the NinjaOne and Hudu connectors are the reference
implementations. External-DMS parity is a design requirement: everything
reachable in the console is reachable as plain REST plus registered
`asset.*` / `document.*` contract events.
