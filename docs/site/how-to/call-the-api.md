---
description: "Every action in the Marula web app, desktop app, and mobile app goes through the same /api/v1/… REST API."
---

# How to call the API

> **Audience:** both — developers and self-host operators calling the API directly.

Every action in the Marula web app, desktop app, and mobile app goes through the same `/api/v1/…` REST API. You can call those exact same endpoints from your own scripts using `curl` or the typed TypeScript client (`@marula-core/sdk`).

## Prerequisites

- A reachable `core-api` server and either a user bearer token (see the [API first-ticket tutorial](../tutorials/first-ticket-api.md)) or a scoped machine token from **Settings → API keys** ([see How to manage API keys](admin-api-keys.md)).

## 1. Authenticate

For interactive user scripts, send email and password to `POST /api/v1/auth/login` to receive an access/refresh token pair. For background scripts and integrations, create a scoped key in **Settings → API keys**.

Send your token on every request in the `Authorization` header:

```http
Authorization: Bearer <your-token>
```

## 2. Read paged collections

Read endpoints use standard `GET /api/v1/…` routes and support pagination query parameters (`?limit=&offset=`).

## 3. Write data and handle the instant-first envelope

Every `POST`, `PATCH`, `PUT`, or `DELETE` mutation returns Marula's instant-first `mutation` envelope containing the updated record, its new `version`, a monotonic sequence number, and the updated timestamp.

If someone else updated the same record at the same moment, the API returns a `409 Conflict` with code `version_conflict` so your client can reconcile cleanly without overwriting their edit ([see Instant-first doctrine](../explanation/instant-first.md)).

## 4. Handle structured error codes

When a request is rejected, Marula returns a structured error object with a machine-readable `reason` code and a clear human message—so your scripts never have to regex-match error text. Common examples include:

- `org_slug_taken` (includes a verified-available `suggestion` field)
- `already_seeded`
- `board_move_status_conflict`

## 5. Use the typed TypeScript SDK

[`@marula-core/sdk`](../../../packages/sdk/) is generated directly from the committed OpenAPI specification (the same spec that generates the [API reference](../reference/api.md)):

```ts
import { createClient } from "@marula-core/sdk";

const client = createClient({ baseUrl: "https://your-host" });
```

## 6. Build external connectors

External connectors are standard web services that call the `/api/v1/…` REST endpoints and receive HMAC-signed webhooks. Check `plugins/connectors/` (such as the NinjaOne and Hudu reference connectors) and [How to extend Marula with plugin surfaces](admin-plugin-surfaces.md) for working examples.
