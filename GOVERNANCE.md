# Governance

Marula Core is an open-source project. This document says who decides what, and how
that changes as the project grows. It is deliberately short and deliberately honest
about the starting point.

## Current model: single maintainer, foundation-shaped

The project is currently maintained by one person, who holds final decision authority
(the "benevolent dictator" model that most young open-source projects start with).
That is a statement of fact, not a permanent design. The structure below exists so the
project can move to shared governance without a rewrite of its rules, only a change in
who holds the roles.

### Roles

| Role | Can do | How you get it |
|---|---|---|
| **Contributor** | Open issues and PRs. Every PR needs DCO sign-off. | Show up |
| **Reviewer** | Review and approve PRs in an area | Named in `MAINTAINERS.md` by a maintainer |
| **Maintainer** | Merge, release, triage security, appoint reviewers | Invited by a maintainer after sustained, high-quality contribution |
| **BDFL** | Final say on disputes, governance changes, trademark | Currently the project founder |

## How decisions are made

1. **Small changes** — a PR with DCO sign-off, reviewed and merged.
2. **Architecture decisions** — an ADR in `docs/adr/`, merged with the change it
   describes. An ADR records the decision *and* what was rejected, so the next person
   inherits the reasoning rather than re-litigating it.
3. **Contentious changes** — discussion on the issue, then a maintainer decides. The
   decision and its rationale go in the issue or an ADR; "because the maintainer said
   so" is not a good enough record.
4. **Governance changes** — this file changes by PR, with the rationale in the PR.

## What this project promises contributors

- **No CLA.** Contributions are accepted under the DCO, which certifies you have the
  right to submit the code under the project's licence, without assigning copyright.
- **The licence will not change.** The core is AGPL-3.0 and stays AGPL-3.0. The SDK and
  plugin surfaces are Apache-2.0. A relicensing would need the consent of every
  contributor whose code is affected.
- **No rug-pull on features.** The open core is a complete product, not a demo of one.
  Features do not move from core into a paid tier after you depend on them.

## What this governance does not cover

- **The Marula name and marks** — see `TRADEMARK.md`. Code licences grant no trademark
  rights.
- **Hosted and commercial services** — those are operated by the project's commercial
  entity under its own terms. They are not governed by this document, and they are not
  required to run Marula Core.

## Roadmap

`ROADMAP.md` is the public statement of what ships next. It records *what* and *why*,
never internal wave, lane or staffing detail.
