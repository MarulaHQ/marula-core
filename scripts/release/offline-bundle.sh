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
# The compose file is the release's own, and its image values are written out in
# full: a value Compose interpolates (${APP_IMAGE}) is resolved for the image list
# but cannot be rewritten in the file, so the bundle check refuses that shape
# rather than ship one that names its images two ways.
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

# The sums, in the shape `sha256sum -c` reads: sorted, two spaces, paths relative
# to the directory. `find` never yields SHA256SUMS because the name is excluded,
# which is the file being written beside it.
write_sums() {
  local dir=$1
  # shellcheck disable=SC2094  # SHA256SUMS is excluded from its own input
  (
    cd "$dir"
    if command -v sha256sum >/dev/null 2>&1; then
      find . -type f ! -path './SHA256SUMS' -print0 |
        LC_ALL=C sort -z |
        xargs -0 sha256sum |
        sed 's|  \./|  |' >SHA256SUMS
      sha256sum --quiet -c SHA256SUMS
    else
      find . -type f ! -path './SHA256SUMS' -print0 |
        LC_ALL=C sort -z |
        xargs -0 shasum -a 256 |
        sed 's|  \./|  |' >SHA256SUMS
      shasum -a 256 -c SHA256SUMS >/dev/null
    fi
  )
}

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

# Compose builds the `build:` services and nothing else, so the third-party
# images the file references (the database, the cache, the pooler) are never in
# the store unless something pulled them. `--ignore-buildable` asks for exactly
# those, and it is what turns "these images are not in the local store" into a
# bundle that carries what it claims to. The rehearsal on 2026-11-19 reached this
# script for the first time and died on the three it had never pulled.
say "pulling the images compose references but does not build"
# The pooler is opt-in in the base file, so it only appears in the list when the
# profile is selected. A bundle that omits it leaves an air-gapped host unable to
# turn the profile on later.
COMPOSE_PROJECT_NAME=marula docker compose "${COMPOSE_ARGS[@]}" --profile pooler pull --ignore-buildable

STAGING=$(mktemp -d)
trap 'rm -rf "$STAGING"' EXIT
BUNDLE="marula-$VERSION-offline-$OS-$ARCH"
BUNDLE_DIR="$STAGING/$BUNDLE"
mkdir -p "$BUNDLE_DIR"

say "collecting the image list"
COMPOSE_PROJECT_NAME=marula docker compose "${COMPOSE_ARGS[@]}" --profile pooler config --images |
  LC_ALL=C sort -u >"$STAGING/images.refs"
[ -s "$STAGING/images.refs" ] || die "compose reported no images; is $COMPOSE_FILE readable?"

# What the bundle calls each image. A digest survives `docker save`/`docker load`
# only on a store that keeps repo digests (the containerd image store does; the
# overlay2 driver most hosts and CI use does not), and an image that is saved by
# its digest reference carries no tag in the tar at all: it comes back with no
# name, so neither `name:tag@digest` nor `name:tag` finds it. The tag is the only
# name that survives, so the bundle names every image by its tag, saves it under
# that tag, and the compose file beside it is rewritten to match. What the digest
# pinned here is the bundle's own SHA256SUMS, which covers the tar and is signed
# at release level; inside a bundle the digest buys nothing that check does not.
#
# The rule is narrow on purpose: a reference with no tag (`prom/prometheus@sha256:…`)
# is left alone, because dropping its digest would rewrite it to `:latest`, a
# different image.
bundle_name() {
  local ref=$1 name=${1%@*}
  case "${name##*/}" in
    *:*) printf '%s\n' "$name" ;;
    *) return 1 ;;
  esac
}

# The name Docker writes for an image, which is not always the one Compose spells.
# The classic exporter records `docker.io/library/busybox:1.36` as
# `busybox:1.36`, and it records a name with no tag as `:latest`. Comparing the
# two spellings is what keeps a correct archive from failing a name check.
normalise_ref() {
  local ref=$1
  case "$ref" in
    docker.io/library/*) ref=${ref#docker.io/library/} ;;
    index.docker.io/library/*) ref=${ref#index.docker.io/library/} ;;
    docker.io/*) ref=${ref#docker.io/} ;;
    index.docker.io/*) ref=${ref#index.docker.io/} ;;
    library/*) ref=${ref#library/} ;;
  esac
  case "${ref##*/}" in
    *:*) ;;
    *) ref="$ref:latest" ;;
  esac
  printf '%s\n' "$ref"
}

REFS=()
IMAGES=()
while IFS= read -r ref; do
  [ -n "$ref" ] || continue
  REFS+=("$ref")
  if name=$(bundle_name "$ref"); then
    IMAGES+=("$name")
  else
    case "$ref" in
      *@sha256:*)
        # It pins a digest and carries no tag, so nothing in a tar can name it:
        # load would restore an image with no name, and the install could not
        # find it by any. Refuse here, where the fix is a tag in the compose file.
        die "$ref pins a digest and carries no tag, so the bundle cannot name it: give it a tag in $COMPOSE_FILE"
        ;;
    esac
    IMAGES+=("$ref")
  fi
done <"$STAGING/images.refs"

# Two references that derive one name would overwrite each other at the save, and
# the bundle would carry one image where the compose file names two.
for index in "${!IMAGES[@]}"; do
  for earlier in "${!IMAGES[@]}"; do
    [ "$earlier" -lt "$index" ] || break
    if [ "${IMAGES[$earlier]}" = "${IMAGES[$index]}" ] &&
      [ "${REFS[$earlier]}" != "${REFS[$index]}" ]; then
      die "${IMAGES[$index]} would name two different images (${REFS[$earlier]} and ${REFS[$index]}); they need distinct tags"
    fi
  done
done

unavailable=()
for index in "${!IMAGES[@]}"; do
  image=${IMAGES[$index]}
  # Give the image the name the bundle will carry, so `docker save` writes it and
  # `docker load` restores it. A reference that already reads as its own bundle
  # name (a locally built `marula-core-api`) needs nothing.
  #
  # A tag that already exists locally is retargeted on purpose. The digest is what
  # the compose file names, so it is what the bundle must carry, and a stale local
  # tag from an earlier pull must not win; the inspect below then asks for the name
  # the bundle will use, which is that image.
  if [ "${REFS[$index]}" != "$image" ]; then
    # Fatal, not a note: if the name is already taken by something else, the
    # inspect below would pass on that image and the bundle would carry it.
    docker tag "${REFS[$index]}" "$image" >/dev/null 2>&1 ||
      die "could not tag ${REFS[$index]} as $image"
  fi
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
# Sorted after the digest is gone, not before: `app:1@sha256:a` sorts before
# `app:1-a@sha256:b` but the names it becomes sort the other way, and the compose
# check below compares the two lists line for line.
printf '%s\n' "${IMAGES[@]}" | LC_ALL=C sort -u >"$BUNDLE_DIR/images.list"

say "copying the release directory"
cp -R "$RELEASE_DIR" "$BUNDLE_DIR/marula-$VERSION-$OS-$ARCH"
cp "$ROOT/scripts/release/install-offline.sh" "$BUNDLE_DIR/install-offline.sh"
chmod 0755 "$BUNDLE_DIR/install-offline.sh"

# The compose file in the bundle has to name the images the way the bundle made
# them. Both groups are needed to tell a tagged reference from a naked digest: a
# registry host may carry a port (`localhost:5005/thing:1.0`), and a reference
# with no tag has nothing to fall back on, so it is left exactly as it was.
# Every tagged digest reference in the file becomes its tag. The rule is not
# anchored on `image:` because a value may be quoted, or an alias whose anchor is
# defined under another key, and either would leave the file naming one image
# while images.list names another. A reference with no tag does not match, which
# is deliberate: dropping its digest would silently mean `:latest`.
say "naming the images by tag in the bundled compose files"
TAGGED_DIGEST='([A-Za-z0-9._-]+(:[0-9]+)?/)?([A-Za-z0-9._/-]+):([A-Za-z0-9._-]+)@sha256:[0-9a-f]{64}'
TAGGED_TAG='\1\3:\4'
for compose in "$BUNDLE_DIR/marula-$VERSION-$OS-$ARCH"/deploy/docker-compose*.yml; do
  [ -f "$compose" ] || continue
  sed -E "s#${TAGGED_DIGEST}#${TAGGED_TAG}#g" "$compose" >"$compose.tmp"
  mv "$compose.tmp" "$compose"
  # Anything this left behind is a shape the rewrite did not understand, and the
  # bundle would name one image two ways. Refuse to ship it.
  if remaining=$(grep -nE "$TAGGED_DIGEST" "$compose"); then
    printf 'offline-bundle: %s still names a tagged image by digest:\n' "$compose" >&2
    printf '%s\n' "$remaining" >&2
    die "this bundle would name one image two ways; that image needs a tag a tar can carry"
  fi
done

# The bundle's own compose file is the one the install runs, and it has to resolve
# to exactly what `images.list` names. That is the invariant behind all of the
# above, and it is worth asking compose rather than trusting the rewrite: a
# `--compose-file` that is not the file in the release directory, or a shape the
# rewrite did not understand, shows up here and nowhere else.
say "checking the bundled compose file against the image list"
BUNDLED_COMPOSE="$BUNDLE_DIR/marula-$VERSION-$OS-$ARCH/deploy/docker-compose.yml"
CHECK_ARGS=(--env-file "$ENV_FILE" -f "$BUNDLED_COMPOSE")
for profile in ${PROFILES[@]+"${PROFILES[@]}"}; do
  [ -n "$profile" ] && CHECK_ARGS+=(--profile "$profile")
done
COMPOSE_PROJECT_NAME=marula docker compose "${CHECK_ARGS[@]}" --profile pooler config --images |
  LC_ALL=C sort -u >"$STAGING/bundled.images"
if ! diff -u "$BUNDLE_DIR/images.list" "$STAGING/bundled.images" >"$STAGING/images.diff"; then
  printf 'offline-bundle: the bundled compose file and images.list disagree:\n' >&2
  cat "$STAGING/images.diff" >&2
  die "the archive would carry one set of names and ask for another; an image value Compose interpolates is one shape this script cannot rewrite"
fi

# And the archive has to carry every name it lists. Two references that Docker
# treats as one tag would collapse here: the save keeps one, and the install would
# find the other missing.
say "checking the archive carries every name it lists"
tar -xf "$BUNDLE_DIR/images.tar" -O manifest.json >"$STAGING/manifest.json" ||
  die "images.tar has no manifest.json"
# The names the tar itself records, one per line, in the form the exporter wrote
# them.
grep -o '"RepoTags":\[[^]]*\]' "$STAGING/manifest.json" |
  sed -e 's/^"RepoTags":\[//' -e 's/\]$//' -e 's/"//g' |
  tr ',' '\n' >"$STAGING/manifest.names"
for image in "${IMAGES[@]}"; do
  tagged=$(normalise_ref "$image")
  found=0
  while IFS= read -r recorded; do
    [ -n "$recorded" ] || continue
    if [ "$(normalise_ref "$recorded")" = "$tagged" ]; then
      found=1
      break
    fi
  done <"$STAGING/manifest.names"
  [ "$found" -eq 1 ] ||
    die "images.tar does not carry $tagged; if two images share that tag they need distinct names"
done

# The nested release directory carries the sums it was assembled with, and one of
# those files has just changed. Left stale, it would accuse the bundle of the very
# edit that makes it installable.
say "rechecking the bundled release directory"
write_sums "$BUNDLE_DIR/marula-$VERSION-$OS-$ARCH"

say "writing SHA256SUMS"
write_sums "$BUNDLE_DIR"

mkdir -p "$OUT"
ARCHIVE="$OUT/$BUNDLE.tar.zst"
say "writing $ARCHIVE"
rm -f "$ARCHIVE"
tar -cf - -C "$STAGING" "$BUNDLE" | zstd -T0 -19 -q -o "$ARCHIVE"

say "done"
printf '  %s\n  %s bytes\n  verify with: tar --zstd -tf %s\n' \
  "$ARCHIVE" "$(wc -c <"$ARCHIVE" | tr -d ' ')" "$ARCHIVE"
