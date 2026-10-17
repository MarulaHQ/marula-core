# E2E suite rules (p37-env-orchestration)

One page of rules every Playwright flow author follows. Enforced by review,
the flake gate only catches symptoms.

## Boot the stack through ONE script — never hand-roll

`scripts/e2e-stack.sh` allocates a per-run DB name, random high ports, and
run-id-keyed container names, boots core-api + realtime + a scratch Valkey,
seeds the demo org deterministically (below), and writes `e2e/.e2e-stack.env`.
Playwright's config and `scripts/e2e-build-web.sh` read that file — do not set
`NEXT_PUBLIC_API_URL`/ports/scents by hand or you reintroduce boot drift
(report class 2/3). Teardown is `scripts/e2e-stack.sh down [RUN_ID]` and only
touches resources carrying that run id — never a parallel lane's DB.

`up` returns as soon as core-api answers its health check: core-api and realtime
detach with their own stdio (`</dev/null`, logs under `.kilo/e2e-logs/<run>/`),
so piping the boot anywhere can never block. `up` narrates every stage with its
elapsed time and ends by printing the tail of both service logs; for a live view
run it again with `E2E_FOLLOW=1`, which streams those logs instead of returning
(Ctrl-C stops following, the stack keeps running). If a boot fails, read the log
— do not wait on a call that will not return. The run registry and those logs live
under the shared root (the main checkout), so `scripts/dev-status.sh` lists every
lane's stack no matter where it is invoked.

## Deterministic demo state is mechanical, not tribal

The stack script reseeds (`seed-demo --force`), truncates `auth_lockouts` /
`auth_attempt_log`, and deletes per-user `onboarding.*`/`tours.*` rows before
bootstrap. Every suite run starts from the identical demo org. Nothing a
previous lane left behind is inherited.

## Order-independence rule for flow authors

Flows must be green in ANY order, in BOTH projects (light before dark), and on
a dirty demo org:

1. **Login fixtures retry through the durable auth gate.** Never call the
   login endpoint once and throw on 401: `01-auth` arms the Wave 19 lockout
   (30s → doubling), which would 401 every later flow. Use `apiLogin` /
   `loginContext` from `e2e/helpers` (both retry with backoff through the
   gate). Anything that logs in through UI forms belongs in `01-auth` only.
2. **Stateful flows (onboarding, tours, merge queue) stage their own scratch
   state and restore what they armed.** Never assume pristine demo rows:
   - onboarding: re-arm via `/onboarding/reset/org` / `/reset/user` at test
     start (flow 12 does this)
   - tours: reset `tours.disabled` / `tours.completed` to their defaults at
     test start, and clear suppression you armed at test end (flow 13)
   - merge queue: stage candidates via feed-upsert with per-run unique ids
     (`Date.now()` suffix + random MACs), never reuse demo rows (flow 11).
3. **If a test asserts a row-level consequence, it must also be able to
   CREATE that row.** A test that only reads whatever the environment happens
   to contain is a flake in a shared org.

## Evidence writes stay inside the lane that writes them

A run writes evidence into the running lane's own directory, never into another
lane's committed record. `e2e/helpers/evidence.ts` resolves it: `E2E_EVIDENCE_DIR`
if set, else `design/qa-<branch>`, else `design/qa-<run id>`, else
`design/qa-<short sha>`. The branch name is the directory name, so the lane that
owns `design/qa-p34-plugin-surface-closeout` still lands there. Every other lane gets
its own directory. Specs and helpers must use `evidenceDir()`/`evidencePath()`
instead of a literal `design/qa-*` path — one sweep on 2026-10-16 rewrote seven
tracked files belonging to three other lanes, and a lane gating with `git add -A`
would have committed that over their records. `node scripts/check-evidence-writes.mjs`
fails the gate when a frozen path comes back into the suite, and `pnpm
evidence:check` runs it.

## The shared cargo target dir has a provenance guard

Every checkout compiles into `.kilo/target-shared` (disk guardrails DX6), and
cargo records dep-info paths relative to the workspace root — so an artifact a
peer worktree built from different sources can pass this tree's freshness check
and get linked, producing a phantom `cannot find value …` or a
`built for newer 'macOS'` linker error charged to an innocent crate.
`scripts/lane-target-guard.py check --profile release --fix` closes it: it resets
what the verdict flagged for the profile being built — the workspace members
when a peer tree wrote them, the whole profile when the flagged writes landed
outside the member set (a bare `cargo test` writes third-party dev units too) —
and `mark` claims the directory after a successful build.
`scripts/e2e-stack.sh up` runs both around its release builds.
It also refuses to reset while a build holds the profile's own cargo lock
(`<target-dir>/<profile>/.cargo-lock`), so two sessions on one machine cannot
clean each other's artifacts. A refusal cleans nothing and exits 3: re-run once
that build finishes rather than reaching for `MARULA_LANE_GUARD=0`, which also
skips the `mark` steps. `--force` is the explicit override.
`pnpm target:check` reports the state by hand, and `scripts/repro-shared-target-poison.sh`
reproduces the failure and the fix on a throwaway workspace. A lane that resets
the directory by hand must pass `--release` — `cargo clean -p …` without it
leaves the poisoned release artifact in place.

A boot that fails before the registry entry is written now removes its own
container pair, so a failed `up` no longer leaves an unregistered stack behind
(`E2E_KEEP=1` opts out). It only ever touches the run id it created.
