#!/usr/bin/env bash
# install.sh — install Marula from the release bundle that carries this script.
#
# The bundle is the whole distribution: the `marula` CLI, the wizard front-end,
# the compose files, and a checksum list over all of it. This script checks that
# list, confirms Docker is here, and hands every decision to the CLI's installer,
# which measures the host, writes the env file and starts the stack.
#
#   ./install.sh                                  # the wizard, installing into ~/marula
#   ./install.sh --dir /srv/marula                # a server: print the plan, write nothing
#   ./install.sh --dir /srv/marula --apply        # write the env file and start the stack
#   ./install.sh --dir /srv/marula --profile all-in-one
#
# Anything this script does not recognise is passed to `marula install`, so the
# CLI's own flags are the documentation for the rest.
#
# Non-purpose: it does not download anything, verify a release signature, or
# install Docker. The signature over the archive is checked where the archive was
# fetched (see docs/install.md), and Docker is a prerequisite the operator
# brings.
set -euo pipefail

SELF=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")
HERE=$(dirname "$SELF")
cd "$HERE"

say() { printf '==> %s\n' "$*"; }
die() { printf 'install: %s\n' "$*" >&2; exit 1; }

INSTALL_DIR="${MARULA_INSTALL_DIR:-$HOME/marula}"
ARGS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --dir)
      # Kept out of ARGS: this script passes --dir itself, and two copies of the
      # same flag would make the CLI refuse the whole command.
      [ $# -ge 2 ] || die "--dir needs a value"
      INSTALL_DIR="$2"
      shift 2
      ;;
    --compose-file | --ui-dir)
      # Both are fixed by the bundle. Taking another one here would describe a
      # stack whose files this bundle does not carry.
      die "$1 is fixed by the bundle; unpack the archive whole and run this script from it"
      ;;
    -h | --help)
      awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$SELF"
      exit 0
      ;;
    *)
      ARGS+=("$1")
      shift
      ;;
  esac
done

command -v docker >/dev/null 2>&1 || die "docker is not on PATH; this bundle needs it"
docker compose version >/dev/null 2>&1 ||
  die "the docker compose plugin is not available; see https://docs.docker.com/compose/install/"

[ -x "$HERE/marula" ] || die "$HERE/marula is missing; unpack the whole archive"
[ -f "$HERE/deploy/docker-compose.yml" ] || die "$HERE/deploy/docker-compose.yml is missing; unpack the whole archive"
[ -d "$HERE/apps/installer/dist" ] || die "$HERE/apps/installer/dist is missing; unpack the whole archive"

say "checking the bundle against its own checksums"
if command -v sha256sum >/dev/null 2>&1; then
  sha256sum --quiet -c SHA256SUMS
else
  shasum -a 256 --quiet -c SHA256SUMS
fi

# With no arguments at all, open the wizard: that is the laptop path, and asking
# someone to learn a flag before they have seen the product is the friction this
# script exists to remove. Any flag they pass goes to `marula install` untouched.
if [ "${#ARGS[@]}" -eq 0 ] && [ "$INSTALL_DIR" = "$HOME/marula" ]; then
  ARGS=(--ui)
fi

say "installing into $INSTALL_DIR"
exec "$HERE/marula" install \
  --compose-file "$HERE/deploy/docker-compose.yml" \
  --ui-dir "$HERE/apps/installer/dist" \
  --dir "$INSTALL_DIR" \
  ${ARGS[@]+"${ARGS[@]}"}
