#!/usr/bin/env bash
# p13-test.sh — one test entry point, nextest when it exists.
#
#   scripts/p13-test.sh                    the workspace sweep, whatever runner is here
#   scripts/p13-test.sh -p core-api        arguments go to the runner
#   scripts/p13-test.sh --doc              doctests, which nextest does not run
#   scripts/p13-test.sh --list-runners     what would run, and why
#
# cargo-nextest runs one process per test, schedules across cores and prints a
# per-test table with timings; `cargo test` runs every suite binary in one
# process after another. Measured on this tree in
# design/qa-p52-build-throughput/REVIEW.md.
#
# The nextest path is not forced. Plain `cargo test --workspace` still works
# exactly as before on a machine without nextest, and this script falls back to
# it without asking. P13_TEST_RUNNER=cargo forces the fallback on a machine that
# has both (the reason to do that: a suite that shares process state with
# another suite behaves differently under one-process-per-test, so a red
# result under nextest and green under cargo test is a test-isolation bug worth
# seeing rather than hiding).
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || exit 2

RUNNER="${P13_TEST_RUNNER:-}"
USE_NEXTEST=0

detect() {
  if [ "$RUNNER" = "cargo" ]; then USE_NEXTEST=0; return; fi
  if [ -n "$RUNNER" ] && [ "$RUNNER" != "nextest" ]; then
    printf 'p13-test: unknown P13_TEST_RUNNER=%s (cargo|nextest)\n' "$RUNNER" >&2
    exit 2
  fi
  if command -v cargo-nextest >/dev/null 2>&1 \
     || command -v cargo-nextest-nextest >/dev/null 2>&1; then
    USE_NEXTEST=1
  elif [ "$RUNNER" = "nextest" ]; then
    printf 'p13-test: P13_TEST_RUNNER=nextest but cargo-nextest is not installed\n' >&2
    printf '          cargo binstall cargo-nextest\n' >&2
    exit 2
  fi
}

detect

# --list-runners answers "why did my suite change shape" without running it.
for arg in "$@"; do
  if [ "$arg" = "--list-runners" ]; then
    if [ "$USE_NEXTEST" = 1 ]; then
      printf 'runner   cargo nextest run\n'
      printf 'config   .config/nextest.toml (profile default)\n'
      printf 'note     doctests are NOT run by nextest; add --doc or run cargo test --doc\n'
    else
      printf 'runner   cargo test\n'
      printf 'reason   %s\n' \
        "$([ "$RUNNER" = "cargo" ] && echo 'P13_TEST_RUNNER=cargo' || echo 'cargo-nextest not installed')"
    fi
    exit 0
  fi
done

# --doc: cargo test is the only runner for doctests.
DOCTESTS=0
for arg in "$@"; do
  [ "$arg" = "--doc" ] && DOCTESTS=1 && break
done

if [ "$DOCTESTS" = 1 ]; then
  printf 'p13-test: cargo test %s\n' "$*"
  exec cargo test "$@"
fi

# An empty argument list becomes the workspace sweep, not `cargo test` with no
# filter (which is the same thing, but stated once so the log reads clearly).
if [ "$#" -eq 0 ]; then
  set -- --workspace --no-fail-fast
fi

if [ "$USE_NEXTEST" = 1 ]; then
  printf 'p13-test: cargo nextest run %s\n' "$*"
  exec cargo nextest run "$@"
fi

printf 'p13-test: cargo test %s   (no nextest on this machine)\n' "$*"
exec cargo test "$@"
