# @marula-core/connector-hudu

> **Purpose:** reference [Hudu](https://hudu.com) connector for Project
> Meridian — proof of the connector pattern: it talks to Meridian ONLY
> through the public REST API via `@marula-core/sdk` (typed client + scoped
> API key) and receives Meridian's HMAC-signed webhooks. Apache-2.0.
> **Non-purpose:** no Meridian internals are imported; no credential
> vaulting, no hosted/operated service.

## What it does

| Command  | Effect |
|----------|--------|
| `verify` | One typed SDK call (`GET /api/clients`) proving key validity + `clients:read` scope + connectivity. |
| `listen` | HTTP receiver for signed webhook deliveries (`POST /meridian/hook`). Verified `client.*` events are mirrored to Hudu companies — or **logged only** in none mode. |

Signature scheme (receiver side): `X-Meridian-Signature: t=<unix>,v1=<hex>`
over `"{t}.{rawBody}"` (HMAC-SHA256, constant-time compare) with a ±10 min
anti-replay window. Raw bytes are verified before any JSON parsing.

## Setup

1. In Meridian: **Settings → Integrations → Webhooks** — create a webhook
   filtering on `client.created` / `client.updated`, copy the signing
   secret **once shown** (`whsec_…`), point the URL at this receiver.
2. Create a scoped API key with `clients:read` (rotation story: create a
   new key, swap env, revoke the old).
3. Configure and run:

```bash
export MARULA_BASE_URL=https://meridian.example.test
export MARULA_API_KEY=mrd_…            # clients:read
export MARULA_WEBHOOK_SECRET=whsec_…

# Optional — absent ⇒ NONE MODE:
export HUDU_BASE_URL=https://yourcompany.huducloud.com
export HUDU_API_KEY=…

npm run verify
npm run listen                            # PORT defaults to 8789
```

## None mode

Without `HUDU_*` variables the connector runs fully functional against
Meridian — deliveries are verified, events parsed, and every external push
is printed as `[none-mode] would …`. Meridian itself never requires Hudu:
connectors enhance a working standalone product, never gate it.

## In-repo typecheck note

`tsc` resolves `@marula-core/sdk` through a local path mapping into
`packages/sdk/src/index.ts` for the acceptance probe. Outside this repo,
`npm install @marula-core/sdk` resolves normally — the import specifier is
the published package name, nothing else.
