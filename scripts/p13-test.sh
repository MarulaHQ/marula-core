#!/usr/bin/env bash
# p13-test.sh — one test entry point, nextest when it exists.
#
#   scripts/p13-test.sh                    the workspace sweep, whatever runner is here
#   scripts/p13-test.sh -p core-api        arguments go to the runner
#   scripts/p13-test.sh --doc              doctests, which nextest does not run
#   scripts/p13-test.sh --list-runners     what would run, and why
#   scripts/p13-test.sh --sweep 2 -p core-api   GC stale artifacts, then run
#   scripts/p13-test.sh --batch 12 -p core-api  chunked run, bounded disk
#   scripts/p13-test.sh --print-plan --batch 12 -p core-api   show the chunks
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

# --- lane-gate flags (gate-ergonomics lane, 2026-10-28) ----------------------
# --sweep N     reclaim stale artifact generations before running (DX6c). Cargo
#               never collects old artifact generations, so the shared cache
#               grows by roughly one cycle per source change until something
#               sweeps it. A full core-api cycle is ~116 integration binaries at
#               ~2-3GB per generation.
# --batch N     run a package's integration targets in chunks of N and clean the
#               package's artifacts between chunks, so peak disk is one chunk
#               rather than every target at once. --test is a cargo test flag,
#               so this path runs cargo test even when nextest is installed.
# --print-plan  show what --sweep and --batch would do, and run nothing.
SWEEP_DAYS=""
BATCH=0
PRINT_PLAN=0
LANE_PKGS=()
PASS=()
while [ "$#" -gt 0 ]; do
  case "$1" in
    --sweep|--batch)
      [ "$#" -ge 2 ] || { printf 'p13-test: %s needs a value
' "$1" >&2; exit 2; }
      if [ "$1" = "--sweep" ]; then SWEEP_DAYS="$2"; else BATCH="$2"; fi
      shift 2 ;;
    --sweep=*) SWEEP_DAYS="${1#--sweep=}"; shift ;;
    --batch=*) BATCH="${1#--batch=}"; shift ;;
    --print-plan) PRINT_PLAN=1; shift ;;
    -p|--package)
      [ "$#" -ge 2 ] || { printf 'p13-test: %s needs a package name
' "$1" >&2; exit 2; }
      LANE_PKGS+=("$2"); PASS+=("$1" "$2"); shift 2 ;;
    --package=*) LANE_PKGS+=("${1#--package=}"); PASS+=("$1"); shift ;;
    *) PASS+=("$1"); shift ;;
  esac
done
set -- ${PASS[@]+"${PASS[@]}"}

if [ -n "$SWEEP_DAYS" ] && ! printf '%s' "$SWEEP_DAYS" | grep -Eq '^[0-9]+$'; then
  printf 'p13-test: --sweep needs a day count, e.g. --sweep 2\n' >&2; exit 2
fi
if [ "$BATCH" != 0 ] && ! printf '%s' "$BATCH" | grep -Eq '^[1-9][0-9]*$'; then
  printf 'p13-test: --batch needs a positive target count, e.g. --batch 12\n' >&2; exit 2
fi

# A sweep before a heavy run is the cheapest way to keep the cache at one
# generation. --print-plan leaves the cache alone.
if [ -n "$SWEEP_DAYS" ]; then
  printf 'p13-test: sweeping artifacts older than %sd first\n' "$SWEEP_DAYS"
  if [ "$PRINT_PLAN" = 0 ]; then
    "$ROOT/scripts/dev-clean.sh" --go --stale "$SWEEP_DAYS" || exit $?
  fi
fi

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


# Batched run: one chunk of a package's integration targets at a time, with the
# package's artifacts collected between chunks. Coverage is the same as
# `-p <package>`; peak disk is one chunk. Stop at the first failing chunk so the
# log names the target that broke.
if [ "$BATCH" -gt 0 ]; then
  if [ "${#LANE_PKGS[@]}" -eq 0 ]; then
    printf 'p13-test: --batch needs at least one -p <package>\n' >&2; exit 2
  fi
  free_gi() { df -Pk . | awk 'NR==2 { print int($4 / 1048576) }'; }
  printf 'p13-test: batching %s target(s) per chunk (cargo test)\n' "$BATCH"
  tmp="$(mktemp -t p13-targets)"
  trap 'rm -f "$tmp"' EXIT
  PKGS="$(IFS=,; printf '%s' "${LANE_PKGS[*]}")" python3 - > "$tmp" <<'PLAN'
import json, os, subprocess
wanted = [p for p in os.environ["PKGS"].split(",") if p]
meta = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--no-deps", "--format-version", "1"]))
for pkg in meta["packages"]:
    if pkg["name"] in wanted:
        for target in pkg["targets"]:
            if "test" in target.get("kind", []):
                print(pkg["name"] + "\t" + target["name"])
PLAN
  failed=0
  cur=""
  tests=()
  flush_chunks() {
    [ -n "$cur" ] || return 0
    total=${#tests[@]}
    i=0
    while [ "$i" -lt "$total" ]; do
      chunk=("${tests[@]:$i:$BATCH}")
      targs=()
      for t in "${chunk[@]}"; do targs+=("--test" "$t"); done
      printf 'p13-test: %s targets %s-%s of %s (free %sGi)\n' \
        "$cur" "$((i + 1))" "$((i + ${#chunk[@]}))" "$total" "$(free_gi)"
      if [ "$PRINT_PLAN" = 1 ]; then
        printf '           cargo test -p %s %s\n' "$cur" "${targs[*]}"
      else
        if ! cargo test -p "$cur" "${targs[@]}"; then
          failed=$((failed + 1))
          break
        fi
        i=$((i + BATCH))
        [ "$i" -lt "$total" ] && cargo clean -p "$cur" >/dev/null 2>&1
        continue
      fi
      i=$((i + BATCH))
    done
    tests=()
  }
  while IFS="$(printf '\t')" read -r pkg tname; do
    [ -n "$pkg" ] || continue
    if [ -n "$cur" ] && [ "$pkg" != "$cur" ]; then
      flush_chunks
    fi
    cur="$pkg"
    tests+=("$tname")
  done < "$tmp"
  flush_chunks
  if [ "$failed" -ne 0 ]; then
    printf 'p13-test: %s chunk(s) failed\n' "$failed" >&2
    exit 1
  fi
  printf 'p13-test: every chunk passed\n'
  exit 0
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
