# Perf program — budgets, harness, baselines

> Established by Wave 13 lane 8 `p13-perf-baseline` (test-infra only: no
> product code, no migrations). This is the Phase-4 perf program's anchor
> document: the budgets, the tooling that enforces them, and the measured
> baselines they were set against.

## 1. Budgets

| Budget | Target | Enforced by |
|---|---|---|
| Ticket list (primary reads) | **p95 < 150ms @ 50k tickets/org** | `perf-smoke` CI job (`.github/workflows/ci.yml`) via `measure::assert_budget` |
| Deep pagination (`OFFSET 40000`) | 400ms regression tripwire (not a product budget) | same smoke; see the finding below |
| Scale harness | 100k tickets/org, reproducible | `perf-harness seed/load` (manual / nightly) |

The primary budget covers every standard list shape: unfiltered page 1,
priority filter, assignee scope, client scope. Deep OFFSET pagination is
measured and tracked with a tripwire, not product-budgeted (finding F2).

## 2. Tooling (`crates/perf-harness`)

Test-infra crate, AGPL, not part of the API surface. Three pieces:

- **Seed generator** (`perf-harness seed`): deletes + re-inserts the perf
  org in one transaction. Every measured entity gets a deterministic id —
  `md5('<tag>')::uuid` computed in-database — so the same scale always
  reproduces the same dataset. Set-based `generate_series` inserts: 100k
  tickets + 150k custom-field values seed in ~12s. Distribution: priority
  10/20/60/10% (p1–p4), category 40/35/20/5% (closed/open/in_progress/
  waiting), 70% assigned round-robin, created_at spread over ~2 years,
  three ticket custom fields with values on ~50% of tickets.
- **Measurement** (`perf-harness measure`): runs the EXACT server-side
  path of `GET /api/tickets` — `tx.begin → set_org_and_user →
  core_db::tickets::list_tickets → custom_fields::list_values_many →
  commit` — through the `meridian_app` pool under RLS, over five request
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
page); only OFFSET pagination grows.

### HTTP end-to-end @ 50k (core-api, 30s load runs)

| Concurrency | Throughput | p50 | p95 | p99 | max |
|---|---|---|---|---|---|
| 1 (no queueing) | 33 rps | 18.4ms | **117.1ms** | 143.9ms | — |
| 8 (saturated, closed-loop) | 138 rps | 34.9ms | **217.9ms** | 336.3ms | 610ms |

### HTTP end-to-end @ 100k tickets/org (30s, concurrency 8)

| Throughput | p50 | p95 | p99 | max |
|---|---|---|---|---|
| 74 rps | 55.0ms | **528.8ms** | 686.1ms | 930.3ms |

## 4. Findings (measure-first output; product fixes ride later perf lanes)

- **F1 — Budget holds at the read layer.** The server-side ticket-list
  query path is well under 150ms p95 at 50k (≈30ms) and flat at 100k.
  The budget is realistic.
- **F2 — OFFSET pagination is the scaling debt.** Deep pages cost
  O(offset) rows scanned + sorted (measured 148–237ms p95 @50k, 108ms
  @100k with different plans). Follow-up (UNCHECKED in TASKLIST.md):
  keyset/cursor pagination for the ticket list before the offset budget
  can become a product budget.
- **F3 — Saturated closed-loop load overshoots.** With 8 back-to-back
  workers, queueing (not the query) pushes HTTP p95 over budget even at
  50k (218ms); single-stream traffic is comfortably inside (117ms).
  The HTTP budget at load is a capacity question (pool sizing,
  connection count, worker pool) — track with the Phase-4 load program;
  `perf-harness load` is the measuring stick (`--budget-ms` gates it).
- **F3a — per-request HTTP overhead ≈ 6–8ms** (auth + JSON + stack):
  p50 18.4ms HTTP vs ≈10ms SQL for page 1. Bounded, not urgent.
- **F4 — Plan shape @50k:** RLS one-time-filter + parallel seq scan +
  top-N heapsort over the org's tickets (EXPLAIN captured in every
  `measure` run). An index serving the `(priority CASE, due_at,
  created_at)` sort is the candidate optimization if page-1 latency
  ever creeps toward budget — deliberately NOT added now (test-infra
  lane, no migrations).

## 5. CI contract

- `perf-smoke` job runs `scripts/perf-smoke.sh` on every push/PR: seeds
  50k, measures, asserts. The test is env-gated (`MERIDIAN_PERF_SMOKE=1`)
  so the plain `cargo test --workspace` sweep skips it (fast sweeps).
- Budget overrides are for local triage only: `PERF_SMOKE_BUDGET_MS` /
  `PERF_SMOKE_DEEP_PAGE_BUDGET_MS` (via `measure::assert_budget`
  constants). A CI failure is investigated, never shrugged off.
- No PG port claims: the smoke uses the CI service DB (5432) or a
  testcontainer on a random host port; local runs use a scratch port
  outside the lane range (e.g. 54399).
