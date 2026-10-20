#!/usr/bin/env bash
# p13-build-bench.sh — measure the build levers instead of assuming them.
#
#   scripts/p13-build-bench.sh                    before/after on a cold target
#   scripts/p13-build-bench.sh --legs all         every leg, warm cache included
#   scripts/p13-build-bench.sh --legs baseline,turbo,test   add the test comparison
#   scripts/p13-build-bench.sh --dry-run          print the plan, build nothing
#   scripts/p13-build-bench.sh --json out.json    also write the numbers as JSON
#   scripts/p13-build-bench.sh --require-accelerator
#                                                 exit 3 instead of reporting a
#                                                 noise floor as if it were a win
#   scripts/p13-build-bench.sh --keep             keep the per-leg target dirs
#
# A failed leg, a contaminated run (cargo started mid-measurement) or
# --require-accelerator with no accelerator exits non-zero.
#
# Every leg compiles one fixed target set from a COLD target directory of its
# own under $TMPDIR. Nothing here writes `.kilo/target-shared`, so a measurement
# cannot invalidate a lane's artifacts and a lane's build cannot move these
# numbers.
#
# WHY IT REFUSES TO RUN WHILE SOMETHING ELSE COMPILES. A wall clock taken next
# to another compiler measures contention, not this tree. The script refuses
# when any cargo or rustc process is live and names it.
#
# WHAT THE LEGS ARE.
#   baseline    stock cargo: no wrapper, no linker override (RUSTFLAGS set but
#               empty, which drops any target rustflags a stray layer added)
#   turbo       baseline plus whatever this host really has — ld64.lld / lld /
#               mold as the linker, sccache as rustc's wrapper
#   turbo-warm  turbo again, same sccache directory, fresh target dir: the
#               branch-switch case, and the only leg that shows what sccache is
#               worth once it has seen these crates
#   test        the suites already built, run twice on the same binaries: once
#               under `cargo test`, once under `cargo nextest run`
#
# If this host has neither a linker nor sccache, `baseline` and `turbo` are the
# same configuration. The run says that instead of reporting a speedup it did
# not measure.
#
# The machine-local overlay (.cargo/p13-fast.local.toml) has to be absent: it is
# a third state that would leak into both legs. `scripts/p13-build-env.sh
# disable` is the one command that removes it.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || exit 2

LEGS="baseline,turbo"
REPEATS=1
DRY_RUN=0
FORCE=0
KEEP=0
REQUIRE_ACCEL=0
JSON_OUT=""
PACKAGES="${P13_BENCH_PACKAGES:--p core-domain -p core-events -p core-otel --all-targets}"
# nextest takes a package/suite filter but not cargo's `--all-targets`, so the
# test leg names the same packages without it.
TEST_PACKAGES="$(printf '%s' "$PACKAGES" | sed 's/--all-targets//g' | sed 's/  */ /g; s/^ //; s/ $//')"

for arg in "$@"; do
  case "$arg" in
    --legs=*) LEGS="${arg#--legs=}" ;;
    --repeats=*) REPEATS="${arg#--repeats=}" ;;
    --json=*) JSON_OUT="${arg#--json=}" ;;
    --dry-run) DRY_RUN=1 ;;
    --force) FORCE=1 ;;
    --keep) KEEP=1 ;;
    --require-accelerator) REQUIRE_ACCEL=1 ;;
    -h|--help) sed -n '2,44p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    --legs) SET_LEGS=1 ;;
    --repeats) SET_REPEATS=1 ;;
    --json) SET_JSON=1 ;;
    -*) printf 'p13-build-bench: unknown argument: %s\n' "$arg" >&2; exit 2 ;;
    *)
      if [ "${SET_LEGS:-0}" = 1 ]; then LEGS="$arg"; SET_LEGS=0
      elif [ "${SET_REPEATS:-0}" = 1 ]; then REPEATS="$arg"; SET_REPEATS=0
      elif [ "${SET_JSON:-0}" = 1 ]; then JSON_OUT="$arg"; SET_JSON=0
      else printf 'p13-build-bench: unexpected argument: %s\n' "$arg" >&2; exit 2
      fi ;;
  esac
done
[ "${SET_LEGS:-0}" = 1 ] && LEGS="baseline,turbo"
[ "${SET_REPEATS:-0}" = 1 ] && REPEATS=1
[ "${SET_JSON:-0}" = 1 ] && JSON_OUT="p13-build-bench.json"

[ "$LEGS" = "all" ] && LEGS="baseline,turbo,turbo-warm"
case ",$LEGS," in
  *,baseline,*|*,turbo,*|*,turbo-warm,*|*,test,*) ;;
  *) printf 'p13-build-bench: --legs takes baseline,turbo,turbo-warm,test (or all)\n' >&2; exit 2 ;;
esac
case "$REPEATS" in ''|*[!0-9]*) printf 'p13-build-bench: --repeats takes a number\n' >&2; exit 2 ;; esac

# ---------------------------------------------------------------------------
# helpers (defined before any use; bash resolves functions at call time)
# ---------------------------------------------------------------------------
now_ms() { python3 -c 'import time; print(int(time.time()*1000))'; }
secs() { python3 -c "print(f'{$1/1000:.1f}')"; }
pct_delta() { # <base_ms> <new_ms>; negative means faster
  python3 - "$1" "$2" <<'PY'
import sys
base, new = int(sys.argv[1]), int(sys.argv[2])
print(f"{(new - base) * 100.0 / base:+.1f}%")
PY
}
busy_pids() {
  pgrep -f 'cargo (build|check|test|run|clippy)|rustc --crate-name' 2>/dev/null \
    | awk -v me="$$" -v up="$PPID" '$1 != me && $1 != up'
}
refuse_or_exit() {
  printf 'p13-build-bench: REFUSED — %s\n' "$1" >&2
  [ -n "${2:-}" ] && printf '                 %s\n' "$2" >&2
  exit 3
}

# ---------------------------------------------------------------------------
# preflight
# ---------------------------------------------------------------------------
OVERLAY="$ROOT/.cargo/p13-fast.local.toml"
if [ -f "$OVERLAY" ] && [ "$FORCE" != 1 ]; then
  refuse_or_exit "the machine overlay exists, so neither leg is stock" \
    "scripts/p13-build-env.sh disable   (then re-run; --force measures the current state anyway)"
fi

BUSY="$(busy_pids)"
if [ -n "$BUSY" ] && [ "$FORCE" != 1 ]; then
  refuse_or_exit "cargo/rustc is running (pids: $(printf '%s' "$BUSY" | tr '\n' ' '))" \
    "wait for it — a measurement sharing a machine with a build is not a measurement (--force to override)"
fi

INFO_JSON="$(bash scripts/p13-build-env.sh info --json 2>/dev/null)"
field() { printf '%s' "$INFO_JSON" | sed -n "s/.*\"$1\":\"\([^\"]*\)\".*/\1/p"; }
bool_field() { printf '%s' "$INFO_JSON" | sed -n "s/.*\"$1\":\(true\|false\).*/\1/p"; }

LINKER_FLAG="$(field linker_flag)"
LINKER_KIND="$(field linker)"
HAS_SCCACHE="$(bool_field sccache)"
HAS_NEXTEST="$(bool_field nextest)"
TREE="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
DIRTY=""
[ -n "$(git -C "$ROOT" status --porcelain 2>/dev/null)" ] && DIRTY=" (+dirty)"

ACCELERATOR=0
[ -n "$LINKER_FLAG" ] && ACCELERATOR=1
[ "$HAS_SCCACHE" = 1 ] && ACCELERATOR=1

leg_env_desc() {
  case "$1" in
    baseline) printf 'stock: no wrapper, no linker override' ;;
    turbo) printf 'wrapper=%s linker=%s' \
             "$([ "$HAS_SCCACHE" = 1 ] && echo sccache || echo none)" \
             "${LINKER_KIND:-none}" ;;
    turbo-warm) printf 'as turbo, sccache cache already filled' ;;
    test) printf 'two runners, one set of built binaries' ;;
  esac
}

WORK="$(mktemp -d "${TMPDIR:-/tmp}/p13-bench.XXXXXX")"
cleanup() {
  if [ "$KEEP" = 1 ]; then printf 'kept: %s\n' "$WORK"; else rm -rf "$WORK"; fi
}
trap cleanup EXIT

printf '\n=== P13 build throughput ===\n'
printf 'host      %s %s\n' "$(field os)" "$(uname -m)"
printf 'tree      %s%s\n' "$TREE" "$DIRTY"
printf 'targets   cargo build %s\n' "$PACKAGES"
printf 'tools     linker=%s  sccache=%s  nextest=%s\n' \
  "${LINKER_KIND:-none}" \
  "$([ "$HAS_SCCACHE" = 1 ] && echo yes || echo no)" \
  "$([ "$HAS_NEXTEST" = 1 ] && echo yes || echo no)"
printf 'legs      %s   repeats=%s\n' "$LEGS" "$REPEATS"
printf 'work      %s\n' "$WORK"

if [ "$ACCELERATOR" = 0 ]; then
  printf '\nNO ACCELERATOR ON THIS HOST. baseline and turbo are the same\n'
  printf 'configuration, so the delta below is the noise floor, not a speedup.\n'
  printf 'Install one and re-run:\n'
  printf '  %s\n' "$(bash scripts/p13-build-env.sh info 2>/dev/null | sed -n 's/^              install: //p' | head -1)"
  printf '  cargo binstall sccache\n'
  if [ "$REQUIRE_ACCEL" = 1 ] && [ "$DRY_RUN" = 0 ]; then
    refuse_or_exit "--require-accelerator was given but this host has no linker and no sccache" \
      "nothing was built; install a tool above and re-run, or drop the flag to measure the noise floor"
  fi
fi

if [ "$DRY_RUN" = 1 ]; then
  printf '\nplan (nothing built):\n'
  for leg in ${LEGS//,/ }; do
    printf '  %-11s target=<work>/%s-r1  %s\n' "$leg" "$leg" "$(leg_env_desc "$leg")"
  done
  exit 0
fi

# ---------------------------------------------------------------------------
# one cold build
# ---------------------------------------------------------------------------
# leg_env <leg> <sccache-dir> — `env` arguments for that leg
leg_env() {
  local leg="$1" sccachedir="$2"
  case "$leg" in
    baseline)
      # Set-but-empty RUSTFLAGS is the documented stock baseline: zero flags,
      # and it outranks any target rustflags another config layer contributed.
      printf '%s\n' "-u" "RUSTC_WRAPPER" "RUSTFLAGS="
      ;;
    turbo|turbo-warm)
      if [ "$HAS_SCCACHE" = 1 ]; then
        printf '%s\n' "RUSTC_WRAPPER=sccache" "SCCACHE_DIR=$sccachedir" "SCCACHE_CACHE_SIZE=10G"
      else
        printf '%s\n' "-u" "RUSTC_WRAPPER" "SCCACHE_DIR=$sccachedir"
      fi
      printf '%s\n' "RUSTFLAGS=$LINKER_FLAG"
      ;;
  esac
}

time_build() { # <leg> <target-dir> <sccache-dir> — prints elapsed ms, exit 0/1
  local leg="$1" tdir="$2" sccachedir="$3"
  local envs=() line rc start end
  while IFS= read -r line; do envs+=("$line"); done < <(leg_env "$leg" "$sccachedir")
  mkdir -p "$tdir"
  start="$(now_ms)"
  env "${envs[@]}" CARGO_TARGET_DIR="$tdir" cargo build $PACKAGES >"$WORK/$leg.log" 2>&1
  rc=$?
  end="$(now_ms)"
  printf '%s' "$((end - start))"
  return "$rc"
}

print_row() { printf '  %-11s %9s  %9s\n' "$1" "$2" "$3"; }
MS_BASELINE=""
MS_TURBO=""
MS_WARM=""
FAILED=""
SCCACHE_SHARED="$WORK/sccache"

for rep in $(seq 1 "$REPEATS"); do
  [ "$REPEATS" -gt 1 ] && printf '\nrepeat %s of %s\n' "$rep" "$REPEATS"
  [ "$rep" = 1 ] && printf '\n  %-11s %9s  %9s\n' "leg" "wall" "vs base"
  [ "$rep" = 1 ] && printf '  %s\n' "--------------------------------------------"

  case ",$LEGS," in *,baseline,*)
    if ms="$(time_build baseline "$WORK/baseline-r$rep" "")"; then
      MS_BASELINE="$ms"; print_row baseline "$(secs "$ms")s" "—"
    else FAILED="$FAILED baseline"; print_row baseline "FAILED" "—"; fi
  ;; esac

  case ",$LEGS," in *,turbo,*)
    if ms="$(time_build turbo "$WORK/turbo-r$rep" "$SCCACHE_SHARED")"; then
      MS_TURBO="$ms"
      d="—"; [ -n "$MS_BASELINE" ] && d="$(pct_delta "$MS_BASELINE" "$ms")"
      print_row turbo "$(secs "$ms")s" "$d"
    else FAILED="$FAILED turbo"; print_row turbo "FAILED" "—"; fi
  ;; esac

  case ",$LEGS," in *,turbo-warm,*)
    if ms="$(time_build turbo-warm "$WORK/turbo-warm-r$rep" "$SCCACHE_SHARED")"; then
      MS_WARM="$ms"
      d="—"; [ -n "$MS_BASELINE" ] && d="$(pct_delta "$MS_BASELINE" "$ms")"
      print_row turbo-warm "$(secs "$ms")s" "$d"
    else FAILED="$FAILED turbo-warm"; print_row turbo-warm "FAILED" "—"; fi
  ;; esac
done

# ---------------------------------------------------------------------------
# the test-runner lever
# ---------------------------------------------------------------------------
case ",$LEGS," in *,test,*)
  if [ "$HAS_NEXTEST" = 0 ]; then
    printf '\n  test leg skipped: cargo-nextest is not installed\n'
  else
    tdir="$WORK/baseline-r1"
    if [ ! -d "$tdir" ]; then
      tdir="$WORK/test"
      mkdir -p "$tdir"
      printf '\n  building the suites once, untimed …\n'
      env -u RUSTC_WRAPPER RUSTFLAGS= CARGO_TARGET_DIR="$tdir" \
        cargo build $PACKAGES >"$WORK/test-build.log" 2>&1 \
        || printf '  test leg skipped: the untimed build failed (%s)\n' "$WORK/test-build.log"
    fi
    if [ -d "$tdir" ]; then
      printf '\n  test leg: %s (nextest does not run doctests; cargo test does)\n' "$TEST_PACKAGES"
      t0="$(now_ms)"
      env -u RUSTC_WRAPPER RUSTFLAGS= CARGO_TARGET_DIR="$tdir" \
        cargo test $TEST_PACKAGES --no-fail-fast >"$WORK/run-cargo-test.log" 2>&1
      c_rc=$?
      t1="$(now_ms)"
      t2="$(now_ms)"
      env -u RUSTC_WRAPPER RUSTFLAGS= CARGO_TARGET_DIR="$tdir" \
        cargo nextest run $TEST_PACKAGES --no-fail-fast >"$WORK/run-nextest.log" 2>&1
      n_rc=$?
      t3="$(now_ms)"
      printf '  %-11s %9s  %9s   exit %s\n' "cargo test" "$(secs "$((t1 - t0))")s" "—" "$c_rc"
      printf '  %-11s %9s  %9s   exit %s\n' "nextest" "$(secs "$((t3 - t2))")s" \
        "$(pct_delta "$((t1 - t0))" "$((t3 - t2))")" "$n_rc"
    fi
  fi
;; esac

# ---------------------------------------------------------------------------
# verdict inputs
# ---------------------------------------------------------------------------
printf '\n'
if [ -n "$FAILED" ]; then
  printf 'FAILED legs:%s — logs in %s\n' "$FAILED" "$WORK"
  exit 1
fi
BUSY_AFTER="$(busy_pids)"
if [ -n "$BUSY_AFTER" ]; then
  printf 'CONTAMINATED: cargo/rustc ran while the legs were timing (pids: %s).\n' \
    "$(printf '%s' "$BUSY_AFTER" | tr '\n' ' ')"
  printf 'These numbers are not a measurement; re-run on an idle machine.\n'
fi

if [ -n "$JSON_OUT" ]; then
  python3 - "$JSON_OUT" "$TREE" "$LINKER_KIND" "$HAS_SCCACHE" "$HAS_NEXTEST" \
    "$MS_BASELINE" "$MS_TURBO" "$MS_WARM" "$PACKAGES" <<'PY'
import json, sys
out, tree, linker, scc, nextest, base, turbo, warm, pkgs = sys.argv[1:10]
ms = lambda v: int(v) if v else None
doc = {
    "tree": tree, "targets": pkgs,
    "tools": {"linker": linker or None, "sccache": scc == "true", "nextest": nextest == "true"},
    "wall_ms": {"baseline": ms(base), "turbo": ms(turbo), "turbo_warm": ms(warm)},
}
if doc["wall_ms"]["baseline"] and doc["wall_ms"]["turbo"]:
    b = doc["wall_ms"]["baseline"]
    doc["turbo_delta_pct"] = round((doc["wall_ms"]["turbo"] - b) * 100.0 / b, 1)
open(out, "w").write(json.dumps(doc, indent=2) + "\n")
print(f"wrote {out}")
PY
fi
exit 0
