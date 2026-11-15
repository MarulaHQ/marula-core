# Workflows

What runs, when, and why. Wave 52 lane `p52-ci-gates-restore` split the old
`.disabled` set in two: a small per-push set that decides whether a change may
merge, and a nightly set for the checks that need a long clock, a simulator, a
Docker daemon or a signature database.

| File | Trigger | Jobs | State |
| --- | --- | --- | --- |
| `ci.yml` | pull request, push to `main` | `boundary`, `rust`, `frontend`, `ui-gates`, `token-conformance`, `migrations`, `upgrade-scope`, `upgrade-matrix`, `openapi-pins`, `container-lint`, `container-security`, `rls-pool-reads`, `rls-coverage`, `rust-contract` | **enabled 2026-10-31** — the 2026-10-29 pause is over by user direction, so the merge-blocking set runs again. Renaming the file is still the switch; adding `.disabled` parks it without deleting it. While it was off, nothing on a pull request was red and the pages that cite it as enforcement (`docs/perf.md`, `docs/theme.md`, `docs/upgrade-compatibility-policy.md`, `docs/tech-stack.md`) described a gate that was not running |
| `nightly.yml.disabled` | schedule 05:30 UTC, manual dispatch | `perf-smoke`, `perf-load-budget`, `perf-scale-100k`, `domain-coverage`, `clamd-live` | parked — the heavy scheduled set, by user direction 2026-10-21: minutes, not doubt |
| `e2e.yml` | schedule 05:30 UTC, pull request labelled `run-e2e`, manual dispatch | `e2e` | **enabled 2026-11-15** — the Playwright suite owns its triggers. It ran only on the night schedule while it lived in `nightly.yml`, which meant a flow regression waited a day and a reviewer had no way to ask for the suite on the pull request in front of them. The label is the gate: an unlabelled pull request pays nothing |
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
simulator, a ClamAV signature database, or a long enough clock that a pull
request cannot afford it. Those checks still catch regressions, just a day
later, and the trade is deliberate: a scanner or a capacity number should not
hold a merge.

The E2E suite breaks that pattern and is honest about it: it boots Docker inside
the job and it is the most expensive thing here, but it also catches things no
per-push gate can. It lives in its own file so the schedule and the `run-e2e`
label are its only triggers.

## Where the slow gates went

`ci.yml` used to carry `e2e`, `perf-smoke`, `perf-load` and `upgrade-matrix`
inline. All four are nightly now, except `e2e`, which has its own file. The
`api-conformance` job that booted a server split in two: the derived-spec and
SDK-snapshot comparison stayed per push (it reads a pinned fixture), and the
comparison against a running server became `api-live-conformance`.

Three of those nightly jobs came back to `ci.yml` on 2026-11-15, because the
argument for the split is cost and none of them was expensive for the reason the
split exists:

- **`api-live-conformance`** is now the last two steps of `rust-contract`, not a
  job. The test binary and the database are already there; a separate job would
  rebuild both to ask the same question. It is the third golden layer, and the
  only one that can see a route served without being in the derived document.
- **`container-security`** (trivy config over `deploy/`) sits beside
  `container-lint`. The policy bundle costs about a minute, and a Dockerfile
  edited in a pull request deserves the answer a Dockerfile gets. Trivy does not
  read compose files; pointed at one it reports `Detected config files num=0`.
  So `scripts/check-compose-hardening.py` covers those, and its `--self-test` is
  what stops it becoming a check that cannot fail.
- **`upgrade-matrix`** runs on every pull request that touches
  `crates/core-db/{migrations,src}/`, `crates/marula-core-cli/src/` or
  `fixtures/upgrades/`, and on every push to `main`. `upgrade-scope` computes
  that from a two-dot diff against the base branch; a push to `main` has no base
  and always runs, because a merge is exactly where the forward path from a
  shipped release can break.

What stays nightly needs a long clock rather than a changed diff: the perf
budgets and the 100k scale seed, the instrumented coverage rebuild, and clamd
waiting on a signature database.

## Asking for the E2E suite on a pull request

Add the `run-e2e` label. The job starts on the `labeled` event, so the same
label added twice does nothing and an unlabelled pull request is unchanged. It
is worth spending the minutes when a change touches a flow under `e2e/flows/`,
a theme pack, the router, or anything that only misbehaves with a real stack and
a real browser behind it.

The suite builds core-api and the worker in release and a Next standalone
bundle, so it is the most expensive thing this repository runs. Two facts are
worth knowing before reading a red run: `MARULA_EMBEDDING_TIER: minimal` is
mandatory (a 2-core runner is below the `standard` tier's floor and core-api
refuses to boot rather than under-provision), and the flake gate is strict on
first runs — a test that needed a retry beyond the budget fails the job rather
than absorbing it.

## The runner image is pinned on purpose

`rust`, `rust-tests-rest` and `e2e` run on `ubuntu-24.04`, not
`ubuntu-latest`. The label moved to Ubuntu 26.04 on 2026-10-19 and was still
serving mixed builds of it: one run landed on `ubuntu26/20261111.82`, which came
up with no Ubuntu archive sources, so `apt-get update` fetched only the
Microsoft repository and every distro package failed as "Unable to locate". The
same step passed eighty minutes earlier on `ubuntu26/20261111.196`.

Those three jobs are the only ones that install distro packages — GTK and
WebKit headers for the Tauri member, and Playwright's browser dependencies.
Pinning the distro is cheaper than discovering which image build a run landed
on. When the 26.04 fleet stops disagreeing with itself, move them back and
delete this section.

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
