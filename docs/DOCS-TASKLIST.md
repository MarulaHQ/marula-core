# Marula Core — Public docs tasklist

> **Status:** Active · **Created:** 2026-10-26
> **Scope:** the public Markdown corpus under `docs/site/` and its path-level licence. The website platform is tracked in `marula-hq/docs/BUILD-TASKLIST.md` Wave 6.
> **Binding plan:** `docs/planning/44-public-docs-site.md` in the org home · **ADR:** [`adr/0025-docs-content-license.md`](adr/0025-docs-content-license.md)

## Lane C-DOCS-1 — Licence and publishing boundary

- [x] **C-DOCS-1.1** Add `docs/site/LICENSE` with the full CC BY 4.0 legal code.
- [x] **C-DOCS-1.2** Add `docs/site/NOTICE` with copyright, licence, source, trademark and code-sample notices.
- [x] **C-DOCS-1.3** Update root `README.md`: Core AGPL-3.0, SDK/plugins Apache-2.0, docs CC BY 4.0.
- [x] **C-DOCS-1.4** Update `CONTRIBUTING.md`: docs contributions to `docs/site/` are submitted under CC BY 4.0; the DCO wording covers both licences.
- [x] **C-DOCS-1.5** Add the licence section to `docs/site/README.md`.
- [ ] **C-DOCS-1.6** Audit docs authorship and record the result in ADR 0025. **Accept:** no unresolved contributor whose docs contributions cannot be relicensed.

## Lane C-DOCS-2 — Content contract and navigation

- [x] **C-DOCS-2.1** Add `docs/site/nav.json` with the published groups: Start here, Tutorials, Set up and administer, Tickets/time/field work, Billing and contracts, Client portal, Automation/API/agents, Self-host and operate, Reference, Explanation, Troubleshooting, Get help. **Accept:** every page appears exactly once. *Landed: 97 public pages mapped; `index` is the home; `README`/`DOCS-COVERAGE` excluded.*
- [ ] **C-DOCS-2.2** Add `description` to every page; derive from the first paragraph, then edit to 80–160 characters.
- [ ] **C-DOCS-2.3** Add `tags` and `redirect_from` where useful.
- [ ] **C-DOCS-2.4** Add GFM alerts to install, upgrade, backup, restore, deletion, permissions, migration and billing-correction pages.
- [ ] **C-DOCS-2.5** Add code fence titles and line highlights to install, upgrade and API examples.
- [ ] **C-DOCS-2.6** Capture the top 10 workflows in light and dark; store under `docs/site/assets/`; add alt text and `*Figure: …*` captions.
- [ ] **C-DOCS-2.7** Keep `**Audience:**` on every task page; the website parses it into manifest metadata.

## Lane C-DOCS-3 — API reference and validators

- [ ] **C-DOCS-3.1** Split `docs/site/scripts/generate-api-reference.mjs` into `reference/api/index.md`, `reference/api/<tag>.md`, `reference/api/schemas.md` and `reference/api/manifest.json`.
- [ ] **C-DOCS-3.2** Update `check-docs-site.mjs` for anchors, frontmatter, generated-page freshness and nav coverage.
- [ ] **C-DOCS-3.3** Update `check-docs-ia.mjs` for `nav.json` coverage and the audience vocabulary.
- [ ] **C-DOCS-3.4** Add `docs/site/scripts/check-docs-license.mjs` (or fold into the site checker): `LICENSE` and `NOTICE` exist; the README licence section names CC BY 4.0.
- [ ] **C-DOCS-3.5** Run both checkers in CI on every docs PR. **Accept:** a broken anchor, missing description, orphan page or stale generated page fails.

## Lane C-DOCS-4 — Release and in-app help

- [ ] **C-DOCS-4.1** Update `docs/site/help-manifest.json` `docsBase` to the versioned URL at release (`https://getmarula.com/docs/psa/v<version>`).
- [ ] **C-DOCS-4.2** Add a release check that every help-manifest path resolves to a docs route.
- [ ] **C-DOCS-4.3** Add `.github/workflows/docs-release.yml`: on `v*`, run docs validators and dispatch `docs-sync` to `marula-hq/website` with `product=psa`, `version`, `sha`.
- [ ] **C-DOCS-4.4** Add a `docs` job to the main CI workflow: `node docs/site/scripts/check-docs-site.mjs` and `check-docs-ia.mjs`.

**Lane gate:** the corpus is licensed, navigable, validated, versioned in the website lock, and dispatched on release.

## Dependencies

| Lane | Depends on | Notes |
|---|---|---|
| C-DOCS-1 | founder licence decision (done) | Blocks the website footer and agent artifacts |
| C-DOCS-2 | C-DOCS-1 | Can run in parallel with the website platform |
| C-DOCS-3 | C-DOCS-2 | API split is the source for the website API browser |
| C-DOCS-4 | C-DOCS-3, website lock | Release dispatch needs the website workflow |

## Checks

```sh
node docs/site/scripts/check-docs-site.mjs
node docs/site/scripts/check-docs-ia.mjs
```

Both must pass before a docs PR merges. The website build re-checks links and frontmatter from the synced copy.
