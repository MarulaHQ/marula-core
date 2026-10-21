# Perf program — budgets, harness, baselines

> Established by Wave 13 lane 8 `p13-perf-baseline` (test-infra only: no
> product code, no migrations). This is the Phase-4 perf program's anchor
> document: the budgets, the tooling that enforces them, and the measured
> baselines they were set against.

## 1. Budgets

| Budget | Target | Enforced by |
|---|---|---|
| Ticket list (primary reads) | **p95 < 150ms @ 50k tickets/org** | `perf-smoke` CI job (`.github/workflows/ci.yml`) via `measure::assert_budget` |
| Deep pagination, keyset (`cursor`, depth 40k) | same primary budget — keyset IS budgeted since Wave 15 lane 3 | same smoke (the `deep_page_keyset40k` mix) |
| Search deep pagination, keyset (`cursor`, depth 40k) | same primary budget — Wave 52 lane P5 | same smoke (the `search_keyset40k` mix) |
| Search deep pagination, legacy `OFFSET 40000` | 400ms regression tripwire (not a product budget) | same smoke (the `search_deep_offset40k` mix) |
| Deep pagination, legacy `OFFSET 40000` | 400ms regression tripwire (not a product budget) | same smoke. See the finding below |
| HTTP capacity (saturated closed-loop) | p95 ≤ 150ms @ 50k, concurrency 8, keyset mix | `Nightly perf` job (`.github/workflows/nightly.yml.disabled`) via `perf-harness load --budget-ms` |
| Scale harness | 100k tickets/org, reproducible | `perf-harness seed/load` (nightly: measurement + 600ms tripwire, not gated) |

The primary budget covers every standard list shape: unfiltered page 1,
priority filter, assignee scope, client scope, and (since the keyset
cursor shipped) deep-page walks in the product shape. Legacy OFFSET
pagination stays measured and tracked with a tripwire, not
product-budgeted (finding F2: offset has no enforceable product budget
until clients migrate. The cursor is the budgeted path).

## 2. Tooling (`crates/perf-harness`)

Test-infra crate, AGPL, not part of the API surface. Four pieces:

- **Seed generator** (`perf-harness seed`): deletes + re-inserts the perf
  org in one transaction. Every measured entity gets a deterministic id
  (`md5('<tag>')::uuid`, computed in-database) so the same scale always
  reproduces the same dataset. Set-based `generate_series` inserts: 100k
  tickets + 150k custom-field values seed in ~12s. Distribution: priority
  10/20/60/10% (p1–p4), category 40/35/20/5% (closed/open/in_progress/
  waiting), 70% assigned round-robin, created_at spread over ~2 years,
  three ticket custom fields with values on ~50% of tickets.
- **Measurement** (`perf-harness measure`): runs the EXACT server-side
  path of `GET /api/tickets`
  (`tx.begin → set_org_and_user →
  core_db::tickets::list_tickets → custom_fields::list_values_many →
  commit`) through the `marula_app` pool under RLS, over five request
  mixes (page 1, priority p1, assignee, client, deep page), 5 warmups +
  40 timed runs, p50/p95/p99/max per mix, plus `EXPLAIN (ANALYZE,
  BUFFERS)` for the page-1 mix. Since Wave 52 lane P5 it also runs the search
  path (`search_page` / `search_page_after`) over three mixes (page 1, the
  legacy deep offset, and the keyset walk at the same depth) against one
  indexed row per seeded ticket. `perf-harness smoke` = seed + measure +
  budget assert (what CI runs).
- **HTTP load runner** (`perf-harness load`): logs the seeded harness user
  in through the real `POST /api/auth/login`, then drives
  `GET /api/tickets` with a deterministic request mix (xorshift-seeded,
  `--seed`, default 42): 60% page-1, 10% p1, 10% assignee, 10% client,
  10% deep page. Closed-loop workers (`--concurrency`, default 8),
  end-to-end latency (bearer auth, RLS tx, serialization, axum stack),
  JSON report, exit 1 over budget.
- **Multi-endpoint session** (`perf-harness journey`, Wave 49 lane R5): the
  session-shaped workload a users-per-instance number needs. One shared
  login, then N virtual users each loop a deterministic mixed session over
  the ticket workspace (board list, open ticket, read thread, my-work, SLA,
  add comment, change status), reads ~80% and writes ~20%, with a
  `--think-ms` pause between steps. Per-endpoint p50/p95/p99/max plus the
  aggregate per-user rate and Little's-law users-per-instance figure. See §7.

### Running it

```sh
# CI perf smoke (also: .github/workflows/ci.yml `perf-smoke` job)
./scripts/perf-smoke.sh            # DATABASE_URL env or throwaway testcontainer

# Local baseline against a scratch DB (use a port outside the lane range)
MARULA_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:54399/marula_perf \
  perf-harness seed --tickets 50000
MARULA_TEST_DATABASE_URL=... perf-harness measure --budget-ms 150

# Scale leg: 100k tickets/org through the HTTP stack
MARULA_TEST_DATABASE_URL=... perf-harness seed --tickets 100000 --clients 1000 --users 25
DATABASE_URL=... BIND_ADDR=127.0.0.1:18080 core-api   # scratch port
perf-harness load --base-url http://127.0.0.1:18080 --duration 30s --concurrency 8 --budget-ms 150
```

The load runner resolves the deep-page keyset anchor by itself: one
untimed offset call at login captures the server's `x-next-cursor`
header, and the mix's 10% deep-page share then rides
`?limit=50&cursor=<that>` (falling back to the legacy offset URL against
a pre-keyset server — the runner stays usable across versions).

## 3. Measured baselines (2026-09-13, lane 8)

Environment: local dev machine (Apple Silicon, macOS), Postgres 17
(pgvector image parity), release build, warm planner/pool cache, seeded
perf org, RLS-scoped `marula_app` traffic. 5 warmups + 40 timed runs
per mix.

### SQL path @ 50k tickets/org — the budget dataset (all PASS)

| Mix | p50 | p95 | p99 | max |
|---|---|---|---|---|
| page1_default | 26.7ms | **29.9ms** | 39.6ms | 39.6ms |
| priority_p1 | 10.9ms | 11.4ms | 11.6ms | 11.6ms |
| assignee_mine | 8.6ms | 8.9ms | 9.0ms | 9.0ms |
| client_scoped | 7.2ms | 8.1ms | 8.3ms | 8.3ms |
| deep_page_offset40k | 124.9ms | 147.6ms | 204.0ms | 204.0ms |

Headroom at 50k: ~5× on the primary mix (29.9ms vs 150ms).

### SQL path @ 100k tickets/org

| Mix | p50 | p95 | max |
|---|---|---|---|
| page1_default | 23.3ms | 24.0ms | 25.3ms |
| deep_page_offset40k | 95.9ms | 108.0ms | 214.2ms |

Primary reads are flat in dataset size (they never scan past the first
page). Only OFFSET pagination grows.

### HTTP end-to-end @ 50k (core-api, 30s load runs)

| Concurrency | Throughput | p50 | p95 | p99 | max |
|---|---|---|---|---|---|
| 1 (no queueing) | 33 rps | 18.4ms | **117.1ms** | 143.9ms | — |
| 8 (saturated, closed-loop) | 138 rps | 34.9ms | **217.9ms** | 336.3ms | 610ms |

### HTTP end-to-end @ 100k tickets/org (30s, concurrency 8)

| Throughput | p50 | p95 | p99 | max |
|---|---|---|---|---|
| 74 rps | 55.0ms | **528.8ms** | 686.1ms | 930.3ms |

### Wave 15 lane 3 (`p15-perf-product`, 2026-09-16) — after keyset pagination

Same environment, same 50k fixture, release build. The keyset mix
(`deep_page_keyset40k`) resumes at depth 40k from a real cursor. The
legacy offset mix is unchanged.

| Mix | p50 | p95 |
|---|---|---|
| page1_default | 29.0ms | 30.9ms |
| deep_page_offset40k (unchanged legacy) | 157.4ms | 173.0ms |
| **deep_page_keyset40k** | **24.2ms** | **25.6ms** |

Keyset deep pages are ~6× cheaper than offset at depth 40k and sit flat
at the page-1 cost — the O(offset) scan is gone. HTTP end-to-end @ 50k
with the keyset deep-page mix in place of the offset leg:

| Concurrency | Throughput | p50 | p95 | p99 | max |
|---|---|---|---|---|---|
| 1 | 42 rps | 29.2ms | **31.2ms** | 36.1ms | 212ms |
| 4 | 170 rps | 29.0ms | **32.2ms** | 36.6ms | 123ms |
| 8 (saturated) | 259 rps | 32.2ms | **49.2ms** | 64.9ms | 184ms |

### Wave 52 lane P5 (`p52-search-keyset`, 2026-10-19) — search after keyset

Same 50k fixture (one indexed row per ticket, `network` in every title so a
deep page is a real deep page), release build, five warmups and forty timed
runs per mix. Search had the ticket list's old shape: `LIMIT/OFFSET` plus a
full-predicate `COUNT(*)`. It now walks a cursor and counts a bounded floor.

| Mix | p50 | p95 |
|---|---|---|
| search_page1 | 35.1ms | 39.8ms |
| search_deep_offset40k (unchanged legacy) | 54.8ms | 57.8ms |
| **search_keyset40k** | **16.6ms** | **23.0ms** |

A deep search page is 2.5× cheaper than offset at the same depth, and it
lands below the first page: the anchor predicate prunes by score before the
sort, so a cursor page orders roughly a quarter of the corpus instead of all
of it. Page 1 is the most expensive search shape at 39.8ms p95, because every
matching row is scored and ordered — that is the cost of relevance ranking,
not pagination.

The exact count is gone with it: `total` is now a floor capped at 1,000 with
`total_capped` saying so, which bounds the count query at 1,001 rows for
`search` instead of scanning the predicate a second time.

In the same run the ticket mixes held their previous numbers (page 1 at
34.4ms p95, legacy offset at 135.1ms, keyset at 26.8ms), which is the
control: this lane changed search only.

## 4. Findings (measure-first output. Product fixes ride later perf lanes)

- **F1 — Budget holds at the read layer.** The server-side ticket-list
  query path is well under 150ms p95 at 50k (≈30ms) and flat at 100k.
  The budget is realistic.
- **F2 — OFFSET pagination was the scaling debt. FIXED by keyset.**
  Deep pages cost O(offset) rows scanned + sorted (measured 148–237ms
  p95 @50k). Wave 15 lane 3 shipped keyset/cursor pagination: the
  opaque `cursor` query param resumes the ranked list
  `(priority rank, due_at NULLS LAST, created_at, id)` strictly after
  the anchor row and pages at page-1 cost (`deep_page_keyset40k`
  25.6ms p95 @50k vs 173.0ms legacy offset in the same environment).
  The offset tripwire stays for regression coverage of legacy clients. The product budget now applies to the keyset mix (enforceable —
  see §5). Legacy behavior is unchanged for callers that don't pass
  `cursor`.
- **F3 — Saturated closed-loop overshoot (RESOLVED twice over).** The
  218ms p95 @50k measured in the baseline was NOT pure queueing: after
  the keyset mix replaced the offset deep-page leg, the same
  saturation point runs at 49.2ms p95 / 259 rps. The remainder is the
  genuinely capacity-shaped cost (queueing + serialization) and it
  holds under the 150ms HTTP budget. See §5 for the capacity decision.
- **F3a — per-request HTTP overhead ≈ 6–8ms** (auth + JSON + stack):
  p50 18.4ms HTTP vs ≈10ms SQL for page 1. Bounded, not urgent.
- **F4 — Plan shape @50k. Candidate index measured, DELIBERATELY NOT
  SHIPPED.** RLS one-time-filter + seed scan + top-N heapsort over the
  org's tickets (EXPLAIN captured in every `measure` run). The
  candidate index (an expression index on
  `(org_id, (CASE priority…), due_at NULLS LAST, created_at, id)`)
  was built on the seeded 50k fixture and measured: it drops the page-1
  execution to **0.66ms** (~45× faster, planner goes straight to an
  ordered index scan, no sort, no seed scan). Decision, per the
  measure-first rule ("only act when page-1 p95 heads toward budget"):
  **not shipped** — page-1 p95 is ~31ms @50k (4–5× headroom) and the
  keyset cursor already removed the deep-page scaling term, so the
  budget pressure that would justify a new index + its write-path tax
  does not exist today. Revisit when page-1 p95 exceeds ~2/3 budget OR
  whenever the SQL sort plan regresses. The full measured recipe is in
  the lane landing record (the private build tracker), and the same index
  can now be tried without building it — `hypopg` and the recipe are in
  [`docs/site/reference/db-observability.md`](site/reference/db-observability.md).

## 5. HTTP capacity decision (Wave 15 lane 3)

The saturated closed-loop point is **concurrency 8** — 8 back-to-back
workers sharing one bearer token, the documented reproducing
configuration (`perf-harness load --seed 42`). Measured after the keyset
fix: p95 49.2ms, 259 rps @50k — **the 150ms product budget holds at
saturation with ~3× headroom**. The pre-keyset 218ms reading was the
offset deep-page leg, not queueing capacity.

**Pool sizing / connection count (explicit; corrected 2026-10-17 by Wave 49 lane R1,
resolved 2026-10-21 by Wave 52 lane P9).**
A default stack, by process, opens:

| Process | Pool | Ceiling | Env knob |
| --- | --- | --- | --- |
| core-api | app | 20 | `MARULA_DB_POOL_MAX` |
| core-api | reporting (RLS role) | 8 | `MARULA_REPORTING_POOL_MAX` |
| core-api | LISTEN (event trail + presence NOTIFY) | 1, direct | `MARULA_LISTEN_DATABASE_URL` |
| worker | app (owner role) | 20 | `MARULA_DB_POOL_MAX` |
| worker | delivery reporting | 5 | `DELIVERY_POOL_MAX` |

That is **54 client connections per instance**, and PgBouncer holds the server
side at **15 per (database, user) pair**: 30 across the app role and the owner
role, however many instances are added. The arithmetic that forced this:
71 per instance meant a second instance asked for 142 against Postgres's default
`max_connections = 100`, so the pool ceiling was the first limit hit, not the
last.

Two of those numbers moved in lane P9 and each has its own reason.
`MARULA_REPORTING_POOL_MAX` was 25, a quarter of the budget spent on reports, a
surface a user opens occasionally; the hot path saturates at concurrency 8, so
the reporting ceiling is 8. The LISTEN connection is dialled directly at
`MARULA_LISTEN_DATABASE_URL` because a subscription cannot survive a connection
the pooler hands to the next client. `docs/reference/connection-pooling.md`
carries the measurements, including the two settings a transaction pooler needs
(`server_reset_query = DISCARD ALL` after every transaction, and the statement
cache off) and the cross-tenant probe that verifies RLS still holds through it.

Every production ceiling is an env knob since lane R1; a present-but-invalid
value (0, non-numeric, or above 1024) refuses boot rather than silently
reverting to the default, because a wrong pool size is a capacity incident and
not a cosmetic one. `MARULA_DB_STATEMENT_CACHE` takes 0 (the default, and what
a pooler requires) through 4096. Acquire timeouts are 10s (app) / 30s
(reporting). Saturation at c8 keeps the queue in the axum worker pool, never in
the pool acquire path.

**Enforcement:** the nightly `Nightly perf` workflow gates the c8 @50k
run at 150ms (`--budget-ms 150`) and surfaces the JSON report as a step
summary + artifact. The 100k scale leg is measured on a 600ms
order-of-magnitude tripwire (capacity observation, not budget).

## 6. CI contract

- `perf-smoke` job runs `scripts/perf-smoke.sh` on every push/PR: seeds
  50k, measures (page-1, priority, assignee, client, keyset deep-page,
  legacy offset mixes), asserts. The SQL gate covers every shape the
  budget covers — including the keyset deep page since the cursor
  shipped. The test is env-gated (`MARULA_PERF_SMOKE=1`) so the plain
  `cargo test --workspace` sweep skips it (fast sweeps).
- `Nightly perf` (`.github/workflows/nightly.yml.disabled`, cron 05:30 UTC
  + workflow_dispatch): gates the saturated HTTP capacity point
  (`perf-harness load --concurrency 8 --budget-ms 150` @50k through the
  release-built core-api, connected as `marula_app` so RLS is in the
  path) and measures the 100k scale leg on a 600ms tripwire (not gated).
  Results surface as a step summary + `nightly-load-report` /
  `nightly-scale-report` artifacts. A red nightly is investigated in the
  next session, never shrugged off.
  - Wave 34 lane 6 repaired this definition before any re-enable: the
    `perf-load-budget` `services:` block was indented one level too deep
    (the file had never parsed), both Boot steps omitted the explicit
    `JWT_SECRET` / `MARULA_SECRET_KEY` that fail-closed core-api
    requires since Wave 19, and both inherited the superuser
    `DATABASE_URL` (RLS bypassed) instead of the app role.
  - The `.disabled` suffix is USER-GATED: turning the schedule on is a
    user decision, never a lane action.
- Budget overrides are for local triage only: `PERF_SMOKE_BUDGET_MS` /
  `PERF_SMOKE_DEEP_PAGE_BUDGET_MS` (via `measure::assert_budget`
  constants). A CI failure is investigated, never shrugged off.
- No PG port claims: the smoke uses the CI service DB (5432) or a
  testcontainer on a random host port. Local runs use a scratch port
  outside the lane range (e.g. 54399).

## 7. Sizing method: users per instance (Wave 49 lane R5)

The single-endpoint runner answers "how fast is the board list". A
users-per-instance number needs a session, so `perf-harness journey` drives
the multi-endpoint agent workload described in §2.

```sh
MARULA_TEST_DATABASE_URL=... perf-harness seed --tickets 50000
DATABASE_URL=... BIND_ADDR=127.0.0.1:18080 core-api &
perf-harness journey --base-url http://127.0.0.1:18080 \
  --users 16 --think-ms 500 --duration 30s --tickets 50000 --label 50k \
  --budget-ms 150 --out journey-50k.json
```

The run exits non-zero when any endpoint rejects more than 1% of requests or
when aggregate p95 crosses the budget.

The report carries per-endpoint latency and two aggregate numbers the
capacity model multiplies:

- `req_per_user_rps` = throughput / users. This is the per-user request rate
  at the chosen think-time, the open assumption from the R1 capacity note.
- `users_per_instance` = throughput x (think_ms + p50) / 1000. Little's law:
  the users one instance holds at the measured pace.

`--users` is the closed-loop client count. Raising it raises throughput until
the box saturates; the largest run whose p95 still clears the budget is the
ceiling, and its `users_per_instance` is the number to quote. `--think-ms` is
the knob that turns throughput into a user count, so it is always stated
beside the number.

The **dataset-penalty exponent** (the second open assumption) comes from the
same command at two scales. Run it at 50k and 100k and read the ratio, with
`--label` recording the scale. A power law over a doubling gives
`log(p95_100k / p95_50k) / log(2)`. Say which mix grew: page-1 reads are flat
in dataset size (§3), so a rise is the write or deep-read legs.

### Measured (2026-10-17)

Apple Silicon dev box, release core-api, scratch Postgres (e2e-stack), the perf
org freshly seeded @50k, think 500ms, 30s per run, 0 errors in every run. A
parallel lane's e2e stack was up during the window, so absolute throughput is
contention-bound. The sweep is internally consistent: each point's
`users_per_instance` reconstructs its own `users`, which is what a closed loop
at that pace should do.

| users | rps | p50 | p95 | p99 | req/user | users/inst | budget |
|---|---|---|---|---|---|---|---|
| 16 | 30.9 | 13.0 | 47.2 | 55.5 | 1.93 | 15.9 | OK |
| 64 | 123.7 | 15.0 | 50.8 | 66.0 | 1.93 | 63.7 | OK |
| 256 | 485.0 | 8.9 | 48.2 | 163.7 | 1.89 | 246.8 | OK |
| 384 | 695.3 | 25.9 | 93.2 | 294.8 | 1.81 | 365.7 | OK |
| 512 | 747.8 | 148.1 | 257.4 | 442.8 | 1.46 | 484.7 | OVER |

The knee sits between 384 sessions (695 rps, 93ms p95) and 512 (257ms p95,
over). So on this box **one instance holds ~384 concurrent mixed sessions at
p95 ≤ 150ms**, and the per-user rate is 1.8-1.9 rps while the box has
headroom, falling toward 1.5 as it saturates.

The **dataset-penalty exponent was then measured on a quiet box**, in a
follow-up with no other stack running. Same procedure, one stack at a time, a
fresh seed and a 25s settle per scale, the two sub-saturation concurrencies,
and a second 50k pass to prove the box held steady across the session. The two
50k passes agreed within noise, so the cross-scale comparison is trustworthy:

| concurrency | scale | rps | p50 | p95 |
|---|---|---|---|---|
| 32 | 50k | 60.8 | 20.8 | 51.3 |
| 32 | 100k | 61.0 | 19.9 | 58.2 |
| 64 | 50k | 121.8 | 21.8 | 53.3 |
| 64 | 100k | 121.6 | 20.9 | 69.3 |

Throughput is unchanged and the median is unchanged. The penalty is in the
tail: p95 rises 1.13 to 1.30 times, an exponent of about 0.2 to 0.4 over the
doubling, and the growth is the `list_board` leg (p95 56.8ms to 68.0ms at
concurrency 32, 61.1ms to 92.4ms at 64). The other endpoints stay flat. So at
this scale the dataset cost is sub-linear and lives in the tail, and the
Little's-law users-per-instance figure, which reads the median, is
dataset-insensitive between 50k and 100k. The method and the raw reports are in
`internal/design/qa-p49-100k-penalty/`.

The full model, with the sizing table, the calculator and the limits, is
`docs/capacity-model.md`.
