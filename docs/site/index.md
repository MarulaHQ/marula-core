# Meridian documentation

![Meridian logo lockup](../../meridian-ci/logo/lockup-navy@2x.png)

Meridian is an open-source PSA (professional services automation) core for
MSPs: tickets, clients, billing, scheduling, docs, agents. You can self-host it, extend it,
and leave with the whole system.

> "The core is open source. Host it yourself, fork it, break it, rebuild it.
> You can leave with your whole system on any given day and you can extend it
> any which way and make it entirely your own."
> — [MANIFESTO.md](../../MANIFESTO.md), quoted verbatim

These docs follow [Diátaxis](https://diataxis.fr/): each page lives in exactly
one quadrant, and the quadrants stay in their lanes.

| Quadrant | Answers | Start here |
|---|---|---|
| [Tutorials](tutorials/index.md) | "Take me through it, step by step" | [Your first ticket](tutorials/first-ticket.md) |
| [How-to guides](how-to/index.md) | "How do I do this specific thing?" | [Upgrade to the latest release](how-to/upgrade.md) |
| [Reference](reference/index.md) | "What exactly is it?" | [API reference](reference/api.md) |
| [Explanation](explanation/index.md) | "Why is it this way?" | [Why we're building Meridian](explanation/why-meridian.md) |

## Where the rest lives

- Architecture decision records: [`docs/adr/`](../adr/)
- Instant-first interaction doctrine: [`docs/instant-first.md`](../instant-first.md)
- Upgrade & compatibility policy: [`docs/upgrade-compatibility-policy.md`](../upgrade-compatibility-policy.md)
- Operator runbooks: canonical scaffold lives at `deploy/runbooks/`
  (`deploy/runbooks/README.md`); authored runbook content lives in the site
  under [reference/](reference/index.md) — see the
  [coverage audit](DOCS-COVERAGE.md)
- Docs coverage audit & IA plan: [`DOCS-COVERAGE.md`](DOCS-COVERAGE.md)
- Public waitlist/marketing site: the separate `website/` package — product
  positioning is quoted there and here from `MANIFESTO.md` verbatim, never
  re-synthesized.
- What shipped and what is coming: [CHANGELOG.md](../../CHANGELOG.md) and
  [ROADMAP.md](../../ROADMAP.md) at the repository root — the website's
  [changelog](https://meridian.dev/changelog) (with an RSS feed) and
  [roadmap](https://meridian.dev/roadmap) pages build from these files.
