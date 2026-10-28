#!/usr/bin/env bash
# bootstrap.sh — the one-line installer the docs print.
#
#   curl -fsSLO https://<release host>/bootstrap.sh
#   bash bootstrap.sh --base-url https://<release host>/vX.Y.Z/
#
# Save it first, then run it. This script refuses to be executed from a pipe
# (`curl … | sh`), because "download a script and hand it to a shell" is the
# habit that makes every later check pointless: the bytes that decide what to
# verify would themselves be unverified.
#
# What it does, in order, stopping at the first failure:
#
#   1. refuses any base URL that is not https, and passes curl a protocol
#      allowlist so a redirect cannot downgrade it either;
#   2. fetches `release.json` (the manifest: which version this directory is),
#      the platform's `SHA256SUMS-<os>-<arch>` and its `.sig`, and the archive
#      the manifest's version names;
#   3. verifies the signature over the sums with an ALREADY TRUSTED `marula`
#      (`--verifier`, or the one on PATH) and stops if the signature or any
#      checksum is wrong. A binary that has not been verified yet cannot
#      verify itself, so the verifier is the copy you already have. If there
#      is none, this script stops and prints the by-hand command instead of
#      guessing. The archive is downloaded first so a verifier that re-hashes
#      what its list names finds every file in place;
#   4. checks the archive's bytes a second way, with the verifier's `artifact
#      checksum` compared against the line the signature covers, and a third
#      way after extraction, against the archive's own tree sums;
#   5. prints where it put the binary and what to run next.
#
# Non-purpose: it does not install Docker, edit a shell profile, or run
# `marula install` for you. Placing a verified binary and starting a stack are
# two decisions; this script makes the first one, and says the second aloud.
set -euo pipefail

say() { printf '==> %s\n' "$*"; }
die() { printf 'bootstrap: %s\n' "$*" >&2; exit 1; }

# ── refuse `curl … | bash` ─────────────────────────────────────────────────
# When bash reads its program from stdin there is no source file to point at,
# which is exactly the shape of a piped execution.
if [ -z "${BASH_SOURCE[0]:-}" ] || [ ! -f "${BASH_SOURCE[0]}" ]; then
  die "this script must be saved to a file and run from there; piping a remote file into a shell is the pattern it refuses"
fi

BASE_URL="${MARULA_RELEASE_BASE_URL:-}"
INSTALL_DIR="${MARULA_INSTALL_DIR:-$HOME/.local/bin}"
KEY=""
VERIFIER="${MARULA_VERIFIER:-}"
OS=""
ARCH=""
KEEP=0

while [ $# -gt 0 ]; do
  case "$1" in
    --base-url)
      BASE_URL="${2:-}"
      shift 2
      ;;
    --install-dir)
      INSTALL_DIR="${2:-}"
      shift 2
      ;;
    --key)
      KEY="${2:-}"
      shift 2
      ;;
    --verifier)
      VERIFIER="${2:-}"
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
    --keep)
      KEEP=1
      shift
      ;;
    -h | --help)
      awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "${BASH_SOURCE[0]}"
      exit 0
      ;;
    *)
      die "unknown argument $1"
      ;;
  esac
done

# ── 1. https, and nothing else ─────────────────────────────────────────────
[ -n "$BASE_URL" ] || die "--base-url is required (or set MARULA_RELEASE_BASE_URL). It is supplied by whoever publishes the release; this script knows no host."
case "$BASE_URL" in
  https://?*) ;;
  *)
    die "refusing $BASE_URL: the release is fetched over https only"
    ;;
esac
case "$BASE_URL" in
  *[[:space:]]*) die "refusing $BASE_URL: it contains whitespace" ;;
esac
BASE_URL="${BASE_URL%/}"
case "$BASE_URL" in
  */..*) die "refusing $BASE_URL: no parent segments in a release URL" ;;
esac

command -v curl >/dev/null 2>&1 || die "curl is not on PATH"
command -v tar >/dev/null 2>&1 || die "tar is not on PATH"

[ -n "$OS" ] || OS=$(uname -s | tr '[:upper:]' '[:lower:]')
[ -n "$ARCH" ] || ARCH=$(uname -m)
case "$ARCH" in
  x86_64 | amd64 | x64) ARCH=x64 ;;
  aarch64 | arm64) ARCH=arm64 ;;
  *) die "unsupported architecture $ARCH" ;;
esac
case "$OS" in
  darwin) OS=macos ;;
  linux | macos) ;;
  *) die "unsupported platform $OS (this release ships macOS, Linux and Windows)" ;;
esac

# ── 3a. the verifier, before anything is downloaded ────────────────────────
if [ -z "$VERIFIER" ]; then
  VERIFIER=$(command -v marula || true)
fi
if [ -z "$VERIFIER" ] || [ ! -x "$VERIFIER" ]; then
  cat >&2 <<'EOF'
bootstrap: no trusted `marula` to verify with.

A binary cannot authenticate itself: the copy that checks the signature has to
be one you already trust. Two ways forward:

  * point this script at a verifier you already have: --verifier /path/to/marula
    (or MARULA_VERIFIER=/path/to/marula), or install a release you trust first;

  * verify by hand and place the binary yourself:
      ssh-keygen -Y verify -f allowed_signers -I releases@getmarula.com -n file \
        -s SHA256SUMS-linux-x64.sig < SHA256SUMS-linux-x64
    with release-keys/allowed_signers from the repository. docs/release.md
    carries the same command with the full context.
EOF
  exit 1
fi
say "verifying with $VERIFIER"

# ── 2. fetch the manifest and the signed sums ──────────────────────────────
WORK=$(mktemp -d)
cleanup() {
  if [ "$KEEP" -eq 1 ]; then
    printf '  kept: %s\n' "$WORK"
  else
    rm -rf "$WORK"
  fi
}
trap cleanup EXIT

# `=https` on both the request and the redirect, so a 302 cannot move the
# download to plain http or to another scheme.
fetch() {
  curl --fail --silent --show-error --location --max-redirs 3 \
    --proto '=https' --proto-redir '=https' --tlsv1.2 --retry 2 --retry-delay 1 \
    --output "$2" "$1"
}

say "fetching $BASE_URL/release.json"
fetch "$BASE_URL/release.json" "$WORK/release.json"
VERSION=$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$WORK/release.json" | head -n 1)
# Strict on purpose: this string becomes a file name below, so anything that
# could climb out of the working directory is refused rather than slugged.
if ! printf '%s' "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$'; then
  die "release.json does not name a plain version (got \"${VERSION:-}\")"
fi
say "this release is $VERSION"

SUMS="SHA256SUMS-$OS-$ARCH"
SIG="$SUMS.sig"
ARCHIVE="marula-$VERSION-$OS-$ARCH.tar.gz"

say "fetching $SUMS and $SUMS.sig"
fetch "$BASE_URL/$SUMS" "$WORK/$SUMS"
fetch "$BASE_URL/$SIG" "$WORK/$SIG"

# The archive is fetched BEFORE the signature is checked, so that a verifier
# which also re-hashes the files its list names finds every one of them in the
# directory it is handed. Verifying first and downloading second left the list
# pointing at a file that was not there yet, which a strict verifier refuses.
say "fetching $ARCHIVE"
fetch "$BASE_URL/$ARCHIVE" "$WORK/$ARCHIVE"

# ── 3b. signature over the sums, then the archive's bytes ──────────────────
say "verifying the signature over $SUMS"
VERIFY_ARGS=(artifact verify --sums "$WORK/$SUMS" --signature "$WORK/$SIG")
if [ -n "$KEY" ]; then
  VERIFY_ARGS+=(--key "$KEY")
fi
if ! "$VERIFIER" "${VERIFY_ARGS[@]}"; then
  die "signature verification failed. Nothing was installed."
fi

EXPECTED=$(awk -v name="$ARCHIVE" '$2 == name { print $1; exit }' "$WORK/$SUMS")
[ -n "$EXPECTED" ] || die "$SUMS has no line for $ARCHIVE. Nothing was installed."

ACTUAL=$("$VERIFIER" artifact checksum "$WORK/$ARCHIVE" | awk '{ print $1 }')
if [ "$ACTUAL" != "$EXPECTED" ]; then
  die "$ARCHIVE does not match the signed checksum (expected $EXPECTED, got $ACTUAL). Nothing was installed."
fi

# ── 4. unpack, check the tree, place the binary ────────────────────────────
say "unpacking"
tar -xzf "$WORK/$ARCHIVE" -C "$WORK"
RELEASE="$WORK/marula-$VERSION-$OS-$ARCH"
[ -d "$RELEASE" ] || die "the archive did not contain marula-$VERSION-$OS-$ARCH"

say "checking the tree's own sums"
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$RELEASE" && sha256sum --quiet -c SHA256SUMS)
else
  (cd "$RELEASE" && shasum -a 256 -c SHA256SUMS >/dev/null)
fi

BIN="$RELEASE/marula"
[ -x "$BIN" ] || die "the archive carried no marula binary"

mkdir -p "$INSTALL_DIR"
[ -w "$INSTALL_DIR" ] || die "$INSTALL_DIR is not writable"
install -m 0755 "$BIN" "$INSTALL_DIR/marula"
say "installed $INSTALL_DIR/marula ($VERSION)"

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    printf '  %s is not on PATH yet. Add it, then open a new shell:\n' "$INSTALL_DIR"
    # The literal `$PATH` is what the operator copies, not a value to expand.
    # shellcheck disable=SC2016
    printf '    export PATH="%s:$PATH"\n' "$INSTALL_DIR"
    ;;
esac
printf '  next: marula install --ui\n'
if [ "$KEEP" -eq 1 ]; then
  printf '  the full release tree (deploy files, installer, wizard) is at %s\n' "$RELEASE"
fi
