# Docs site (Diátaxis) — Phase 4

The Diátaxis site for Meridian. Four quadrants, each kept in its lane:

- [`tutorials/`](tutorials/) — learning-oriented lessons (one worked path)
- [`how-to/`](how-to/) — task-oriented guides (safe paths to one goal)
- [`reference/`](reference/) — descriptive, exhaustive, generated
- [`explanation/`](explanation/) — understanding-oriented (the why)

## Generated pages (do not edit by hand)

| Page | Source | Regenerate |
|---|---|---|
| [`reference/api.md`](reference/api.md) | `packages/sdk/openapi.json` (compact pin, DX12) | `node docs/site/scripts/generate-api-reference.mjs` |
| [`reference/upgrade-matrix.md`](reference/upgrade-matrix.md) | `fixtures/upgrades/` | `node docs/site/scripts/generate-upgrade-matrix.mjs` |

## Verify

```sh
node docs/site/scripts/check-docs-site.mjs
```

Checks every relative link resolves and both generated pages still match
regeneration from the pin + fixtures (stale pages fail).

## Rules

- `MANIFESTO.md` is the canonical positioning narrative — quoted **verbatim**
  (`explanation/why-meridian.md`, pull quotes on `index.md`), never
  re-synthesized.
- No product app code is imported (packages/ui primitives allowed only).
- Runbooks: `../runbooks/` · ADRs: `../adr/` · RFCs: `../rfc`.
