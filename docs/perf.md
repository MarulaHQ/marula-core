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

Test-infra crate, AGPL, not part of the API surface. Three pieces:

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
  commit`) through the `meridian_app` pool under RLS, over five request
  mixes (page 1, priority p1, assignee, client, deep page), 5 warmups +
  40 timed runs, p50/p95/p99/max per mix, plus `EXPLAIN (ANALYZE,
  BUFFERS)` for the page-1 mix. `perf-harness smoke` = seed + measure +
  budget assert (what CI runs).
- **HTTP load runner** (`perf-harness load`): logs the seeded harness user
  in through the real `POST /api/auth/login`, then drives
  `GET /api/tickets` with a deterministic request mix (xorshift-seeded,
  `--seed`, default 42): 60% page-1, 10% p1, 10% assignee, 10% client,
  10% deep page. Closed-loop workers (`--concurrency`, default 8),
  end-to-end latency (bearer auth, RLS tx, serialization, axum stack),
  JSON report, exit 1 over budget.

### Running it

```sh
# CI perf smoke (also: .github/workflows/ci.yml `perf-smoke` job)
./scripts/perf-smoke.sh            # DATABASE_URL env or throwaway testcontainer

# Local baseline against a scratch DB (use a port outside the lane range)
MERIDIAN_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:54399/meridian_perf \
  perf-harness seed --tickets 50000
MERIDIAN_TEST_DATABASE_URL=... perf-harness measure --budget-ms 150

# Scale leg: 100k tickets/org through the HTTP stack
MERIDIAN_TEST_DATABASE_URL=... perf-harness seed --tickets 100000 --clients 1000 --users 25
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
perf org, RLS-scoped `meridian_app` traffic. 5 warmups + 40 timed runs
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
  the lane landing record (TASKLIST_COMPLETED).

## 5. HTTP capacity decision (Wave 15 lane 3)

The saturated closed-loop point is **concurrency 8** — 8 back-to-back
workers sharing one bearer token, the documented reproducing
configuration (`perf-harness load --seed 42`). Measured after the keyset
fix: p95 49.2ms, 259 rps @50k — **the 150ms product budget holds at
saturation with ~3× headroom**. The pre-keyset 218ms reading was the
offset deep-page leg, not queueing capacity.

**Pool sizing / connection count (explicit):** one core-api instance
holds `app` pool max **20** (`core_db::connect`), the reporting pool max
**25** (`AppState::reporting_pool_lazy`), worker delivery pool max **5**,
webhooks pool max **1** — ≈ 51 connections per instance against the
Postgres default `max_connections = 100`. That leaves headroom for
superuser reserved slots and operator tooling only for the FIRST node:
deploying more instances (or the portal API) multiplies the 51-connection
floor, so the decision is: **either raise PG max_connections with the
instance count (51 + per-portal + admin headroom) or front the pools
with a session pooler before the second instance ships.** Acquire
timeouts are 10s (app) / 30s (reporting). Saturation at c8 keeps the
queue in the axum worker pool, never in the pool acquire path.

**Enforcement:** the nightly `Nightly perf` workflow gates the c8 @50k
run at 150ms (`--budget-ms 150`) and surfaces the JSON report as a step
summary + artifact. The 100k scale leg is measured on a 600ms
order-of-magnitude tripwire (capacity observation, not budget).

## 6. CI contract

- `perf-smoke` job runs `scripts/perf-smoke.sh` on every push/PR: seeds
  50k, measures (page-1, priority, assignee, client, keyset deep-page,
  legacy offset mixes), asserts. The SQL gate covers every shape the
  budget covers — including the keyset deep page since the cursor
  shipped. The test is env-gated (`MERIDIAN_PERF_SMOKE=1`) so the plain
  `cargo test --workspace` sweep skips it (fast sweeps).
- `Nightly perf` (`.github/workflows/nightly.yml.disabled`, cron 05:30 UTC
  + workflow_dispatch): gates the saturated HTTP capacity point
  (`perf-harness load --concurrency 8 --budget-ms 150` @50k through the
  release-built core-api, connected as `meridian_app` so RLS is in the
  path) and measures the 100k scale leg on a 600ms tripwire (not gated).
  Results surface as a step summary + `nightly-load-report` /
  `nightly-scale-report` artifacts. A red nightly is investigated in the
  next session, never shrugged off.
  - Wave 34 lane 6 repaired this definition before any re-enable: the
    `perf-load-budget` `services:` block was indented one level too deep
    (the file had never parsed), both Boot steps omitted the explicit
    `JWT_SECRET` / `MERIDIAN_SECRET_KEY` that fail-closed core-api
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
