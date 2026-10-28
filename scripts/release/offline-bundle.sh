#!/usr/bin/env bash
# offline-bundle.sh — the air-gapped install path for one platform.
#
# Input is a release directory that `scripts/release/assemble.mjs` produced.
# Output is one archive:
#
#   marula-<version>-offline-<os>-<arch>.tar.zst
#     marula-<version>-<os>-<arch>/   the whole release directory, unpacked
#     images.tar                     every image the compose file references
#     images.list                    their names, so the installer can check
#     install-offline.sh             loads the images, then `marula install --apply`
#     SHA256SUMS                     every file above, for `sha256sum -c`
#
# The image set is read from `docker compose config --images`, which is the
# only source that knows both the third-party images and the names compose
# derives for the services this repository builds. COMPOSE_PROJECT_NAME is
# pinned to `marula` for that call so the derived names are the same on every
# machine — `install-offline.sh` pins the same value, and that agreement is
# what stops compose from deciding to rebuild an image it cannot build.
#
#   scripts/release/offline-bundle.sh --version 0.3.1
#   scripts/release/offline-bundle.sh --version 0.3.1 --os linux --arch x64 \
#       --profile ai --skip-build
#
# Non-purpose: it publishes nothing and signs nothing. The archive is an asset
# the release job checksums and signs; this script only assembles it.
set -euo pipefail

SELF=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")
ROOT=$(cd "$(dirname "$SELF")/../.." && pwd)
cd "$ROOT"

VERSION=""
OS=""
ARCH=""
OUT=""
RELEASE_DIR=""
ENV_FILE="deploy/dev.env"
COMPOSE_FILE="deploy/docker-compose.yml"
SKIP_BUILD=0
PROFILES=()

say() { printf '==> %s\n' "$*"; }
die() { printf 'offline-bundle: %s\n' "$*" >&2; exit 1; }

while [ $# -gt 0 ]; do
  case "$1" in
    --version)
      VERSION="${2:-}"
      shift 2
      ;;
    --os)
      OS="${2:-}"
      shift 2
      ;;
    --arch)
      ARCH="${2:-}"
      shift 2
      ;;
    --out)
      OUT="${2:-}"
      shift 2
      ;;
    --release-dir)
      RELEASE_DIR="${2:-}"
      shift 2
      ;;
    --env-file)
      ENV_FILE="${2:-}"
      shift 2
      ;;
    --compose-file)
      COMPOSE_FILE="${2:-}"
      shift 2
      ;;
    --profile)
      PROFILES+=("${2:-}")
      shift 2
      ;;
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    -h | --help)
      awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$SELF"
      exit 0
      ;;
    *)
      die "unknown argument $1"
      ;;
  esac
done

[ -n "$VERSION" ] || die "--version is required"
[ -n "$OS" ] || OS=$(uname -s | tr '[:upper:]' '[:lower:]')
[ -n "$ARCH" ] || ARCH=$(uname -m)
case "$ARCH" in
  x86_64 | amd64 | x64) ARCH=x64 ;;
  aarch64 | arm64) ARCH=arm64 ;;
  *) die "unsupported architecture $ARCH" ;;
esac
case "$OS" in
  darwin) OS=macos ;;
  linux | macos | windows) ;;
  *) die "unsupported os $OS" ;;
esac
[ -n "$OUT" ] || OUT="dist/release"
[ -n "$RELEASE_DIR" ] || RELEASE_DIR="$OUT/marula-$VERSION-$OS-$ARCH"
[ -d "$RELEASE_DIR" ] || die "$RELEASE_DIR is not here; run scripts/release/assemble.mjs first"
[ -f "$RELEASE_DIR/SHA256SUMS" ] || die "$RELEASE_DIR/SHA256SUMS is not here"

for tool in docker tar zstd; do
  command -v "$tool" >/dev/null 2>&1 || die "$tool is not on PATH"
done

say "checking the release directory's own sums"
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$RELEASE_DIR" && sha256sum --quiet -c SHA256SUMS)
else
  (cd "$RELEASE_DIR" && shasum -a 256 --quiet -c SHA256SUMS)
fi

[ -f "$ENV_FILE" ] || die "$ENV_FILE is not here; it is what satisfies the compose file's required-variable guards. Pass --env-file with the eight values instead."
[ -f "$COMPOSE_FILE" ] || die "$COMPOSE_FILE is not here"

# One place decides the image names: compose itself, under a pinned project
# name. `--images` prints the third-party images as written and the derived
# name for every `build:` service.
COMPOSE_ARGS=(--env-file "$ENV_FILE" -f "$COMPOSE_FILE")
for profile in ${PROFILES[@]+"${PROFILES[@]}"}; do
  [ -n "$profile" ] && COMPOSE_ARGS+=(--profile "$profile")
done

if [ "$SKIP_BUILD" -eq 0 ]; then
  say "building the services this repository builds (this is the slow step)"
  COMPOSE_PROJECT_NAME=marula docker compose "${COMPOSE_ARGS[@]}" build
else
  say "skipping the build; the images must already be here"
fi

STAGING=$(mktemp -d)
trap 'rm -rf "$STAGING"' EXIT
BUNDLE="marula-$VERSION-offline-$OS-$ARCH"
BUNDLE_DIR="$STAGING/$BUNDLE"
mkdir -p "$BUNDLE_DIR"

say "collecting the image list"
COMPOSE_PROJECT_NAME=marula docker compose "${COMPOSE_ARGS[@]}" config --images |
  LC_ALL=C sort -u >"$STAGING/images.list"
[ -s "$STAGING/images.list" ] || die "compose reported no images; is $COMPOSE_FILE readable?"

IMAGES=()
while IFS= read -r line; do
  [ -n "$line" ] && IMAGES+=("$line")
done <"$STAGING/images.list"

unavailable=()
for image in "${IMAGES[@]}"; do
  if ! docker image inspect "$image" >/dev/null 2>&1; then
    unavailable+=("$image")
  fi
done
if [ "${#unavailable[@]}" -gt 0 ]; then
  printf 'offline-bundle: these images are not in the local store:\n' >&2
  printf '  %s\n' "${unavailable[@]}" >&2
  # shellcheck disable=SC2016  # the printed command is for the operator to run
  printf 'Build the ones compose can build (`docker compose %s build`) or pull the\n' "${COMPOSE_ARGS[*]}" >&2
  printf 'ones it cannot, then run this again without --skip-build.\n' >&2
  exit 1
fi

say "saving ${#IMAGES[@]} images"
docker save --output "$BUNDLE_DIR/images.tar" "${IMAGES[@]}"
cp "$STAGING/images.list" "$BUNDLE_DIR/images.list"

say "copying the release directory"
cp -R "$RELEASE_DIR" "$BUNDLE_DIR/marula-$VERSION-$OS-$ARCH"
cp "$ROOT/scripts/release/install-offline.sh" "$BUNDLE_DIR/install-offline.sh"
chmod 0755 "$BUNDLE_DIR/install-offline.sh"

say "writing SHA256SUMS"
# Compute the sums in the same shape `sha256sum -c` reads: sorted, two spaces,
# paths relative to this directory. `find` never yields SHA256SUMS because the
# name is excluded, which is the file being written beside it.
# shellcheck disable=SC2094
(
  cd "$BUNDLE_DIR"
  if command -v sha256sum >/dev/null 2>&1; then
    find . -type f ! -name SHA256SUMS -print0 |
      LC_ALL=C sort -z |
      xargs -0 sha256sum |
      sed 's|  \./|  |' >SHA256SUMS
    sha256sum --quiet -c SHA256SUMS
  else
    find . -type f ! -name SHA256SUMS -print0 |
      LC_ALL=C sort -z |
      xargs -0 shasum -a 256 |
      sed 's|  \./|  |' >SHA256SUMS
    shasum -a 256 -c SHA256SUMS >/dev/null
  fi
)

mkdir -p "$OUT"
ARCHIVE="$OUT/$BUNDLE.tar.zst"
say "writing $ARCHIVE"
rm -f "$ARCHIVE"
tar -cf - -C "$STAGING" "$BUNDLE" | zstd -T0 -19 -q -o "$ARCHIVE"

say "done"
printf '  %s\n  %s bytes\n  verify with: tar --zstd -tf %s\n' \
  "$ARCHIVE" "$(wc -c <"$ARCHIVE" | tr -d ' ')" "$ARCHIVE"
