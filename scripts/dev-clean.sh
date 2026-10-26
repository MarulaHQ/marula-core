#!/usr/bin/env zsh
# dev-clean.sh — disk-space housekeeping (the private build tracker DX5).
#
#   scripts/dev-clean.sh              dry-run: list what would be cleared
#   scripts/dev-clean.sh --go         actually clear it
#   scripts/dev-clean.sh --go --deep  ALSO reclaim the shared cargo cache's
#                                     full build artifacts + drifted worktree
#                                     target dirs (waits for a cargo build gap
#                                     first; cold-cache rebuild follows)
#   scripts/dev-clean.sh --go --stale N
#                                     ALSO delete shared-cache artifacts not
#                                     touched in N days (fine-grained GC for
#                                     stale deps/ generations; keeps what the
#                                     current tree still links against —
#                                     unlike --deep it never forces a full
#                                     cold rebuild of live artifacts)
#
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# zsh sets $0 to the FUNCTION name inside a function, so the self-test cannot
# re-invoke the script through $0 — capture the script path at top level.
SELF="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
GO=0
DEEP=0
STALE=0
SELFTEST=0
for arg in "$@"; do
  case "$arg" in
    selftest) SELFTEST=1; GO=0 ;;   # proves the counting rules; touches nothing
    --go) GO=1 ;;
    --deep) DEEP=1 ;;
    --stale) STALE=-1 ;; # marker; next arg is the day count
    --stale=*) STALE_DAYS="${arg#--stale=}" ;;
    *) if [[ $STALE -eq -1 ]]; then STALE_DAYS="$arg"; STALE=1; fi ;;
  esac
done
if [[ $STALE -eq 1 && ! "$STALE_DAYS" =~ '^[0-9]+$' ]]; then
  echo "dev-clean: --stale requires a day count, e.g. --stale 7" >&2
  exit 1
fi

# docker: exited containers from merged lanes, orphaned volumes, dangling images
# The docker CLI hangs indefinitely when the daemon is wedged (2026-09-10) and
# ignores the alarm-exec trick (Go runtime owns SIGALRM) — every call runs
# under a kill-9 watchdog so the script can never hang on docker.
#
# The watchdog POLLS rather than sleeping in a detached subshell: the old
# `( sleep "$secs"; kill -9 "$p" ) &` leaked its `sleep` child, which inherited
# the caller's stdout and held it open, so every `$(dout …)` blocked for the
# whole timeout even when docker had already answered (and a leaked sleep could
# later signal a recycled PID). Polling leaves nothing behind and returns as
# soon as the call does.
#
# DOCKER is a test seam: the self-test points it at a fake CLI, so the counting
# rules below are provable without a daemon and without touching real state.
DOCKER="${DOCKER:-docker}"
POLL=0.25   # seconds between liveness checks; the tick budget below assumes it
dout_status() { # dout_status <secs> <args…> — prints output, returns 124 on timeout
  local secs=$1; shift
  local f; f="$(mktemp)"
  "$DOCKER" "$@" >"$f" 2>/dev/null </dev/null &
  local p=$! ticks=0 limit=$((secs * 4))
  while kill -0 "$p" 2>/dev/null; do
    [[ $ticks -ge $limit ]] && break
    sleep "$POLL"
    ticks=$((ticks + 1))
  done
  local rc=0
  if kill -0 "$p" 2>/dev/null; then
    kill -9 "$p" 2>/dev/null
    rc=124
  else
    wait "$p" 2>/dev/null
    rc=$?
  fi
  cat "$f"; rm -f "$f"
  return "$rc"
}
dout() { dout_status "$@"; }
docker_ok() { [[ -n "$(dout 8 info)" ]]; }

# Which volumes `docker volume prune` (no flags) actually removes: the volumes
# docker created ANONYMOUSLY (for `docker run -v <path>`, or an image's VOLUME
# line) that no container is using. Docker marks those with the label
# `com.docker.volume.anonymous`, so presence of labels is the WRONG test — a
# compose volume has labels and is named, so prune keeps it, while an anonymous
# volume with that label is exactly what prune removes. A bare `docker volume
# create` volume has no labels at all and is still named, so it is kept too.
# The first version of this function tested "no labels and counted the two
# anonymous volumes a boot leaves behind as protected; the gate's real `--go`
# reclaimed them and showed the count was wrong.
# Never pass --all/-a to the prune: it would take the named dev state with it,
# including the marula-hq dev Postgres database (marula-hq-dev_pgdata).
volume_facts() { # volume_facts <secs> -> "prunable=<n> protected=<n> names=<…>"
  local secs="$1" vol labels unused prunable=0 protected=0 names=""
  unused="$(dout "$secs" volume ls -qf dangling=true)"
  for vol in $(dout "$secs" volume ls -q); do
    [[ -n "$vol" ]] || continue
    labels="$(dout "$secs" volume inspect "$vol" --format '{{.Labels}}' 2>/dev/null)"
    if [[ "$labels" == *com.docker.volume.anonymous* ]] && printf '%s\n' "$unused" | grep -qx "$vol"; then
      prunable=$((prunable + 1))
    else
      protected=$((protected + 1)); names="$names $vol"
    fi
  done
  printf 'prunable=%s protected=%s names=%s\n' "$prunable" "$protected" "${names# }"
}

# Prune with a budget a real backlog fits in, and say so when it did not finish.
prune_volumes() { # prune_volumes <seconds>
  local secs="$1" out rc
  out="$(dout_status "$secs" volume prune -f)"; rc=$?
  [[ -n "$out" ]] && printf '%s\n' "$out" | sed 's/^/  /'
  if [[ "$rc" -eq 124 ]]; then
    echo "  WARN volume prune was cut off by the ${secs}s watchdog and did not finish — the rest is still there, run scripts/dev-clean.sh --go again (nothing is lost)" >&2
  fi
  return "$rc"
}

dev_clean_selftest() {
  local tmp failures=0 checks=0 fake
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/dev-clean-selftest.XXXXXX")" || return 1
  # shellcheck disable=SC2064
  trap "rm -rf '$tmp'" EXIT
  fake="$tmp/docker"
  cat > "$fake" <<'FAKE'
#!/usr/bin/env bash
case "$1 ${2:-}" in
  "info "*)            echo "fake docker 1.0" ;;
  "system df")         printf 'TYPE            TOTAL     ACTIVE    SIZE      RECLAIMABLE\nImages          5         1         832.7MB   101.1MB (12%%)\nContainers      1         1         20.48kB   0B (0%%)\nLocal Volumes   9         1         646.1MB   470.4MB (72%%)\nBuild Cache     27        0         2.235GB   2.111GB\n' ;;
  "volume ls")         if [[ "${3:-}" == "-qf" || "$*" == *"dangling=true"* ]]; then printf 'boot_scratch\nnamed_no_labels\n'; else printf 'marula_pgdata\nmarula-hq-dev_pgdata\nmeridian_legacy\nnamed_no_labels\nboot_scratch\nheld_scratch\n'; fi ;;
  "volume inspect")    case "${3:-}" in
                         *boot_scratch|*held_scratch) echo "map[com.docker.volume.anonymous:]" ;;
                         *named_no_labels)            echo "map[]" ;;
                         *)                           echo "map[com.docker.compose.project:x]" ;;
                       esac ;;
  "volume prune")      if [[ "${FAKE_PRUNE_SLEEP:-0}" != "0" ]]; then sleep "$FAKE_PRUNE_SLEEP"; fi; echo "Total reclaimed space: 1.1GB" ;;
  "ps -a")             exit 0 ;;
  "rm -f")             exit 0 ;;
  *)                   exit 0 ;;
esac
FAKE
  chmod +x "$fake"
  export DOCKER="$fake"

  say() { echo "  $1"; }
  check() { # check <label> <condition-result>
    checks=$((checks + 1))
    if [[ "$2" == "0" ]]; then say "PASS  $1"; else say "FAIL  $1"; failures=$((failures + 1)); fi
  }

  echo "dev-clean self-test: the reclaimable figure must not promise labeled volumes"
  local facts
  facts="$(volume_facts 20)"
  check "counts only the unused anonymous volume as prunable" \
    "$([[ "$facts" == "prunable=1 protected=5"* ]] && echo 0 || echo 1)"
  check "names the protected volumes, HQ dev Postgres included" \
    "$([[ "$facts" == *marula-hq-dev_pgdata* ]] && echo 0 || echo 1)"

  # The dry run must not repeat docker's 470.4MB as a volume figure it can
  # deliver, and it must state the protected count.
  local dry
  dry="$(DOCKER="$fake" "$SELF" 2>&1 || true)"
  check "dry run reports the anonymous/protected split" \
    "$(printf '%s' "$dry" | grep -q "1 anonymous and unused" && echo 0 || echo 1)"
  check "dry run names the labeled dev state as protected" \
    "$(printf '%s' "$dry" | grep -q "protected:.*marula-hq-dev_pgdata" && echo 0 || echo 1)"
  check "dry run never offers docker's volume RECLAIMABLE figure as ours" \
    "$(printf '%s' "$dry" | grep -q "470" && echo 1 || echo 0)"
  check "an anonymous volume a container still holds stays out of the prunable set" \
    "$(printf '%s' "$facts" | grep -q "held_scratch" && echo 0 || echo 1)"
  check "a named volume with no labels is protected, not prunable" \
    "$(printf '%s' "$facts" | grep -q "named_no_labels" && echo 0 || echo 1)"
  check "the image figure is a size, not docker's percentage" \
    "$(printf '%s' "$dry" | grep -q "101.1MB" && ! printf '%s' "$dry" | grep -q "(12%)" && echo 0 || echo 1)"

  # A prune the watchdog cut off must say so; one that fits must stay quiet.
  local cut fits
  cut="$(FAKE_PRUNE_SLEEP=3 prune_volumes 1 2>&1)"; local cut_rc=$?
  check "truncated prune returns the watchdog code (124)" "$([[ "$cut_rc" -eq 124 ]] && echo 0 || echo 1)"
  check "truncated prune says it was cut off" "$(printf '%s' "$cut" | grep -q "WARN volume prune was cut off" && echo 0 || echo 1)"
  fits="$(FAKE_PRUNE_SLEEP=0 prune_volumes 20 2>&1)"; local fits_rc=$?
  check "a prune that fits returns 0" "$([[ "$fits_rc" -eq 0 ]] && echo 0 || echo 1)"
  check "a prune that fits warns about nothing" "$(printf '%s' "$fits" | grep -q "WARN" && echo 1 || echo 0)"

  if [[ "$failures" -eq 0 ]]; then
    echo "dev-clean self-test: $checks checks pass (fake docker, nothing removed)"
    return 0
  fi
  echo "dev-clean self-test: $failures of $checks failed"
  return 1
}


if [[ $SELFTEST -eq 1 ]]; then
  dev_clean_selftest
  exit $?
fi

before=$(df -g /System/Volumes/Data | tail -1 | awk '{print $4}')

targets=(
  # cargo incremental state — the single biggest silent grower (DX5)
  ".kilo/target-shared/debug/incremental"
  ".kilo/worktrees/*/target/debug/incremental"
  ".kilo/target-shared-*/debug/incremental"
  ".kilo/target-*/debug/incremental"
  ".kilo/worktrees/*/target-*/debug/incremental"
  # next.js dev/build caches — fully regenerable
  "apps/web/.next"
  ".kilo/worktrees/*/apps/web/.next"
)

echo "== dev-clean $( [[ $GO -eq 1 ]] && echo '(applying)' || echo '(dry run — pass --go to apply)' ) =="

for pattern in $targets; do
  for dir in $(eval ls -d "$ROOT/$pattern" 2>/dev/null); do
    size=$(du -sh "$dir" 2>/dev/null | cut -f1)
    echo "  $size  $dir"
    [[ $GO -eq 1 ]] && rm -rf "$dir"
  done
done

# graphify daily backups — keep the two most recent
keep=$(ls -1d "$ROOT"/graphify-out/2026-*(N) 2>/dev/null | sort | tail -2)
for dir in $(ls -1d "$ROOT"/graphify-out/2026-*(N) 2>/dev/null); do
  case "$keep" in *"$dir"*) ;; *)
    size=$(du -sh "$dir" 2>/dev/null | cut -f1)
    echo "  $size  $dir (old graphify backup)"
    [[ $GO -eq 1 ]] && rm -rf "$dir"
  ;; esac
done

if docker_ok; then
  stale=$(dout 10 ps -a --filter status=exited --format '{{.Names}}' | grep -E 'p9-|p10-|p11-|-tests-pg' 2>/dev/null)
  [[ -n "$stale" ]] && for c in $stale; do
    echo "  docker container: $c"
    [[ $GO -eq 1 ]] && dout 10 rm -f "$c" >/dev/null
  done
  if [[ $GO -eq 1 ]]; then
    # `docker volume prune` (no flags) removes UNLABELED volumes only, by
    # design: the labeled ones are deliberate dev state, including the marula-hq
    # dev Postgres database (marula-hq-dev_pgdata). Never pass --all here.
    # A real backlog can outrun a watchdog, so the budget is generous and a
    # prune that gets cut off says so instead of looking finished.
    prune_volumes "${PRUNE_TIMEOUT:-120}" || true
    dout 15 image prune -f >/dev/null
  else
    facts="$(volume_facts 8)"
    prunable="${facts#prunable=}"; prunable="${prunable%% *}"
    protected="${facts#*protected=}"; protected="${protected%% *}"
    protected_names="${facts#*names=}"
    # docker prints the reclaimable size and a percentage in parens when the size
    # is non-zero, and the size alone when it is zero — take the size either way
    # (the old `print $6` printed the percentage).
    i=$(dout 8 system df | awk '/^Images/{v=$NF; if (v ~ /^\(/) v=$(NF-1); print v}')
    echo "  docker volumes: ${prunable:-?} anonymous and unused (the only ones a volume prune removes), ${protected:-?} protected (named dev state, or an anonymous volume a container still holds)"
    [[ -n "$protected_names" ]] && echo "    protected: $protected_names"
    echo "  docker images reclaimable, docker's own count: ${i:-?} (an image prune removes dangling images only)"
  fi
fi

# deep sweep: the shared cargo cache's deps/ grows unbounded (parallel lanes
# duplicate workspace artifacts; incremental-only cleaning freed ~0 bytes and
# starved lanes looped on this script — 2026-09-10). Reclaiming it wholesale
# is safe outside an active build: cargo rebuilds what it needs on demand.
if [[ $DEEP -eq 1 ]]; then
  echo "== deep sweep =="
  deep_dirs=("$ROOT"/.kilo/target-shared/debug(N) "$ROOT"/.kilo/worktrees/*/target(N) "$ROOT"/.kilo/worktrees/*/target-*(N) "$ROOT"/.kilo/target-shared-*(N) "$ROOT"/.kilo/target-*(N))
  for dir in $deep_dirs; do
    echo "  $(du -sh "$dir" 2>/dev/null | cut -f1)  $dir"
  done
  if [[ $GO -eq 1 && ${#deep_dirs[@]} -gt 0 ]]; then
    # deleting under an active cargo build just fails — wait for a gap (15 min)
    i=0
    while pgrep -f "rustc|cargo-clippy|cargo (check|test|build|run)" >/dev/null 2>&1; do
      if [[ $i -ge 45 ]]; then
        echo "  no cargo build gap found in 15min — deep sweep aborted, re-run later"
        exit 1
      fi
      sleep 20; i=$((i+1))
    done
    for dir in $deep_dirs; do
      [[ -d "$dir" ]] && rm -rf "$dir"
    done
  fi
fi

# stale-artifact sweep: fine-grained GC of the shared cache (DX6c, 2026-09-17).
# Cargo keeps EVERY old artifact generation forever (11 libcore_api rlib
# generations and 132 incremental sessions were found at once) and rewrites
# ~8GB of test binaries + incremental state per full core-api cycle. Deleting
# only artifacts whose mtime is older than N days reclaims the generations the
# current tree no longer references while preserving today's warm cache.
if [[ $STALE -eq 1 ]]; then
  echo "== stale sweep (older than ${STALE_DAYS}d) =="
  stale_before=$(du -sh "$ROOT/.kilo/target-shared" 2>/dev/null | cut -f1)
  # deletion under an active build just fails — wait for a gap (15 min)
  if [[ $GO -eq 1 ]]; then
    i=0
    while pgrep -f "rustc|cargo-clippy|cargo (check|test|build|run)" >/dev/null 2>&1; do
      if [[ $i -ge 45 ]]; then
        echo "  no cargo build gap found in 15min — stale sweep aborted, re-run later"
        exit 1
      fi
      sleep 20; i=$((i+1))
    done
  fi
  # macOS find -mtime only takes whole days (+0 = 24h); -newermt gives an
  # exact cutoff (BSD find rejects the ISO "T" form — space-separated only).
  # Incremental is excluded — it has its own unconditional target above and
  # dies anyway with incremental disabled (DX6b).
  cutoff=$(date -v -"${STALE_DAYS}"d '+%Y-%m-%d %H:%M:%S' 2>/dev/null \
    || date -d "$STALE_DAYS days ago" '+%Y-%m-%d %H:%M:%S')
  found=$(find "$ROOT/.kilo/target-shared" ! -newermt "$cutoff" \
    ! -path '*/debug/incremental/*' ! -path '*/.rustc_info.json' \
    ! -path '*/debug/.fingerprint*' 2>/dev/null || true)
  total=0
  if [[ -n "$found" ]]; then
    while IFS= read -r f; do
      size=$(stat -f %z "$f" 2>/dev/null) || size=0
      total=$((total + size))
      [[ $GO -eq 1 ]] && rm -rf "$f"
    done <<< "$found"
    echo "  $(( $(wc -l <<< "$found") )) stale files, $(( total / 1024 / 1024 ))MB (cache was ${stale_before})"
  else
    echo "  nothing older than ${STALE_DAYS}d"
  fi
fi

after=$(df -g /System/Volumes/Data | tail -1 | awk '{print $4}')
echo "== free space: ${before}Gi -> ${after}Gi =="
[[ $GO -eq 0 ]] && echo "(dry run — run scripts/dev-clean.sh --go to apply)"
