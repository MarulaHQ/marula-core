---
description: "Marula is an open-source PSA (professional services automation) platform for IT service teams and MSPs."
---

# Marula documentation

![Marula logo lockup](../../.github/assets/logo-lockup-ink@2x.png)

Marula is an open-source PSA (professional services automation) platform for IT service teams and MSPs. It runs your tickets, timers, client records, dispatch schedule, project billing, and client portal in one place on a database you own and control.

> "The core is open source. Host it yourself, fork it, break it, rebuild it.
> You can leave with your whole system on any given day and you can extend it
> any which way and make it entirely your own."
> — [MANIFESTO.md](../../MANIFESTO.md), quoted verbatim

## Start by role

- **Technicians and service desk staff:** Start with [Your first ticket](tutorials/first-ticket.md), then check [Work a ticket](how-to/staff-work-a-ticket.md), [Run a field service day](how-to/technician-service-day.md), and [Close your day with the day pack](how-to/staff-day-pack.md).
- **Dispatchers and service leads:** Jump to [Dispatch from the board](how-to/staff-dispatch.md), [Set up SLA policies and business-hours calendars](how-to/admin-sla-calendars.md), and [Build dashboards and scheduled reports](how-to/admin-dashboards-reports.md).
- **Office managers and bookkeepers:** Walk through [Your first invoice](tutorials/first-invoice.md), [Set up contract billing models](how-to/admin-contract-types.md), [Verify a billing period before invoicing](how-to/admin-billing-verification.md), and [Portal onboarding](tutorials/portal-onboarding.md).
- **Systems administrators and developers:** See [Self-host local stack reference](reference/self-host-local-stack.md), [Upgrade to the latest release](how-to/upgrade.md), and [Call the API from TypeScript or curl](how-to/call-the-api.md).

## Or start from the task

[Start here](start/index.md) is the same idea, done properly: one page per
reader, each listing tasks in the words people use and linking the page that
answers them.

| You are | Start at |
|---|---|
| Owner — contracts, margin, month close, access | [Owner tasks](start/owners.md) |
| Admin — people, boards, SLA, email, portal, automations | [Admin tasks](start/admins.md) |
| Technician — tickets, service days, stock, billable time | [Technician tasks](start/technicians.md) |
| Operator — stand-up, backup, upgrades, monitoring | [Operator tasks](start/operators.md) |
| Agent author — API, read model, grants, plugins | [Agent and automation tasks](start/agents.md) |

Two pages cut across all of them: [Troubleshooting](troubleshooting.md) starts
from a symptom instead of a feature, and the [Glossary](reference/glossary.md)
defines the vocabulary the rest of the site assumes.

## Browse by section

These docs follow [Diátaxis](https://diataxis.fr/). Each page belongs to one of four sections so step-by-step lessons, quick task checklists, technical specs, and design background stay easy to find.

| Section | Answers | Start here |
|---|---|---|
| [Tutorials](tutorials/index.md) | "Take me through a full workflow, step by step" | [Your first ticket](tutorials/first-ticket.md) |
| [How-to guides](how-to/index.md) | "How do I get this specific job done?" | [How-to index by role](how-to/index.md) |
| [Reference](reference/index.md) | "What are the exact schemas and endpoints?" | [API reference](reference/api.md) |
| [Explanation](explanation/index.md) | "Why does Marula work this way?" | [Why we're building Marula](explanation/why-marula.md) |

## Where the rest lives

- Architecture decision records: [`docs/adr/`](../adr/)
- Instant-first interaction doctrine: [`docs/instant-first.md`](../instant-first.md)
- Upgrade and compatibility policy: [`docs/upgrade-compatibility-policy.md`](../upgrade-compatibility-policy.md)
- Engineering documentation (architecture, operations internals, specs): [`docs/README.md`](../README.md) — the entry point for people building and running Marula, kept out of this site on purpose
- Operator runbooks: canonical scaffold lives at `deploy/runbooks/` (`deploy/runbooks/README.md`), while authored runbook content lives in the site under [reference/](reference/index.md) (see the [coverage audit](DOCS-COVERAGE.md))
- Docs coverage audit and IA plan: [`DOCS-COVERAGE.md`](DOCS-COVERAGE.md)
- Public waitlist and marketing site: the separate site package, which lives outside this repository. Product positioning is quoted there and here from `MANIFESTO.md` verbatim, never re-synthesized.
