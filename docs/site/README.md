# Docs site (Diátaxis) — Phase 4

> This page is for people maintaining the docs. If you are looking for product
> help, start at [`index.md`](index.md) instead.

The Diátaxis site for Marula. Four quadrants, each kept in its lane, plus a
wayfinding layer in front of them:

- [`start/`](start/) — task entry points by reader: owner, admin, technician, operator, agent
- [`tutorials/`](tutorials/) — learning-oriented lessons (one worked path)
- [`how-to/`](how-to/) — task-oriented guides (safe paths to one goal)
- [`reference/`](reference/) — descriptive, exhaustive, generated
- [`explanation/`](explanation/) — understanding-oriented (the why)

[`troubleshooting.md`](troubleshooting.md) sits outside the quadrants on purpose:
it is entered by symptom, not by document type.

## Coverage & IA

The current coverage audit and information-architecture plan (Wave 27) lives
at [`DOCS-COVERAGE.md`](DOCS-COVERAGE.md). Read it before adding pages. It
carries the quadrant conventions and the audience rules.

## Generated pages (do not edit by hand)

| Page | Source | Regenerate |
|---|---|---|
| [`reference/api.md`](reference/api.md) | `packages/sdk/openapi.json` (compact pin, DX12) | `node docs/site/scripts/generate-api-reference.mjs` |
| [`reference/upgrade-matrix.md`](reference/upgrade-matrix.md) | `fixtures/upgrades/` | `node docs/site/scripts/generate-upgrade-matrix.mjs` |

## Verify

```sh
node docs/site/scripts/check-docs-site.mjs
```

Checks every relative link resolves, both generated pages still match
regeneration from the pin + fixtures (stale pages fail), and the publishing
path works: `publish.mjs` renders a complete, byte-deterministic HTML bundle
with no dead in-bundle hrefs (ADR `../adr/0009-docs-publishing-pipeline.md`).
It then runs the information-architecture checker, so one command covers both.

```sh
node docs/site/scripts/check-docs-ia.mjs
```

Checks the tree rather than the links: every page is reachable from
[`index.md`](index.md), every page under a quadrant is listed in that
quadrant's index, no `#anchor` points at a heading that does not exist, the
`**Audience:**` tags use the agreed vocabulary, and every page here has a
path from `docs/README.md`. A new page that nothing links is a failure, not a
warning.

## Adding a page

1. Pick the quadrant. A task someone performs goes in `how-to/`, a worked path
   in `tutorials/`, a table or catalogue in `reference/`, and a "why" in
   `explanation/`.
2. Open with the `> **Audience:** …` line and a `## Prerequisites` section.
3. Add it to that quadrant's `index.md`, and to the reader's entry page under
   [`start/`](start/) if it answers one of their tasks.
4. If the task has no page yet, add a line to that entry page's
   **Not documented yet** list instead of describing behaviour you have not
   checked.
5. Run both checkers.

## Publish

```sh
node docs/site/scripts/publish.mjs
```

Renders the static publication to `dist/` (mirrors this directory tree;
repo-native references drop to their link text). The Phase-2 website
consumes `dist/` wholesale — the markdown here stays the only source of
truth, and these sources are never forked into a publishing format.

## Rules

- `MANIFESTO.md` is the canonical positioning narrative — quoted **verbatim**
  (`explanation/why-marula.md`, pull quotes on `index.md`), never
  re-synthesized.
- No product app code is imported (packages/ui primitives allowed only).
- Runbooks: `../../deploy/runbooks/` · ADRs: [`../adr/`](../adr/) · RFCs: `../rfc`.
