#!/usr/bin/env bash
# one-line-e2e.sh — install Marula from the advertised URL, the way the README
# says to, and check the result with the bundle it brought down.
#
# What this proves that install-e2e.sh cannot: that script starts from an
# unpacked bundle, so the download, the redirect and the script's own checks are
# the caller's business. Readers start from a URL. This leg takes one, runs it
# exactly as the page shows, and then asks the installed stack whether it works.
#
# Non-purpose: the wizard. `--apply` is passed because a runner has no terminal;
# the interactive path belongs to the installer's own tests. The capacity waiver
# is passed for the reason install-e2e.sh states: a runner's free disk is not
# guaranteed past split's floor, and the floors themselves are covered by the
# doctor's unit tests.
#
# Usage: one-line-e2e.sh <url> <empty-install-dir> [expected-version]

set -euo pipefail

say() { printf '==> %s\n' "$*"; }
die() { printf 'one-line-e2e: %s\n' "$*" >&2; exit 1; }

USAGE='usage: one-line-e2e.sh <url> <empty-install-dir> [expected-version]'
URL=${1:-}
INSTALL_DIR=${2:-}
EXPECT=${3:-}
[ -n "$URL" ] && [ -n "$INSTALL_DIR" ] || die "$USAGE"

mkdir -p "$INSTALL_DIR"
INSTALL_DIR=$(cd "$INSTALL_DIR" && pwd)

SCRIPT=$(mktemp)
trap 'rm -f "$SCRIPT"' EXIT

say "fetching $URL"
# Written to a file before it is run: the leg exists to test the front door, and
# a pipe would hide a 404 behind a shell that exits 0 having run nothing.
curl -fsSL --retry 3 --retry-delay 2 -o "$SCRIPT" "$URL"
[ -s "$SCRIPT" ] || die "$URL served an empty body"
sh -n "$SCRIPT" || die "what $URL served is not a shell script"
say "the URL served a shell script ($(wc -c <"$SCRIPT" | tr -d ' ') bytes)"

if [ -n "$EXPECT" ]; then
  # The URL and the asset have to name the same release. `latest` resolves
  # through GitHub's redirect to whichever tag is newest, so this is the check
  # that the redirect, the release page and the script's own default agree.
  grep -Fq "MARULA_VERSION:-$EXPECT" "$SCRIPT" ||
    die "the script from $URL does not pin $EXPECT: the redirect and the asset disagree"
  say "the script pins $EXPECT"
fi

say "running it the way the README does"
sh "$SCRIPT" --dir "$INSTALL_DIR" --images registry --profile split --apply --allow-unfit

BUNDLE_ROOT="${MARULA_BUNDLE_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/marula}"
shopt -s nullglob
RELEASES=("$BUNDLE_ROOT"/marula-"${EXPECT:-*}"-linux-*)
shopt -u nullglob
[ "${#RELEASES[@]}" -eq 1 ] ||
  die "expected one release directory under $BUNDLE_ROOT, found ${#RELEASES[@]}"
ROOT=${RELEASES[0]}
[ -x "$ROOT/marula" ] || die "$ROOT/marula is not executable: the bundle did not survive unpacking"
say "the bundle stayed at $ROOT"

# The installer writes its overrides beside `marula.env`, and every compose call
# below names the ones that exist, so a teardown cannot name a file that was
# never written.
OVERLAYS=()
for overlay in "$INSTALL_DIR/marula.images.yml" "$INSTALL_DIR/marula.restart.yml"; do
  [ -f "$overlay" ] && OVERLAYS+=(-f "$overlay")
done

teardown() {
  for env_file in "$INSTALL_DIR/marula.env" "$INSTALL_DIR/compose.env"; do
    [ -f "$env_file" ] || continue
    say "stopping the stack that used $env_file"
    docker compose --env-file "$env_file" \
      -f "$ROOT/deploy/docker-compose.yml" \
      ${OVERLAYS[@]+"${OVERLAYS[@]}"} down -v >/dev/null 2>&1 || true
  done
}

say "checking the result"
"$ROOT/marula" doctor \
  --dir "$INSTALL_DIR" \
  --profile split \
  --compose-file "$ROOT/deploy/docker-compose.yml" \
  --allow-unfit

say "the API answers on its published port"
ready=0
for _ in $(seq 1 30); do
  if curl -fsS --max-time 2 http://127.0.0.1:8080/readyz >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 2
done
[ "$ready" -eq 1 ] ||
  die "the API did not answer /readyz on 127.0.0.1:8080; the one-liner reported success"
echo "  /readyz ok"

teardown
say "one line from $URL installed a working stack"
