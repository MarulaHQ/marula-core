# Workflows

What runs, when, and why. Wave 52 lane `p52-ci-gates-restore` split the old
`.disabled` set in two: a small per-push set that decides whether a change may
merge, and a nightly set for the checks that need a long clock, a simulator, a
Docker daemon or a signature database.

| File | Trigger | Jobs | State |
| --- | --- | --- | --- |
| `ci.yml` | pull request, push to `main` | `boundary`, `rust`, `frontend`, `ui-gates`, `token-conformance`, `migrations`, `openapi-pins`, `container-lint`, `rls-pool-reads`, `rls-coverage`, `rust-contract` | **enabled 2026-10-31** — the 2026-10-29 pause is over by user direction, so the merge-blocking set runs again. Renaming the file is still the switch; adding `.disabled` parks it without deleting it. While it was off, nothing on a pull request was red and the pages that cite it as enforcement (`docs/perf.md`, `docs/theme.md`, `docs/upgrade-compatibility-policy.md`, `docs/tech-stack.md`) described a gate that was not running |
| `nightly.yml.disabled` | schedule 05:30 UTC, manual dispatch | `perf-smoke`, `perf-load-budget`, `perf-scale-100k`, `e2e`, `api-live-conformance`, `upgrade-matrix`, `domain-coverage`, `container-security`, `clamd-live` | parked — the heavy scheduled set, by user direction 2026-10-21: minutes, not doubt |
| `native.yml.disabled` | schedule 05:45 UTC, manual dispatch | `native-checks`, `maestro-ios` | parked — Maestro drives simulators, so it is a minutes job like the rest of the schedule |
| `changesets.yml` | push to `main` | `version` | enabled |
| `release-installer.yml` | `v*.*.*` tag, pull request touching the installer, manual dispatch | `version`, `scripts`, `build` (one leg per platform), `offline` (Linux legs), `publish` | enabled — builds, signs and checksums the release artifacts onto the run; attaching them to a release page is the maintainer's step |
| `dco.yml` | pull request | `sign-off` | enabled |
| `preview.yml.disabled` | none | `teardown`, `preview` | disabled — needs a host, DNS, TLS and deploy credentials that no workflow can create |
| `release.yml.disabled` | none | `images`, `scan`, `chart` | disabled — pushes artifacts to a registry and a release page, which is the maintainer's publishing decision |

`ga-gates.yml` no longer exists. Its five jobs were folded into this split:
`api-contract-tests` and `u9-customization-crosscheck` into `ci.yml`,
`perf-smoke` into `nightly.yml`, `domain-coverage` and the trivy config scan
into `nightly.yml`, and the three hadolint steps into `ci.yml` as
`container-lint`. Folding beat renaming: its `on:` block had been commented out,
so it was a job list with no owner.

## What decides the side of the split

A job runs per push when it finishes in minutes, needs nothing a runner does not
already have, and fails the same way on a developer's machine. A red one is a
defect in the change.

A job runs nightly when it needs a saturated load loop, a 50k-row seed, a macOS
simulator, a Docker daemon inside the job, or a ClamAV signature database. Those
checks still catch regressions, just a day later, and the trade is deliberate: a
scanner or a capacity number should not hold a merge.

## Where the slow gates went

`ci.yml` used to carry `e2e`, `perf-smoke`, `perf-load` and `upgrade-matrix`
inline. All four are nightly now. The `api-conformance` job that booted a server
split in two: the derived-spec and SDK-snapshot comparison stayed per push (it
reads a pinned fixture), and the comparison against a running server became
`api-live-conformance`.

## A green workflow is not the same as a working one

These files were written and validated on a laptop. A workflow's behaviour is
proven by a run, and a run needs a push. Nothing in this repository's history
proves that `ci.yml` passes: the per-push commands were all executed locally
against the same commit, which is evidence about the commands and not about the
runner. The first push answers it.

`release-installer.yml` is in the same position, and further from a run: its
signing legs read secrets no pull request can see, so the signed path is only
exercised by a tag. Until a tag is cut, the unsigned path and the scripts are
what has been checked, and `docs/release.md` says which commands that was.
