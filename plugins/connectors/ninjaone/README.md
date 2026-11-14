# @marula-core/connector-ninjaone

> **Purpose:** reference [NinjaOne](https://www.ninjarmm.com) connector for
> Project Marula — proof of the connector pattern: it talks to Marula
> ONLY through the public REST API via `@marula-core/sdk` (typed client +
> scoped API key) and receives Marula's HMAC-signed webhooks. Apache-2.0.
> **Non-purpose:** no Marula internals are imported; no credential
> vaulting, no hosted/operated service.

## What it does

| Command  | Effect |
|----------|--------|
| `verify` | One typed SDK call (`GET /api/tickets`) proving key validity + `tickets:read` scope + connectivity. |
| `listen` | HTTP receiver for signed webhook deliveries (`POST /marula/hook`). Verified `ticket.*` events are pushed to NinjaOne as device activity notes — or **logged only** in none mode. |

Signature scheme (receiver side): `X-Marula-Signature: t=<unix>,v1=<hex>`
over `"{t}.{rawBody}"` (HMAC-SHA256, constant-time compare) with a ±10 min
anti-replay window. Raw bytes are verified before any JSON parsing.

## Setup

1. In Marula: **Settings → Integrations → Webhooks** — create a webhook
   with your event filter and copy the signing secret **once shown**
   (`whsec_…`), pointing the URL at this receiver.
2. Create a scoped API key with `tickets:read` (rotation story: create a
   new key, swap env, revoke the old).
3. Configure and run:

```bash
export MARULA_BASE_URL=https://marula.example.test
export MARULA_API_KEY=mrl_…            # tickets:read
export MARULA_WEBHOOK_SECRET=whsec_…

# Optional — absent ⇒ NONE MODE:
export NINJAONE_BASE_URL=https://app.ninjarmm.com
export NINJAONE_TOKEN=…                  # OAuth2 client-credentials token

npm run verify
npm run listen                            # PORT defaults to 8788
```

## Delivery contract

`listen` accepts `POST /marula/hook` and nothing else (`404` elsewhere). A body
over 64 KiB is refused with `413` while it is still being read, before the
signature is checked. A failed push answers `502`, so the sender retries: there
is no queue behind this receiver, and `200` means the work happened.

A `ticket.*` event must carry `payload.ticket_id` and `payload.device_id` (the
NinjaOne device id to post against). Missing either is `400`, with the field
named in the body. There is no ticket-to-device mapping in Marula's public
payload today, so an event without a device is refused rather than posted to a
guessed one. `payload.ticket_number` and `payload.ticket_title` are optional and
only refine the note's label.

## None mode

Without `NINJAONE_*` variables the connector runs fully functional against
Marula — deliveries are verified, events parsed, and every external push
is printed as `[none-mode] would …`. Marula itself never requires
NinjaOne: connectors enhance a working standalone product, never gate it.

## In-repo typecheck note

`tsc` resolves `@marula-core/sdk` through a local path mapping into
`packages/sdk/src/index.ts` for the acceptance probe. Outside this repo,
`npm install @marula-core/sdk` resolves normally — the import specifier is
the published package name, nothing else.
