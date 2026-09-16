# ADR 0009: Docs publishing pipeline — deterministic in-repo publisher

Date: 2026-09-16 · Status: Accepted · Lane: `p15-docs-closeout` (Wave 15 lane 4)

## Context

`docs/site/` has shipped since Wave 14 lane 6 as the Diátaxis documentation
set: markdown sources + deterministic generators (API reference from the
OpenAPI pin, upgrade matrix from `fixtures/upgrades/`), verified by
`docs/site/scripts/check-docs-site.mjs` (link resolution + generator
freshness). What it deliberately did NOT decide (the lane-6 deferral) is the
**publishing pipeline**: how the markdown becomes a hosted/published render
when the live marketing site (ADR-adjacent plan: `docs/11-web-presence.md`)
moves past its waitlist phase.

The decision was forced now (Wave 15 lane 4) so the publishing path can ship
verified, not just planned. Two options were on the table — plus one quietly
in front of us:

1. **Starlight-class docs framework** (Astro + Starlight, or MkDocs-style).
2. **Website-package integration** (`website/` Next.js MDX/components route).
3. **A deterministic in-repo publisher** — render `docs/site` markdown with
   a dependency-free script into a static bundle the website can consume.

## Decision

Markdown under `docs/site/` stays the **single source of truth**, and the
publishing target is (3): the shipped deterministic publisher
`docs/site/scripts/publish.mjs` + the zero-dependency renderer
`docs/site/scripts/lib/minidown.mjs`.

- `node docs/site/scripts/publish.mjs` renders every page into
  `docs/site/dist/` as self-contained static HTML (light/dark by
  `prefers-color-scheme`, reduced-motion honored — the standing motion law
  applies to the document bundle too). No timestamps, no locale, no
  directory-order dependence: same sources ⇒ byte-identical output.
- The bundle mirrors the site's directory structure; every in-bundle link
  resolves. Repo-native references (e.g. `../../../MANIFESTO.md`) are
  DROPPED to their link text in the publication — the markdown keeps them
  for GitHub, the bundle never links a path it does not carry.
- `check-docs-site.mjs` now verifies the publishing path: it publishes
  twice into disposable dirs, asserts completeness (every markdown page has
  its published twin), byte-determinism, and a dead-href sweep.
- Consumer seam: the Phase-2 website build copies `docs/site/dist/`
  wholesale (no markdown rewrite, no MDX conversion, no component
  re-implementation). Until then the bundle is openable from the filesystem
  as-is.

## Rejected alternatives, and why

**Starlight-class framework.** Powerful (sidebar/IA/search), but it brings a
whole toolchain to a 19-page site, introduces markdown-frontmatter/config
churn, and tempts editing sources INTO framework form — the "forking the
markdown sources" failure mode the decision brief explicitly guards
against. Revisit only if the doc corpus grows past what hand-maintained
navigation actually serves; the renderer is the seam a migration would
replace, and the sources survive untouched.

**Website-package integration.** Binds the docs corpus to Phase-2 website
work (not started beyond the waitlist scaffold), pulls markdown rendering
into React/MDX build churn, and couples doc-layer verification (which must
stay cheap and owner-readable: plain markdown, links, pins) to the
marketing site's build system. The publisher's `dist/` placeholder means a
future MDX route can consume the SAME sources if the product wants richer
docs-run components — that path stays open without blocking on it.

## Consequences

- One renderer (never a second per consumer), narrow construct surface:
  if a new page needs more markdown constructs, they are added to
  `minidown.mjs` with evidence in the corpus — not worked around with raw
  HTML.
- Publishing is a pure function of the repo (no network, no deps), so CI
  and any operator can re-verify with one command
  (`node docs/site/scripts/check-docs-site.mjs`).
- Hosted deployment is a deployment-layer fact (copy `dist/`), tracked as
  the Phase-2 website follow-up — not a further source-of-truth decision.
