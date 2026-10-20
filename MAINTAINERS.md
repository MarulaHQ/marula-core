# Maintainers

The people responsible for this repository. See `GOVERNANCE.md` for what the roles
mean.

## Current

| Person | Role | Areas | Contact |
|---|---|---|---|
| `@tlengoasa` | BDFL / maintainer | everything | `oss@getmarula.com` |

Solo at present. Reviewers and maintainers are added here as the project grows — the
roles exist now so that adding someone is a one-line change, not a governance debate.

## What a maintainer here is expected to do

- **Review PRs** against the rules in `CONTRIBUTING.md`: conventional commits, DCO
  sign-off, no `unwrap()` outside tests, clippy and `tsc` clean, RLS and migration
  rules respected.
- **Release** per `RELEASING.md`, signing tags from the project identity — not a
  personal key.
- **Triage security** reported through `SECURITY.md` inside its stated windows, and
  keep the fix and the advisory in step.
- **Keep the public record honest.** The public repo carries the product, the ADRs and
  the roadmap. Internal planning, evidence and commercial material stay out of it; a
  maintainer is the person who notices when that line is about to be crossed.

## Adding a maintainer

A maintainer invites someone after sustained contribution, usually with several merged
PRs across more than one area and at least one review cycle where their judgement was
visible. Add them here and to `.github/CODEOWNERS` in the same PR, with the areas they
own. There is no vote while there is one maintainer; once there are three, adding a
maintainer needs a second maintainer's agreement.

## Removing a maintainer

Inactive for six months, or a breach of the code of conduct, moves someone to
*emeritus* here rather than deleting the record.
