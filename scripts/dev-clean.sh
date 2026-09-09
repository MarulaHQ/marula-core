#!/usr/bin/env zsh
# dev-clean.sh — disk-space housekeeping (TASKLIST DX5).
#
#   scripts/dev-clean.sh          dry-run: list what would be cleared
#   scripts/dev-clean.sh --go     actually clear it
#
# Safe-by-construction: never touches running dev servers' source trees,
# gitignored-but-live node_modules of active lanes, or the shared cargo
# cache's real build artifacts (only its incremental state).
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GO=0
[[ "${1:-}" == "--go" ]] && GO=1

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
keep=$(ls -1d "$ROOT"/graphify-out/2026-* 2>/dev/null | sort | tail -2)
for dir in $(ls -1d "$ROOT"/graphify-out/2026-* 2>/dev/null); do
  case "$keep" in *"$dir"*) ;; *)
    size=$(du -sh "$dir" 2>/dev/null | cut -f1)
    echo "  $size  $dir (old graphify backup)"
    [[ $GO -eq 1 ]] && rm -rf "$dir"
  ;; esac
done

# docker: exited containers from merged lanes, orphaned volumes, dangling images
if docker info >/dev/null 2>&1; then
  # only EXITED lane containers — running ones belong to active lanes
  stale=$(docker ps -a --filter status=exited --format '{{.Names}}' | grep -E 'p9-|p10-|p11-|-tests-pg' 2>/dev/null)
  [[ -n "$stale" ]] && for c in $stale; do
    echo "  docker container: $c"
    [[ $GO -eq 1 ]] && docker rm -f "$c" >/dev/null
  done
  if [[ $GO -eq 1 ]]; then
    docker volume prune -f >/dev/null 2>&1
    docker image prune -f >/dev/null 2>&1
  else
    v=$(docker system df 2>/dev/null | awk '/Local Volumes/{print $6}')
    i=$(docker system df 2>/dev/null | awk '/Images/{print $6}')
    echo "  docker reclaimable: volumes ${v:-?}, images ${i:-?}"
  fi
fi

after=$(df -g /System/Volumes/Data | tail -1 | awk '{print $4}')
echo "== free space: ${before}Gi -> ${after}Gi =="
[[ $GO -eq 0 ]] && echo "(dry run — run scripts/dev-clean.sh --go to apply)"
