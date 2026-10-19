#!/usr/bin/env bash
# Refresh the `.sqlx` offline cache after editing a `sqlx::query!` macro.
#
# Usage: scripts/sqlx-prepare.sh [database-url]
#
# Why this exists: the workspace builds with `SQLX_OFFLINE=true`, so a changed
# query has no cached type data and `cargo check` fails with "no cached data for
# this query". The fix is `cargo sqlx prepare`, which needs a migrated database
# and `SQLX_OFFLINE` unset — a combination that is easy to get wrong. This is the
# scripted form, and it ends by proving the cache is complete with the same
# offline check CI runs.
#
# Commit the `.sqlx` changes together with the query that caused them.
set -euo pipefail

cd "$(dirname "$0")/.."

url="${1:-${DATABASE_URL:-}}"
if [ -z "$url" ]; then
  echo "usage: scripts/sqlx-prepare.sh <database-url>   (or set DATABASE_URL)" >&2
  exit 2
fi

export DATABASE_URL="$url"
# The macros must talk to the database, so the offline switch comes off for
# the prepare run only.
unset SQLX_OFFLINE

echo "[sqlx-prepare] applying migrations"
cargo sqlx migrate run --source crates/core-db/migrations

echo "[sqlx-prepare] preparing the offline cache"
cargo sqlx prepare --workspace -- --all-targets

echo "[sqlx-prepare] verifying the cache with SQLX_OFFLINE=true"
SQLX_OFFLINE=true cargo check --workspace --all-targets

echo "[sqlx-prepare] done — commit the .sqlx changes with the query edit"
