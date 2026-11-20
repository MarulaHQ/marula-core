#!/bin/sh
# install.sh — the one-line entrance to a Marula release.
#
#   curl -fsSL https://github.com/MarulaHQ/marula-core/releases/latest/download/install.sh | sh
#   curl -fsSL .../releases/latest/download/install.sh | sh -s -- --dir /srv/marula --apply
#   curl -fsSL .../releases/download/v0.6.1/install.sh | sh -s -- --version 0.6.1
#
# What it does, in order: works out the platform, downloads that platform's
# archive, its checksum list and the signature over that list, verifies the
# signature against the release key pinned below, verifies the archive against
# the list, unpacks it, and hands over to the bundle's own installer. Anything
# after `--` is passed through to that installer unchanged.
#
# Why the key is pinned in this file rather than fetched: the release key is the
# only thing that makes a download trustworthy, so it cannot come down the same
# channel as the download. This is the same key as
# `release-keys/allowed_signers` in the repository, and
# `docs/release.md` explains the trust model.
#
# Non-purpose: it does not install Docker, it does not run without a shell, and
# it refuses to skip the signature because a missing tool is not a reason to
# trust a download.
set -eu

REPO="MarulaHQ/marula-core"
KEY_ID="releases@getmarula.com"
KEY_LINE="releases@getmarula.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOJw34oaF4eoSLXMzPpVLpXkXGOtAYEnE/ZhY9/hItn9 releases@getmarula.com"
KEY_FINGERPRINT="SHA256:F1n6L6KfpRiPy8tF0Y465VWobIVrYPExwseUefv4Heg"

say() { printf '==> %s\n' "$*"; }
die() { printf 'install: %s\n' "$*" >&2; exit 1; }

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
      sed -n '2,/^set -eu$/p' "$0" 2>/dev/null | sed 's/^# \{0,1\}//; $d' ||
        printf 'see https://getmarula.com/docs\n'
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

case "$(uname -s)" in
  Linux) OS=linux ;;
  Darwin) OS=macos ;;
  *)
    die "this script covers Linux and macOS. On Windows, run it inside WSL or Git Bash."
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
  BASE="$RELEASES_URL/download/v${VERSION#v}"
fi

if [ "$OFFLINE" -eq 1 ]; then
  [ "$OS" = "linux" ] || die "the air-gapped bundle is published for Linux only"
  ASSET="marula-${VERSION}-offline-linux-${ARCH}.tar.zst"
else
  ASSET="marula-${VERSION}-${OS}-${ARCH}.tar.gz"
fi
# The per-platform list names that platform's archive and nothing else. The
# air-gapped bundle is written after the build legs, so only the combined list
# carries it.
if [ "$OFFLINE" -eq 1 ]; then
  SUMS="SHA256SUMS"
else
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

TMP=$(mktemp -d "${TMPDIR:-/tmp}/marula-install.XXXXXX")
trap 'rm -rf "$TMP"' EXIT INT TERM

# "latest" cannot name an asset, because asset names carry the version. The
# redirect from releases/latest tells us which one it is, and the first file we
# fetch resolves it for us.
say "fetching ${ASSET} from ${VERSION}"
if [ "$VERSION" = "latest" ]; then
  # The checksum list is the cheap way to learn the real version: follow the
  # redirect and read the tag out of the final URL.
  if command -v curl >/dev/null 2>&1; then
    # The release *page* redirect, not the asset URL: asset downloads leave for
    # the CDN, and the final URL there carries no tag to read. This one ends at
    # `/tag/vX.Y.Z`.
    if ! PAGE=$(curl -fsSL -o /dev/null -w '%{url_effective}' "$RELEASES_URL/latest" 2>/dev/null); then
      PAGE=""
    fi
    RESOLVED=$(printf '%s' "$PAGE" | sed -n 's#.*/tag/v\([^/]*\)/*$#\1#p' | head -1)
    [ -n "${RESOLVED:-}" ] || die "could not resolve the latest release; pass --version"
    VERSION="$RESOLVED"
    ASSET=$(printf '%s' "$ASSET" | sed "s/marula-latest-/marula-${VERSION}-/")
    BASE="$RELEASES_URL/download/v${VERSION}"
    say "latest is ${VERSION}"
  else
    die "--version is required when only wget is available"
  fi
fi

fetch "$BASE/$SUMS" "$TMP/$SUMS"
fetch "$BASE/$SUMS.sig" "$TMP/$SUMS.sig"

command -v ssh-keygen >/dev/null 2>&1 ||
  die "needs ssh-keygen to verify the release signature (openssh-client). Without it a download cannot be trusted."

printf '%s\n' "$KEY_LINE" > "$TMP/allowed_signers"
say "verifying the signature over $SUMS"
ssh-keygen -Y verify -f "$TMP/allowed_signers" -I "$KEY_ID" -n file -s "$TMP/$SUMS.sig" < "$TMP/$SUMS" >/dev/null 2>&1 ||
  die "the signature does not verify against the release key $KEY_FINGERPRINT. Do not install this download."

say "fetching $ASSET"
fetch "$BASE/$ASSET" "$TMP/$ASSET"

say "checking the archive against the signed list"
# Exactly the asset that was fetched, not every line the list happens to carry:
# a platform list names one archive, and checking anything else would fail on a
# file this script never downloaded.
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

say "unpacking"
case "$ASSET" in
  *.tar.zst)
    if tar --zstd -tf "$TMP/$ASSET" >/dev/null 2>&1; then
      tar --zstd -xf "$TMP/$ASSET" -C "$TMP"
    else
      command -v zstd >/dev/null 2>&1 ||
        die "the air-gapped bundle needs zstd to unpack: apt install zstd, dnf install zstd, or brew install zstd"
      zstd -dc "$TMP/$ASSET" | tar -xf - -C "$TMP"
    fi
    DIR="$TMP/marula-${VERSION}-offline-${OS}-${ARCH}"
    ENTRY="$DIR/install-offline.sh"
    ;;
  *)
    tar -xzf "$TMP/$ASSET" -C "$TMP"
    DIR="$TMP/marula-${VERSION}-${OS}-${ARCH}"
    ENTRY="$DIR/install.sh"
    ;;
esac

[ -x "$ENTRY" ] || die "the archive did not unpack into the shape this script expects ($ENTRY)"

say "handing over to the bundle installer"
# No `cd`: the bundle's installer captures its own starting directory and
# resolves a relative --dir against it, so changing directory here would move a
# caller's `--dir ./instance` into the temporary unpack.
#
# No `exec` either. An exec would replace this shell and the EXIT trap would
# never run, leaving the archive and the unpacked bundle behind on every
# successful install -- and the air-gapped bundle is several hundred megabytes.
# The parse loop stops at the first argument it does not own, so "$@" is exactly
# what the bundle installer should receive, spaces and all.
status=0
"$ENTRY" "$@" || status=$?
trap - EXIT INT TERM
rm -rf "$TMP"
exit "$status"
