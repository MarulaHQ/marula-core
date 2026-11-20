#!/bin/sh
# install.sh — the one-line entrance to a Marula release.
#
#   curl -fsSL https://github.com/MarulaHQ/marula-core/releases/latest/download/install.sh -o /tmp/marula-install.sh
#   sh /tmp/marula-install.sh
#   sh /tmp/marula-install.sh --dir /srv/marula --apply
#   sh /tmp/marula-install.sh --version 0.6.1
#
# Written to a file and then run, rather than piped into `sh`: a pipeline's exit
# status is the last command's, so a 404 would hand an empty script to `sh` and
# report success.
#
# What it does, in order: works out the platform, downloads that platform's
# archive, its checksum list and the signature over that list, verifies the
# signature against the release key pinned below, verifies the archive against
# the list, unpacks it into a directory it keeps, and hands over to the bundle's
# own installer. Anything it does not recognise is passed through unchanged.
#
# The bundle is kept, not thrown away: `./marula`, the compose files and the
# operator runbooks live in it, and every "keep the unpacked release directory"
# instruction in docs/install.md refers to it.
#
# Why the key is pinned in this file rather than fetched: the release key is the
# only thing that makes a download trustworthy, so it cannot come down the same
# channel as the download. It is the same key as `release-keys/allowed_signers`.
#
# Non-purpose: it does not install Docker, and it does not continue when
# `ssh-keygen` is missing, because a missing tool is not a reason to trust a
# download. Windows is not an install target yet.
set -eu

REPO="MarulaHQ/marula-core"
KEY_ID="releases@getmarula.com"
KEY_LINE="releases@getmarula.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOJw34oaF4eoSLXMzPpVLpXkXGOtAYEnE/ZhY9/hItn9 releases@getmarula.com"
KEY_FINGERPRINT="SHA256:F1n6L6KfpRiPy8tF0Y465VWobIVrYPExwseUefv4Heg"

say() { printf '==> %s\n' "$*"; }
die() { printf 'install: %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<'USAGE'
Usage: install.sh [--version <X.Y.Z>] [--offline] [--help] [-- <installer args>]

Downloads the Marula release for this machine, checks its signature, unpacks it
and runs the bundle's installer. With no installer arguments that opens the
wizard and installs into ~/marula.

  --version <X.Y.Z>   Install this release instead of the newest one.
  --offline           Fetch the air-gapped bundle (Linux only).
  --dir <path>        Passed to the installer: which directory to install into.
  --apply             Passed to the installer: write the env file and start.

Anything else is passed straight through, with or without a `--` before it:
  install.sh --dir /srv/marula --apply
USAGE
}

VERSION="${MARULA_VERSION:-latest}"
OFFLINE=0
while [ $# -gt 0 ]; do
  case "$1" in
    --version)
      [ $# -ge 2 ] || die "--version needs a value"
      VERSION="$2"
      shift 2
      ;;
    --version=*)
      VERSION="${1#--version=}"
      shift
      ;;
    --offline)
      OFFLINE=1
      shift
      ;;
    --help | -h)
      usage
      exit 0
      ;;
    --)
      shift
      break
      ;;
    *)
      break
      ;;
  esac
done

# `v0.6.1` and `0.6.1` are both things a user will type, and both have to name
# the same archive.
VERSION="${VERSION#v}"

case "$(uname -s)" in
  Linux) OS=linux ;;
  Darwin) OS=macos ;;
  *)
    die "this installs on Linux and macOS. Windows has no native installer yet; see https://github.com/MarulaHQ/marula-core/issues/114"
    ;;
esac

case "$(uname -m)" in
  x86_64 | amd64) ARCH=x64 ;;
  arm64 | aarch64) ARCH=arm64 ;;
  *) die "no published build for $(uname -m)" ;;
esac

if [ "$OS" = "macos" ] && [ "$ARCH" = "x64" ]; then
  die "no published build for Intel macOS; the release carries Apple silicon and Linux only"
fi

# A mirror can repoint the downloads without touching the script, and that is
# safe because the signature below is pinned: a mirror cannot substitute bytes.
RELEASES_URL="${MARULA_RELEASES_URL:-https://github.com/$REPO/releases}"

if [ "$VERSION" = "latest" ]; then
  BASE="$RELEASES_URL/latest/download"
else
  BASE="$RELEASES_URL/download/v${VERSION}"
fi

if [ "$OFFLINE" -eq 1 ]; then
  [ "$OS" = "linux" ] || die "the air-gapped bundle is published for Linux only"
  ASSET="marula-${VERSION}-offline-linux-${ARCH}.tar.zst"
  # The per-platform list names that platform's online archive only. The
  # air-gapped bundle is written after the build legs, so the combined list is
  # the one that carries it.
  SUMS="SHA256SUMS"
else
  ASSET="marula-${VERSION}-${OS}-${ARCH}.tar.gz"
  SUMS="SHA256SUMS-${OS}-${ARCH}"
fi

fetch() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 3 --retry-delay 2 -o "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$2" "$1"
  else
    die "needs curl or wget"
  fi
}

TMP=$(mktemp -d "${TMPDIR:-/tmp}/marula-download.XXXXXX")
# A signal has to leave, not tidy up and carry on: cleaning up under a running
# child would delete the installer mid-install. `exit` runs the EXIT trap.
trap 'rm -rf "$TMP"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# "latest" cannot name an asset, because asset names carry the version. The
# release page redirect ends at /tag/vX.Y.Z, which is where the version can be
# read; asset URLs leave for the CDN and lose the tag on the way.
if [ "$VERSION" = "latest" ]; then
  say "resolving the newest release"
  command -v curl >/dev/null 2>&1 ||
    die "--version is required when only wget is available"
  if ! PAGE=$(curl -fsSL -o /dev/null -w '%{url_effective}' "$RELEASES_URL/latest" 2>/dev/null); then
    PAGE=""
  fi
  VERSION=$(printf '%s' "$PAGE" | sed -n 's#.*/tag/v\([^/]*\)/*$#\1#p' | head -1)
  [ -n "$VERSION" ] || die "could not resolve the newest release; pass --version"
  BASE="$RELEASES_URL/download/v${VERSION}"
  if [ "$OFFLINE" -eq 1 ]; then
    ASSET="marula-${VERSION}-offline-linux-${ARCH}.tar.zst"
  else
    ASSET="marula-${VERSION}-${OS}-${ARCH}.tar.gz"
  fi
  say "newest release is ${VERSION}"
fi

say "fetching ${ASSET}"
fetch "$BASE/$SUMS" "$TMP/$SUMS"
fetch "$BASE/$SUMS.sig" "$TMP/$SUMS.sig"

command -v ssh-keygen >/dev/null 2>&1 ||
  die "needs ssh-keygen to verify the release signature (openssh-client). Without it a download cannot be trusted."

printf '%s\n' "$KEY_LINE" > "$TMP/allowed_signers"
say "verifying the signature over $SUMS"
ssh-keygen -Y verify -f "$TMP/allowed_signers" -I "$KEY_ID" -n file -s "$TMP/$SUMS.sig" < "$TMP/$SUMS" >/dev/null 2>&1 ||
  die "the signature does not verify against the release key $KEY_FINGERPRINT. Do not install this download."

fetch "$BASE/$ASSET" "$TMP/$ASSET"

say "checking the archive against the signed list"
# Exactly the asset that was fetched, not every line the list happens to carry.
if ! grep -E "[[:space:]]${ASSET}\$" "$TMP/$SUMS" > "$TMP/one"; then
  die "the signed list does not name $ASSET"
fi
(
  cd "$TMP"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum --quiet -c one
  else
    shasum -a 256 --quiet -c one
  fi
) || die "the archive does not match its signed checksum; do not install it"

# The bundle is kept: `./marula`, the compose files and the runbooks are in it,
# and docs/install.md tells an operator to stay there for doctor, upgrades and
# the optional profiles. It is unpacked beside the download and moved into place
# only once it is whole.
BUNDLE_ROOT="${MARULA_BUNDLE_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/marula}"
mkdir -p "$BUNDLE_ROOT" || die "could not create $BUNDLE_ROOT"

say "unpacking into $BUNDLE_ROOT"
PARTIAL="$BUNDLE_ROOT/.unpack.$$"
rm -rf "$PARTIAL"
trap 'rm -rf "$TMP" "$PARTIAL"' EXIT
case "$ASSET" in
  *.tar.zst)
    if tar --zstd -tf "$TMP/$ASSET" >/dev/null 2>&1; then
      tar --zstd -xf "$TMP/$ASSET" -C "$BUNDLE_ROOT" --one-top-level="$PARTIAL" 2>/dev/null ||
        { mkdir -p "$PARTIAL"; tar --zstd -xf "$TMP/$ASSET" -C "$PARTIAL"; }
    else
      command -v zstd >/dev/null 2>&1 ||
        die "the air-gapped bundle needs zstd to unpack: apt install zstd, dnf install zstd, or brew install zstd"
      mkdir -p "$PARTIAL"
      zstd -dc "$TMP/$ASSET" | tar -xf - -C "$PARTIAL"
    fi
    DIR_NAME="marula-${VERSION}-offline-${OS}-${ARCH}"
    ENTRY_NAME="install-offline.sh"
    ;;
  *)
    mkdir -p "$PARTIAL"
    tar -xzf "$TMP/$ASSET" -C "$PARTIAL"
    DIR_NAME="marula-${VERSION}-${OS}-${ARCH}"
    ENTRY_NAME="install.sh"
    ;;
esac

# The archive carries one top-level directory. Move it into place, replacing an
# older unpack of the same version rather than nesting a second copy.
BUNDLE="$BUNDLE_ROOT/$DIR_NAME"
if [ -d "$PARTIAL/$DIR_NAME" ]; then
  rm -rf "$BUNDLE"
  mv "$PARTIAL/$DIR_NAME" "$BUNDLE"
elif [ -d "$PARTIAL" ]; then
  rm -rf "$BUNDLE"
  mv "$PARTIAL" "$BUNDLE"
fi
rm -rf "$PARTIAL"
ENTRY="$BUNDLE/$ENTRY_NAME"
[ -x "$ENTRY" ] || die "the archive did not unpack into the shape this script expects ($ENTRY)"

# An air-gapped bundle wraps the release directory in one more level, so the
# directory with `deploy/`, `marula` and `release.json` is not the outer one.
# Report the one an operator should stay in.
RELEASE_DIR="$BUNDLE"
if [ -d "$BUNDLE/marula-${VERSION}-${OS}-${ARCH}" ]; then
  RELEASE_DIR="$BUNDLE/marula-${VERSION}-${OS}-${ARCH}"
fi
say "kept at $BUNDLE"
say "the release directory is $RELEASE_DIR"
say "handing over to the bundle installer"
# No `cd`: the bundle's installer captures its own starting directory and
# resolves a relative --dir against it, so changing directory here would move a
# caller's `--dir ./instance` into the bundle.
status=0
"$ENTRY" "$@" || status=$?
exit "$status"
