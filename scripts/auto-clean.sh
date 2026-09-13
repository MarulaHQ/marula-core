#!/usr/bin/env zsh
# auto-clean.sh — threshold-triggered disk hygiene (runs from the LaunchAgent
# com.meridian.dev-clean.plist; safe to run by hand).
#
#   free space >= SOFT (60Gi): no-op
#   free space <  SOFT:        scripts/dev-clean.sh --go   (caches, incremental, docker junk)
#   free space <  HARD (40Gi): additionally --deep         (shared cargo deps; waits for
#                                                           a cargo build gap)
#
# This is deliberately gated on free space, not a fixed schedule of deletion:
# dev-clean.sh must not starve active lanes by looping (2026-09-10 incident),
# so we only ever prune when the disk actually needs it.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LOG="$HOME/.cache/meridian-auto-clean.log"
SOFT=60
HARD=40

mkdir -p "$(dirname "$LOG")"
# one instance at a time (mkdir is an atomic lock; macOS has no flock(1))
lockdir="$LOG.lock"
if ! mkdir "$lockdir" 2>/dev/null; then
  # stale lock from a crashed run older than 2h is broken
  if [[ -n "$(find "$lockdir" -maxdepth 0 -mmin +120 2>/dev/null)" ]]; then
    rm -rf "$lockdir"; mkdir "$lockdir" || exit 0
  else
    exit 0
  fi
fi
trap 'rm -rf "$lockdir"' EXIT

free=$(df -g /System/Volumes/Data | tail -1 | awk '{print $4}')
stamp="$(date '+%Y-%m-%d %H:%M:%S')"

if (( free >= SOFT )); then
  exit 0
fi

if (( free < HARD )); then
  echo "[$stamp] ${free}Gi free (< ${HARD}Gi hard) — deep clean" >>"$LOG"
  "$ROOT/scripts/dev-clean.sh" --go --deep >>"$LOG" 2>&1
else
  echo "[$stamp] ${free}Gi free (< ${SOFT}Gi soft) — light clean" >>"$LOG"
  "$ROOT/scripts/dev-clean.sh" --go >>"$LOG" 2>&1
fi

after=$(df -g /System/Volumes/Data | tail -1 | awk '{print $4}')
echo "[$stamp] done: ${free}Gi -> ${after}Gi" >>"$LOG"
# keep the log itself bounded
tail -n 200 "$LOG" > "$LOG.tmp" && mv "$LOG.tmp" "$LOG"
