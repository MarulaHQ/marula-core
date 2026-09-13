#!/bin/sh
# Perf smoke (Wave 13 lane 8): ticket-list p95 budget @50k tickets/org.
#
# CI-runnable fast check — NOT the full load run. Seeds the reproducible
# 50k perf org, measures the exact server-side query path of
# GET /api/tickets (list + batched custom-field enrichment under RLS), and
# FAILS when p95 exceeds the budget (150ms default, PERF_SMOKE_BUDGET_MS
# to override). The 100k HTTP load run is a separate manual/nightly
# harness (perf-harness seed/load) — see docs/perf.md.
#
# Database selection mirrors the core-db test suites (handled inside the
# test bootstrap, not here): DATABASE_URL / MERIDIAN_TEST_DATABASE_URL env
# wins; otherwise a throwaway pgvector/pgvector:pg17 testcontainer spins up
# on a random host port (no PG port claims — the lane convention).
set -eu

export MERIDIAN_PERF_SMOKE=1

exec cargo test -p perf-harness --test perf_smoke -- --nocapture
