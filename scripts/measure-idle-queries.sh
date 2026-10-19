#!/usr/bin/env bash
# Measure Postgres transaction churn on an IDLE stack.
#
# Usage: scripts/measure-idle-queries.sh [seconds] [label]
#
# Reads E2E_DATABASE_URL (or DATABASE_URL) and samples the transaction counters
# in `pg_stat_database` for that database. Run it twice with nothing else
# happening — once with the worker configuration under test, once with the
# baseline — and the difference is that worker's idle load: core-api and the
# database contribute the same in both windows.
#
# Why BOTH counters: an idle poll is not always a commit. The outbox relay
# begins a transaction, finds nothing, and drops it — a rollback — so counting
# commits alone reports the poll that the wakeup removed as if it had never
# existed. (The first version of this script did exactly that and reported
# 6.62 against 6.60 for a change that removes about four transactions a
# second.) The poll rate is commits plus rollbacks.
#
# Why not `pg_stat_statements`: the extension is not installed in the shipped
# Postgres image. The counters answer this question without adding one.
set -euo pipefail

url="${E2E_DATABASE_URL:-${DATABASE_URL:-}}"
if [ -z "$url" ]; then
  echo "set E2E_DATABASE_URL (or DATABASE_URL) to the stack's database" >&2
  exit 2
fi

secs="${1:-60}"
label="${2:-idle}"

if ! [[ "$secs" =~ ^[0-9]+$ ]] || [ "$secs" -lt 1 ]; then
  echo "seconds must be a positive integer" >&2
  exit 2
fi

sample() {
  psql "$url" -tAc \
    "SELECT xact_commit, xact_rollback FROM pg_stat_database WHERE datname = current_database()" |
    tr '|' ' '
}

read -r commit_before rollback_before <<<"$(sample)"
sleep "$secs"
read -r commit_after rollback_after <<<"$(sample)"

commit_delta=$((commit_after - commit_before))
rollback_delta=$((rollback_after - rollback_before))
total=$((commit_delta + rollback_delta))

awk -v c="$commit_delta" -v r="$rollback_delta" -v t="$total" -v s="$secs" -v l="$label" \
  'BEGIN {
     printf "%s: %d commits + %d rollbacks = %d transactions over %ss = %.2f tx/s\n",
            l, c, r, t, s, t / s
   }'
