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
