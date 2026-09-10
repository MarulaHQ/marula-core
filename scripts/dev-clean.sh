#!/usr/bin/env zsh
# dev-clean.sh — disk-space housekeeping (TASKLIST DX5).
#
#   scripts/dev-clean.sh              dry-run: list what would be cleared
#   scripts/dev-clean.sh --go         actually clear it
#   scripts/dev-clean.sh --go --deep  ALSO reclaim the shared cargo cache's
#                                     full build artifacts + drifted worktree
#                                     target dirs (waits for a cargo build gap
#                                     first; cold-cache rebuild follows)
#
# Safe-by-construction: never touches running dev servers' source trees,
# gitignored-but-live node_modules of active lanes, or the shared cargo
# cache's real build artifacts (only its incremental state) unless --deep.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GO=0
DEEP=0
for arg in "$@"; do
  case "$arg" in
    --go) GO=1 ;;
    --deep) DEEP=1 ;;
  esac
done

before=$(df -g /System/Volumes/Data | tail -1 | awk '{print $4}')

targets=(
  # cargo incremental state — the single biggest silent grower (DX5)
  ".kilo/target-shared/debug/incremental"
  ".kilo/worktrees/*/target/debug/incremental"
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

# docker: exited containers from merged lanes, orphaned volumes, dangling images
# The docker CLI hangs indefinitely when the daemon is wedged (2026-09-10) and
# ignores the alarm-exec trick (Go runtime owns SIGALRM) — every call runs
# under a kill-9 watchdog so the script can never hang on docker.
dout() {
  local secs=$1; shift
  local f; f="$(mktemp)"
  docker "$@" >"$f" 2>/dev/null &
  local p=$!
  ( sleep "$secs"; kill -9 "$p" 2>/dev/null ) &
  local w=$!
  wait "$p" 2>/dev/null
  kill "$w" 2>/dev/null
  cat "$f"; rm -f "$f"
}
docker_ok() { [[ -n "$(dout 8 info)" ]] }

if docker_ok; then
  stale=$(dout 10 ps -a --filter status=exited --format '{{.Names}}' | grep -E 'p9-|p10-|p11-|-tests-pg' 2>/dev/null)
  [[ -n "$stale" ]] && for c in $stale; do
    echo "  docker container: $c"
    [[ $GO -eq 1 ]] && dout 10 rm -f "$c" >/dev/null
  done
  if [[ $GO -eq 1 ]]; then
    dout 15 volume prune -f >/dev/null
    dout 15 image prune -f >/dev/null
  else
    v=$(dout 8 system df | awk '/Local Volumes/{print $6}')
    i=$(dout 8 system df | awk '/Images/{print $6}')
    echo "  docker reclaimable: volumes ${v:-?}, images ${i:-?}"
  fi
fi

# deep sweep: the shared cargo cache's deps/ grows unbounded (parallel lanes
# duplicate workspace artifacts; incremental-only cleaning freed ~0 bytes and
# starved lanes looped on this script — 2026-09-10). Reclaiming it wholesale
# is safe outside an active build: cargo rebuilds what it needs on demand.
if [[ $DEEP -eq 1 ]]; then
  echo "== deep sweep =="
  deep_dirs=("$ROOT"/.kilo/target-shared/debug(N) "$ROOT"/.kilo/worktrees/*/target(N) "$ROOT"/.kilo/worktrees/*/target-*(N))
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

after=$(df -g /System/Volumes/Data | tail -1 | awk '{print $4}')
echo "== free space: ${before}Gi -> ${after}Gi =="
[[ $GO -eq 0 ]] && echo "(dry run — run scripts/dev-clean.sh --go to apply)"
