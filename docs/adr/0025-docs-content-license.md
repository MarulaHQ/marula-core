# ADR 0025 — Docs content licence and publishing boundary

- Status: accepted
- Date: 2026-10-26
- Related: [ADR 0009](0009-docs-publishing-pipeline.md) (deterministic publisher), [ADR 0024](0024-block-draw-order.md) (latest ADR before this one)
- Supersedes: the website-consumer clause of ADR 0009

## Context

`docs/site/` is the public documentation set for Marula PSA. It has been AGPL-3.0 by inheritance from the repository root, but the root `README.md` licence section names only `crates/` and `apps/` as AGPL and the SDK/plugins as Apache-2.0. The docs have no explicit licence. That ambiguity is a real risk now that the docs are about to render on the proprietary marketing site.

The docs are content, not program code. Creative Commons recommends against CC licences for software, but explicitly allows them for software documentation. The major public docs projects split the two:

- Kubernetes, GitHub Docs, Microsoft Learn, OpenTelemetry and Cloudflare license documentation under **CC BY 4.0**, with code under Apache-2.0 or MIT.
- GitLab licenses only `doc/` under **CC BY-SA 4.0**, with the rest MIT.
- MDN uses **CC BY-SA** for prose and **CC0** for code samples.
- Docker, Rust, Python and Django keep docs under the project’s software licence.

The AGPL’s network clause is the wrong instrument for prose. It also creates a boundary question when the docs render inside a proprietary website.

## Decision

**`docs/site/` is licensed under Creative Commons Attribution 4.0 International (CC BY 4.0). The rest of the repository keeps its current licences: Core code AGPL-3.0, SDK/plugins Apache-2.0.**

- Add `docs/site/LICENSE` with the full CC BY 4.0 legal code.
- Add `docs/site/NOTICE` with copyright, source, trademark and code-sample notices.
- Update `README.md` and `CONTRIBUTING.md` so the docs licence is explicit and future docs contributions are submitted under CC BY 4.0.
- The website may render, style, index and distribute the docs, including commercially, with attribution. The website’s code and visual layer remain proprietary.
- Code examples copied from AGPL code remain AGPL-3.0-or-later and are marked as such. CC BY 4.0 does not override the licence on the code.
- The Marula name and logo remain trademarks. CC BY 4.0 does not grant trademark rights.
- The deterministic publisher (`docs/site/scripts/publish.mjs`) remains the GitHub/offline path. Its “website consumes `dist/` wholesale” clause is superseded: the website consumes the Markdown sources through its own renderer.

## Why CC BY 4.0 and not CC BY-SA 4.0

CC BY-SA 4.0 is the docs-copyleft option (GitLab, MDN). It would require adaptations of the docs to stay under CC BY-SA or a compatible licence. It does not force the website’s source code open, and format/medium changes are not adaptations under CC 4.0. That is viable, but it adds a share-alike analysis around translations, edits and the visual layer for no product gain here. The source is already public on GitHub; the collaboration point is the repository, not a downstream mirror. CC BY 4.0 keeps attribution and removes that compliance surface.

CC BY 4.0 is also the pattern used by the projects Marula is most often compared with: Kubernetes, GitHub, Microsoft, OpenTelemetry, Cloudflare.

## Consequences

- The docs are reusable, mirrorable and translatable with attribution. A competitor may republish them; that is acceptable for documentation and is the cost of the open-source posture.
- The website must render a visible attribution footer on every docs page and carry the licence into `llms.txt`, `manifest.json`, Markdown twins, JSON-LD and MCP responses.
- The docs can be edited without touching the AGPL code licence, and the website can style them without a copyleft question.
- Anyone contributing docs after this ADR submits them under CC BY 4.0. Existing docs contributions are founder and agent lanes; the DCO audit is recorded in the public docs plan.
- The root `LICENSE` still governs everything not explicitly carved out. `docs/site/LICENSE` is the path-level override.

## Rejected alternatives

**Keep docs under AGPL-3.0.** Consistent with the code, but the network clause is a poor fit for prose and creates a boundary question with the proprietary website. It also leaves the current README ambiguity in place.

**CC BY-SA 4.0.** The docs-copyleft option. Rejected for Marula because it adds a share-alike analysis without a product benefit; the source is already public.

**CC BY-ND 4.0.** Blocks translations, edits and reorganisations. Docs need all three.

**CC BY-NC 4.0.** Bans commercial use and makes the marketing site’s use ambiguous. Wrong for an open-source project.

**CC0 1.0.** Waives attribution. Fine for code samples, not for prose where credit matters.

**A separate AGPL docs app.** Clean licence boundary, but a second app, second header, second theme and second deploy. CC BY 4.0 removes the need for that boundary.

## References

- Public docs plan: `docs/planning/44-public-docs-site.md` in the org home
- HQ ADR 0035: `marula-hq/docs/adr/0035-public-docs-site.md`
- [Creative Commons BY 4.0](https://creativecommons.org/licenses/by/4.0/)
- [Creative Commons FAQ: CC and software](https://creativecommons.org/faq/#can-i-apply-a-creative-commons-license-to-software)
- [Kubernetes website licence](https://github.com/kubernetes/website/blob/main/LICENSE)
- [GitHub Docs licence](https://github.com/github/docs)
- [GitLab docs licence](https://gitlab.com/gitlab-org/gitlab/-/blob/master/LICENSE)
- [MDN content licence](https://github.com/mdn/content/blob/main/LICENSE.md)
