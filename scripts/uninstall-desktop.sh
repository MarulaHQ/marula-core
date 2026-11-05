#!/usr/bin/env bash
# Remove the Marula desktop client from this machine.
#
# Two halves, and they are separate on purpose:
#
#   1. the app bundle — what a person means by "uninstall";
#   2. the data dir — the embedded database, `desktop-secret`, `printers.json`,
#      `connection.json` and `window-state.json`.
#
# The second half is only removed with `--purge`, and only after the script
# says what it holds. Deleting a database is not a side effect of tidying an
# Applications folder, and a connected client's only local state is one address,
# which the person may well want to keep.
#
# Dry run by default, like scripts/dev-clean.sh: run it, read the plan, then
# pass --go.
#
#   scripts/uninstall-desktop.sh                     # what would be removed
#   scripts/uninstall-desktop.sh --go                # remove the app bundle
#   scripts/uninstall-desktop.sh --purge --go        # and the data dir
#   scripts/uninstall-desktop.sh --app dist/Marula.app --go
set -euo pipefail

DATA_DIR_MACOS="$HOME/Library/Application Support/Marula"
DATA_DIR_OTHER="$HOME/.local/share/Marula"
BUNDLE_ID="org.marula.desktop"

APPLY=0
PURGE=0
APPS=()

usage() {
  cat <<'USAGE'
Usage: scripts/uninstall-desktop.sh [--app <path>] [--purge] [--go]

  --app <path>  Remove this bundle in addition to the default locations.
  --purge       Also remove the app data dir (the embedded database, the
                install secret, the printer list, the saved connection).
  --go          Apply. Without it the script prints the plan and stops.
  --help        Print this.
USAGE
}

while [ $# -gt 0 ]; do
  case "$1" in
    --app)
      [ $# -ge 2 ] || { echo "error: --app needs a path" >&2; exit 2; }
      APPS+=("$2")
      shift 2
      ;;
    --purge) PURGE=1; shift ;;
    --go) APPLY=1; shift ;;
    --help|-h) usage; exit 0 ;;
    *) echo "error: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [ "$(uname -s)" = "Darwin" ]; then
  DATA_DIR="$DATA_DIR_MACOS"
else
  DATA_DIR="$DATA_DIR_OTHER"
fi

# The bundle's own identifier is the only proof that a `.app` is ours. A path
# that does not carry it is named and skipped, never removed: an uninstaller
# that deletes by path alone is one typo away from someone's other app.
is_marula_bundle() {
  local app="$1" plist identifier=""
  [ -d "$app" ] || return 1
  plist="$app/Contents/Info.plist"
  [ -f "$plist" ] || return 1
  if command -v plutil >/dev/null 2>&1; then
    identifier=$(plutil -extract CFBundleIdentifier raw -o - "$plist" 2>/dev/null || true)
  fi
  if [ -z "$identifier" ]; then
    identifier=$(grep -A1 'CFBundleIdentifier' "$plist" 2>/dev/null | grep -o 'org\.marula\.[a-z.]*' | head -1 || true)
  fi
  [ "$identifier" = "$BUNDLE_ID" ]
}

# Default locations: the installed app, and the repo-local build the desktop
# script leaves in dist/.
DEFAULT_APPS=("/Applications/Marula.app" "$HOME/Applications/Marula.app")
REPO_ROOT=$(cd "$(dirname "$0")/.." && pwd)
[ -d "$REPO_ROOT/dist/Marula.app" ] && DEFAULT_APPS+=("$REPO_ROOT/dist/Marula.app")

FOUND=()
for app in "${DEFAULT_APPS[@]}" "${APPS[@]:-}"; do
  [ -n "$app" ] || continue
  case "$app" in
    /*) ;;
    *) app="$PWD/$app" ;;
  esac
  if is_marula_bundle "$app"; then
    FOUND+=("$app")
  elif [ -e "$app" ]; then
    echo "skip $app — not a Marula app bundle (identifier is not $BUNDLE_ID)"
  fi
done

if [ "${#FOUND[@]}" -eq 0 ] && [ "$PURGE" -eq 0 ]; then
  echo "Nothing to remove: no Marula.app found in the default locations, and --purge was not given."
  echo "Pass --app <path> if the bundle lives somewhere else."
  exit 0
fi

echo "Marula desktop uninstall"
echo
if [ "${#FOUND[@]}" -gt 0 ]; then
  echo "App bundles:"
  for app in "${FOUND[@]}"; do
    echo "  rm -rf \"$app\""
  done
else
  echo "App bundles: none found"
fi

if [ "$PURGE" -eq 1 ]; then
  echo
  if [ -d "$DATA_DIR" ]; then
    SIZE=$(du -sh "$DATA_DIR" 2>/dev/null | cut -f1)
    echo "App data ($SIZE): $DATA_DIR"
    for entry in connection.json desktop-secret printers.json window-state.json pgdata; do
      [ -e "$DATA_DIR/$entry" ] && echo "  $entry"
    done
    echo "  rm -rf \"$DATA_DIR\""
    echo
    echo "This removes the embedded database and the install secret. Sealed rows"
    echo "written under that secret cannot be opened by a later install."
  else
    echo "App data: none at $DATA_DIR"
  fi
fi

if [ "$APPLY" -eq 0 ]; then
  echo
  echo "Dry run. Nothing was removed. Re-run with --go to apply."
  exit 0
fi

for app in "${FOUND[@]:-}"; do
  [ -n "$app" ] || continue
  rm -rf "$app"
  echo "removed $app"
done

if [ "$PURGE" -eq 1 ] && [ -d "$DATA_DIR" ]; then
  rm -rf "$DATA_DIR"
  echo "removed $DATA_DIR"
fi

echo "done"
