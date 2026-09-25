# @meridian/connector-ninjaone

> **Purpose:** reference [NinjaOne](https://www.ninjarmm.com) connector for
> Project Meridian — proof of the connector pattern: it talks to Meridian
> ONLY through the public REST API via `@meridian/sdk` (typed client +
> scoped API key) and receives Meridian's HMAC-signed webhooks. Apache-2.0.
> **Non-purpose:** no Meridian internals are imported; no credential
> vaulting, no hosted/operated service.

## What it does

| Command  | Effect |
|----------|--------|
| `verify` | One typed SDK call (`GET /api/tickets`) proving key validity + `tickets:read` scope + connectivity. |
| `listen` | HTTP receiver for signed webhook deliveries (`POST /meridian/hook`). Verified `ticket.*` events are pushed to NinjaOne as device activity notes — or **logged only** in none mode. |

Signature scheme (receiver side): `X-Meridian-Signature: t=<unix>,v1=<hex>`
over `"{t}.{rawBody}"` (HMAC-SHA256, constant-time compare) with a ±10 min
anti-replay window. Raw bytes are verified before any JSON parsing.

## Setup

1. In Meridian: **Settings → Integrations → Webhooks** — create a webhook
   with your event filter and copy the signing secret **once shown**
   (`whsec_…`), pointing the URL at this receiver.
2. Create a scoped API key with `tickets:read` (rotation story: create a
   new key, swap env, revoke the old).
3. Configure and run:

```bash
export MERIDIAN_BASE_URL=https://meridian.example.test
export MERIDIAN_API_KEY=mrd_…            # tickets:read
export MERIDIAN_WEBHOOK_SECRET=whsec_…

# Optional — absent ⇒ NONE MODE:
export NINJAONE_BASE_URL=https://app.ninjarmm.com
export NINJAONE_TOKEN=…                  # OAuth2 client-credentials token

npm run verify
npm run listen                            # PORT defaults to 8788
```

## None mode

Without `NINJAONE_*` variables the connector runs fully functional against
Meridian — deliveries are verified, events parsed, and every external push
is printed as `[none-mode] would …`. Meridian itself never requires
NinjaOne: connectors enhance a working standalone product, never gate it.

## In-repo typecheck note

`tsc` resolves `@meridian/sdk` through a local path mapping into
`packages/sdk/src/index.ts` for the acceptance probe. Outside this repo,
`npm install @meridian/sdk` resolves normally — the import specifier is
the published package name, nothing else.
