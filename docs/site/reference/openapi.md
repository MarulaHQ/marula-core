# The OpenAPI pin

The API contract is pinned twice, and CI enforces that the two are a
byte-identical pair:

- [`packages/sdk/openapi.json`](../../../packages/sdk/openapi.json) — the
  snapshot `@meridian/sdk` is generated from
- [`fixtures/openapi/v1/openapi.json`](../../../fixtures/openapi/v1/openapi.json)

## The compact rule (DX12)

Both pins are **compact single-line minified JSON**. The API serves
pretty-printed JSON, so after `curl /openapi.json > packages/sdk/openapi.json`
you must run `pnpm --filter @meridian/sdk generate` (it minifies the
snapshot in place) before committing. Never commit the ~43k-line pretty
form; `scripts/check-openapi-pins.sh` fails it in CI.

## What the pin guarantees

- The generated typed SDK client and this docs site's [API
  reference](api.md) are generated from the same committed snapshot, so they
  cannot drift from what the server serves.
- Path-versioned (`/api/v1`…): a supported API version never breaks inside
  its support window; new versions are added alongside, never swapped in
  place (policy §2).

## How this reference is generated

[`docs/site/scripts/generate-api-reference.mjs`](../scripts/generate-api-reference.mjs)
parses the compact pin (it never reads or pastes the pretty form) and emits
[`reference/api.md`](api.md). Regenerate after any OpenAPI regen:

```sh
node docs/site/scripts/generate-api-reference.mjs
```
