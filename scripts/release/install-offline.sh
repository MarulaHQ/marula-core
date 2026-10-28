#!/usr/bin/env bash
# install-offline.sh — the air-gapped install, from the bundle it ships in.
#
# This file is copied into `marula-<version>-offline-<os>-<arch>.tar.zst` by
# `scripts/release/offline-bundle.sh`. On a host with no route to a registry,
# it checks the bundle's own sums, loads the images the bundle carries, and
# then hands the rest to the CLI (`marula install --apply`), which writes the
# env file and starts the stack from the compose file beside this script.
#
#   ./install-offline.sh                      # install into $HOME/marula
#   ./install-offline.sh --dir /srv/marula    # anywhere else
#   ./install-offline.sh --profile all-in-one --set MARULA_EMBEDDING_PROVIDER=openai
#
# Two things make the "no build" promise true, and both are load-bearing:
#
#   * COMPOSE_PROJECT_NAME is pinned to `marula`, because compose derives the
#     image name of a `build:` service from the project name. The bundle saved
#     its images under those derived names; a different project name would make
#     compose look for images that are not here and start BUILDING, which is
#     the one thing an air-gapped host cannot do.
#   * every image is inspected before `marula install` runs, so a bundle that
#     lost an image fails here, by name, instead of half-booting a stack.
#
# Non-purpose: it does not fetch anything, verify a signature, or install
# Docker. The signature over the release is verified where the bundle was
# fetched; Docker is a prerequisite the operator brings.
set -euo pipefail

SELF=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")
HERE=$(dirname "$SELF")
cd "$HERE"

say() { printf '==> %s\n' "$*"; }
die() { printf 'install-offline: %s\n' "$*" >&2; exit 1; }

INSTALL_DIR="${MARULA_INSTALL_DIR:-$HOME/marula}"
ARGS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --dir)
      # Kept out of ARGS: this script passes --dir itself, and two copies of
      # the same flag would make the CLI refuse the whole command.
      [ $# -ge 2 ] || die "--dir needs a value"
      INSTALL_DIR="$2"
      shift 2
      ;;
    --compose-file)
      # The compose file is the bundle's own, beside this script. Taking another
      # one here would describe a stack whose images this bundle does not carry.
      die "--compose-file is fixed by the bundle; use --profile to pick a shape"
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

# The release directory inside the bundle is the only one that looks like this.
shopt -s nullglob
RELEASES=("$HERE"/marula-[0-9]*-*)
shopt -u nullglob
[ "${#RELEASES[@]}" -eq 1 ] || die "expected exactly one marula-<version>-<os>-<arch> directory here, found ${#RELEASES[@]}"
RELEASE="${RELEASES[0]}"
BIN="$RELEASE/marula"
[ -x "$BIN" ] || die "$BIN is missing or not executable"

say "checking the bundle's own sums"
if command -v sha256sum >/dev/null 2>&1; then
  sha256sum --quiet -c SHA256SUMS
else
  shasum -a 256 --quiet -c SHA256SUMS
fi

say "loading the images this bundle carries"
docker load --input images.tar >/dev/null

missing=()
while IFS= read -r image; do
  [ -n "$image" ] || continue
  if ! docker image inspect "$image" >/dev/null 2>&1; then
    missing+=("$image")
  fi
done <images.list
if [ "${#missing[@]}" -gt 0 ]; then
  printf 'install-offline: these images are not loaded and cannot be built offline:\n' >&2
  printf '  %s\n' "${missing[@]}" >&2
  exit 1
fi
say "every image in images.list is loaded"

# See the header: the derived image names depend on this value.
export COMPOSE_PROJECT_NAME=marula

say "installing into $INSTALL_DIR"
mkdir -p "$INSTALL_DIR"
"$BIN" install --apply \
  --compose-file "$RELEASE/deploy/docker-compose.yml" \
  --dir "$INSTALL_DIR" \
  ${ARGS[@]+"${ARGS[@]}"}

say "checking the result"
"$BIN" doctor --compose-file "$RELEASE/deploy/docker-compose.yml" --dir "$INSTALL_DIR"
