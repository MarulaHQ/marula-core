# @meridian/connector-zapier

> **Purpose:** reference Zapier integration for Project Meridian — the
> OUT-and-IN loop over the public API only. The trigger side is RESTHooks
> fed by Meridian's signed webhooks with typed event filters. The action
> side writes tickets and clients through `@meridian/sdk` (typed client,
> scoped API key, idempotency keys), with device records riding the
> feed-upsert seam. Apache-2.0.
> **Non-purpose:** no Meridian internals are imported and no
> zapier-platform code is vendored. No hosted/operated service.

## What it ships

| File | Role |
|------|------|
| `src/events.ts` | The webhook event catalog mirrored from the public surface (14 typed event types), the delivery-envelope shape Meridian signs (`{id, type, org_id, entity, entity_id, occurred_at, payload_schema_ref, payload}`), and the RESTHook trigger set (ticket created / status / comment, client created, SLA escalation). |
| `src/meridian.ts` | Every typed SDK call the app needs: subscribe/unsubscribe webhook, create/update ticket, create client, upsert config item (rides the public feed-upsert seam — dedupe-aware, never a blind insert). |
| `src/zapier-app.ts` | The app definition in plain TypeScript, shaped per zapier-platform-schema (custom auth with `x-api-key`, triggers with subscribe/unsubscribe/perform, creates with input fields) so you can drop it into a `zapier init` scaffold. |
| `src/index.ts` | `verify` (live probe or none-mode printout) and `print-app` (definition as JSON). |

Actions carry an **idempotency key** on every write (Meridian IF1). Zapier
retries performs: the same key with the same payload replays the same
stored result, so a retried Zap never double-creates a record. Reuse a
key with a changed payload and Meridian answers with a typed 409 instead
of silently drifting.

## Setup

1. Create a scoped Meridian API key (Settings → Integrations). The Zapier
   auth fields are `baseUrl` + `apiKey`, and the key rides `x-api-key`.
2. For triggers, the app subscribes a RESTHook through `POST /api/webhooks`
   with an exact event-type filter per trigger (never "everything").
   Verify deliveries in Meridian under Settings → Integrations → Webhooks.

3. Scaffold and push:

```bash
npx zapier init my-meridian-zap   # pick a template, then copy src/ shapes in
npm install                       # resolves @meridian/sdk per package.json
npx zapier push
```

## None mode

Without `MERIDIAN_API_KEY` the definition still renders in full:
`npm run print-app` dumps the app definition JSON, and `npm run verify`
prints what it would probe. Meridian itself never requires Zapier:
integrations enhance a working standalone product, never gate it.

## In-repo typecheck note

`tsc` resolves `@meridian/sdk` through a local path mapping into
`packages/sdk/src/index.ts` for the acceptance probe (run it from the repo
root: `node_modules/typescript/bin/tsc --noEmit --typeRoots
apps/web/node_modules/@types --types node -p connectors/zapier`).
Outside this repo, `npm install @meridian/sdk` resolves normally — the
import specifier is the published package name, nothing else. The
`verify` script targets a built SDK package — in-repo it needs the
connector's own `npm install` to run.
