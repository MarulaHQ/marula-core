#!/usr/bin/env bash
# p13-build-env.sh — write, inspect or remove the machine-local build overlay.
#
# The committed .cargo/config.toml includes `.cargo/p13-fast.local.toml` with
# `optional = true`. This script is the only thing that writes that file, and it
# writes only levers this host can actually run:
#
#   linker   ld64.lld (macOS), lld or mold (Linux)   -> link time
#   sccache   local disk, beside the shared target   -> recompile time
#
# Nothing here is required. No overlay means stock cargo; `disable` restores
# that state by deleting the file. See design/qa-p52-build-throughput/REVIEW.md
# for the measured trade-offs and the group-gate procedure.
#
# Usage
#   scripts/p13-build-env.sh info        what this host can accelerate with (default)
#   scripts/p13-build-env.sh enable      write the overlay for the tools present
#   scripts/p13-build-env.sh disable     delete the overlay
#   scripts/p13-build-env.sh check       exit 1 when the overlay disagrees with this host
#   scripts/p13-build-env.sh selftest    prove the generate/disable/check paths
#
#   --dry-run     with `enable`: print the overlay, write nothing
#   --json        with `info`/`check`: machine-readable
#   --force       with `enable`: rewrite even when the file already matches
#
# Environment seams (used by selftest, and by the benchmark)
#   P13_BIN_ROOT     look for tools only under this dir instead of PATH
#   P13_HOST_OS      override `uname -s` (Darwin|Linux)
#   P13_OVERLAY_PATH override the overlay path (default .cargo/p13-fast.local.toml)
#   P13_TARGET_DIR   override the target dir used to place the sccache dir
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OVERLAY="${P13_OVERLAY_PATH:-$ROOT/.cargo/p13-fast.local.toml}"

# Remote sccache backends. The provenance guard cannot see artifacts a remote
# cache returns, which is why nothing here writes one; exported by hand is
# reported, never configured.
REMOTE_VARS=(SCCACHE_REDIS SCCACHE_BUCKET SCCACHE_S3_BUCKET SCCACHE_GCS_BUCKET
             SCCACHE_AZURE_CONNECTION_STRING SCCACHE_WEBDAV_ENDPOINT)

say() { printf '%s\n' "$*"; }
die() { printf 'p13-build-env: %s\n' "$*" >&2; exit 2; }

# have <prog> — is it runnable? P13_BIN_ROOT narrows the search to one dir so a
# selftest can present a machine that lacks the tool.
have() {
  if [ -n "${P13_BIN_ROOT:-}" ]; then
    [ -x "$P13_BIN_ROOT/$1" ]
  else
    command -v "$1" >/dev/null 2>&1
  fi
}

host_os() { printf '%s' "${P13_HOST_OS:-$(uname -s)}"; }

# ---------------------------------------------------------------------------
# detection
# ---------------------------------------------------------------------------
LINKER_KIND=""
LINKER_FLAG=""
LINKER_ALT=""

detect_linker() {
  LINKER_KIND=""; LINKER_FLAG=""; LINKER_ALT=""
  case "$(host_os)" in
    Darwin)
      local cand cands
      if [ -n "${P13_BIN_ROOT:-}" ]; then
        cands="$P13_BIN_ROOT/ld64.lld"
      else
        cands="$(command -v ld64.lld 2>/dev/null)
/opt/homebrew/opt/lld/bin/ld64.lld
/usr/local/opt/lld/bin/ld64.lld"
      fi
      for cand in $cands; do
        [ -n "$cand" ] && [ -x "$cand" ] || continue
        LINKER_KIND="ld64.lld"
        # clang accepts an absolute path after -fuse-ld, which is the form that
        # works on macOS: Apple's driver has no bare `lld` flavour to switch to.
        LINKER_FLAG="-C link-arg=-fuse-ld=$cand"
        return
      done
      LINKER_ALT="brew install lld"
      ;;
    Linux)
      local mold_ok=0
      have clang && mold_ok=1
      if have ld.lld || have lld; then
        LINKER_KIND="lld"
        LINKER_FLAG="-C link-arg=-fuse-ld=lld"
      elif have mold && [ "$mold_ok" = 1 ]; then
        # mold through the gcc driver needs gcc>=12; naming clang as the driver
        # keeps the flag valid on gcc 11 too.
        LINKER_KIND="mold"
        LINKER_FLAG="-C linker=clang -C link-arg=-fuse-ld=mold"
      fi
      [ -z "$LINKER_KIND" ] && LINKER_ALT="apt install lld  (or mold + clang)"
      ;;
    *)
      LINKER_ALT="no linker recipe for this OS"
      ;;
  esac
}

HAVE_SCCACHE=0
detect_sccache() {
  HAVE_SCCACHE=0
  have sccache && HAVE_SCCACHE=1
  return 0
}

HAVE_NEXTEST=0
detect_nextest() {
  HAVE_NEXTEST=0
  ( have cargo-nextest || have cargo-nextest-nextest ) && HAVE_NEXTEST=1
  return 0
}

remote_cache_vars() {
  local var out=""
  for var in "${REMOTE_VARS[@]}"; do
    [ -n "$(printenv "$var" 2>/dev/null)" ] && out="$out $var"
  done
  printf '%s' "${out# }"
}

# sccache_dir — one cache per machine, not one per worktree.
#
# The shared target dir is an absolute path in .cargo/config.toml (usually the
# main checkout's .kilo/target-shared). Putting the cache beside it keeps one
# copy for every checkout and keeps disk accounting in the place
# scripts/dev-clean.sh already looks. When target-dir is not absolute, sccache's
# own default wins and this returns nothing.
sccache_dir() {
  local td=""
  if [ -n "${P13_TARGET_DIR:-}" ]; then
    td="$P13_TARGET_DIR"
  else
    td="$(sed -n 's/^[[:space:]]*target-dir[[:space:]]*=[[:space:]]*"\(.*\)".*/\1/p' \
          "$ROOT/.cargo/config.toml" 2>/dev/null | head -1)"
  fi
  case "$td" in
    /*) printf '%s/sccache' "$(dirname "$td")" ;;
    *) return 0 ;;
  esac
}

# ---------------------------------------------------------------------------
# overlay rendering
# ---------------------------------------------------------------------------
render_overlay() {
  local dir
  dir="$(sccache_dir)"
  cat <<'EOF'
# GENERATED FILE — written by scripts/p13-build-env.sh; do not edit, do not commit.
#
# .cargo/config.toml includes this file with `optional = true`, so deleting it
# restores stock cargo behaviour with no other change. `scripts/p13-build-env.sh
# disable` deletes it. Content is derived from the host alone, which is what
# makes it identical in every worktree on one machine: identical overlay,
# identical fingerprints, no lane invalidating another's artifacts.
EOF
  say "# host: $(host_os) $(uname -m)  rustc: $(rustc -V 2>/dev/null | cut -d' ' -f2)"

  if [ "$HAVE_SCCACHE" = 1 ]; then
    cat <<'EOF'

[build]
rustc-wrapper = "sccache"
EOF
  fi

  if [ "$HAVE_SCCACHE" = 1 ] && [ -n "$dir" ]; then
    cat <<EOF

[env]
SCCACHE_DIR = "$dir"
SCCACHE_CACHE_SIZE = "10G"
EOF
  fi

  if [ -n "$LINKER_FLAG" ]; then
    local key="macos"
    [ "$(host_os)" = "Linux" ] && key="linux"
    cat <<EOF

# LINKER: $LINKER_KIND. Drop the overlay (or set RUSTFLAGS in the shell, which
# takes precedence over this table) if a system-framework link rejects it.
[target.'cfg(target_os = "$key")']
rustflags = ["$(printf '%s' "$LINKER_FLAG" | sed 's/ -C /", "-C /g')"]
EOF
  fi
}

overlay_would_be_empty() {
  [ "$HAVE_SCCACHE" = 0 ] && [ -z "$LINKER_FLAG" ]
}

overlay_effective() {  # settings only, so the check ignores comment churn
  grep -v '^[[:space:]]*#' "$1" 2>/dev/null | grep -v '^[[:space:]]*$'
}

# render_to <file> — the overlay text, so comparisons never read /dev/stdin
# twice (the second grep would see an exhausted descriptor).
render_to() { render_overlay > "$1"; }

# ---------------------------------------------------------------------------
# commands
# ---------------------------------------------------------------------------
probe() { detect_linker; detect_sccache; detect_nextest; }

cmd_info() {
  probe
  local remote dir
  remote="$(remote_cache_vars)"
  dir="$(sccache_dir)"

  if [ "${JSON:-0}" = 1 ]; then
    printf '{"os":"%s","arch":"%s","linker":"%s","linker_flag":"%s","sccache":%s,"nextest":%s,"remote_cache_vars":"%s","overlay":"%s","overlay_present":%s}\n' \
      "$(host_os)" "$(uname -m)" "$LINKER_KIND" "$LINKER_FLAG" \
      "$([ "$HAVE_SCCACHE" = 1 ] && echo true || echo false)" \
      "$([ "$HAVE_NEXTEST" = 1 ] && echo true || echo false)" \
      "$remote" "$OVERLAY" "$([ -f "$OVERLAY" ] && echo true || echo false)"
    return 0
  fi

  say "host          $(host_os) $(uname -m)  $(rustc -V 2>/dev/null)"
  say "linker        ${LINKER_KIND:-none}${LINKER_FLAG:+  ($LINKER_FLAG)}"
  [ -z "$LINKER_KIND" ] && say "              install: $LINKER_ALT"
  say "sccache       $([ "$HAVE_SCCACHE" = 1 ] && echo yes || echo 'no  (cargo binstall sccache)')"
  if [ "$HAVE_SCCACHE" = 1 ]; then
    say "sccache dir   ${dir:-sccache default}"
  fi
  say "nextest       $([ "$HAVE_NEXTEST" = 1 ] && echo yes || echo 'no  (cargo binstall cargo-nextest)')"
  say "overlay       $OVERLAY"
  say "              $([ -f "$OVERLAY" ] && echo present || echo 'absent — stock cargo')"
  if [ -n "$remote" ]; then
    say "REMOTE CACHE  $remote is set in this shell."
    say "              Nothing in this repo configures a remote cache, and the"
    say "              provenance guard cannot vouch for artifacts it did not"
    say "              build (scripts/lane-target-guard.py docstring). Unset it"
    say "              before a shared-target build."
  fi
  if overlay_would_be_empty; then
    say ""
    say "No accelerator on this host, so \`enable\` writes nothing. That is the"
    say "documented fallback: stock cargo, no overlay, no build change."
  fi
  return 0
}

cmd_enable() {
  probe
  if overlay_would_be_empty; then
    say "p13-build-env: no accelerator present — nothing to enable."
    say "  linker: ${LINKER_ALT:-none}"
    say "  sccache: not installed"
    [ -f "$OVERLAY" ] && say "  note: a stale overlay exists at $OVERLAY; run \`disable\` to remove it."
    return 0
  fi

  local rendered; rendered="$(render_overlay)"
  if [ "${DRY_RUN:-0}" = 1 ]; then
    say "$rendered"
    return 0
  fi
  mkdir -p "$(dirname "$OVERLAY")"
  local cand; cand="$(mktemp)"
  printf '%s\n' "$rendered" > "$cand"
  if [ -f "$OVERLAY" ] && [ "${FORCE:-0}" != 1 ] \
     && [ "$(overlay_effective "$OVERLAY")" = "$(overlay_effective "$cand")" ]; then
    rm -f "$cand"
    say "p13-build-env: overlay already matches this host — unchanged."
    return 0
  fi
  mv "$cand" "$OVERLAY"
  say "p13-build-env: wrote $OVERLAY"
  say "  linker: ${LINKER_KIND:-none}"
  say "  sccache: $([ "$HAVE_SCCACHE" = 1 ] && echo yes || echo no)"
  say "  The next cargo command on this machine relinks (and, with sccache,"
  say "  fills the cache once). Nothing else in the tree changed."
  return 0
}

cmd_disable() {
  if [ -f "$OVERLAY" ]; then
    rm -f "$OVERLAY"
    say "p13-build-env: removed $OVERLAY — stock cargo from here."
  else
    say "p13-build-env: no overlay at $OVERLAY — already stock."
  fi
  return 0
}

cmd_check() {
  probe
  local rc=0 detail="ok"
  if overlay_would_be_empty && [ -f "$OVERLAY" ]; then
    detail="stale overlay: no accelerator on this host"
    rc=1
  elif ! overlay_would_be_empty && [ ! -f "$OVERLAY" ]; then
    detail="overlay missing: this host has ${LINKER_KIND:-no linker}/$( [ "$HAVE_SCCACHE" = 1 ] && echo sccache || echo no-sccache )"
    rc=1
  elif [ -f "$OVERLAY" ]; then
    local cand; cand="$(mktemp)"; render_to "$cand"
    if [ "$(overlay_effective "$OVERLAY")" != "$(overlay_effective "$cand")" ]; then
      detail="overlay does not match what this host would generate"
      rc=1
    fi
    rm -f "$cand"
  fi
  if [ "${JSON:-0}" = 1 ]; then
    printf '{"ok":%s,"detail":"%s","overlay":"%s"}\n' \
      "$([ "$rc" = 0 ] && echo true || echo false)" "$detail" "$OVERLAY"
  else
    say "p13-build-env: $detail"
  fi
  return "$rc"
}

# ---------------------------------------------------------------------------
# selftest — proves the three paths without a toolchain change, without cargo
# ---------------------------------------------------------------------------
cmd_selftest() {
  local tmp; tmp="$(mktemp -d)"
  local fails=0 checks=0
  expect() { # expect <label> <got> <want>
    checks=$((checks + 1))
    if [ "$2" != "$3" ]; then
      fails=$((fails + 1))
      printf '  FAIL  %s: got %s, want %s\n' "$1" "$2" "$3"
    fi
  }

  local saved_overlay="$OVERLAY" saved_root="${P13_BIN_ROOT:-}" saved_os="${P13_HOST_OS:-}"
  export P13_OVERLAY_PATH="$tmp/overlay.toml"
  OVERLAY="$P13_OVERLAY_PATH"
  export P13_TARGET_DIR="$tmp/shared/target"

  # 1. a machine with nothing: enable writes nothing, check is clean
  mkdir -p "$tmp/empty"
  export P13_BIN_ROOT="$tmp/empty" P13_HOST_OS=Darwin
  cmd_enable >/dev/null
  expect "no tools -> no overlay" "$([ -f "$OVERLAY" ] && echo yes || echo no)" "no"
  cmd_check >/dev/null; expect "no tools -> check clean" "$?" "0"

  # 2. macOS with lld only: linker line, no wrapper
  mkdir -p "$tmp/mac"; : > "$tmp/mac/ld64.lld"; chmod +x "$tmp/mac/ld64.lld"
  export P13_BIN_ROOT="$tmp/mac"
  cmd_enable >/dev/null
  expect "lld -> overlay written" "$([ -f "$OVERLAY" ] && echo yes || echo no)" "yes"
  expect "lld -> fuse-ld path" \
    "$(grep -c "link-arg=-fuse-ld=$tmp/mac/ld64.lld" "$OVERLAY")" "1"
  expect "lld -> target_os macos" \
    "$(grep -c "cfg(target_os = \"macos\")" "$OVERLAY")" "1"
  expect "lld alone -> no rustc-wrapper" "$(grep -c 'rustc-wrapper' "$OVERLAY")" "0"
  cmd_check >/dev/null; expect "lld -> check clean" "$?" "0"

  # 3. macOS with lld + sccache: both levers, cache beside the shared target
  : > "$tmp/mac/sccache"; chmod +x "$tmp/mac/sccache"
  cmd_enable >/dev/null
  expect "sccache -> wrapper" "$(grep -c 'rustc-wrapper = "sccache"' "$OVERLAY")" "1"
  expect "sccache -> dir beside target" \
    "$(grep -c "SCCACHE_DIR = \"$tmp/shared/sccache\"" "$OVERLAY")" "1"

  # 4. Linux with lld: the same lever, spelled for the other OS
  mkdir -p "$tmp/linux"; : > "$tmp/linux/ld.lld"; chmod +x "$tmp/linux/ld.lld"
  export P13_BIN_ROOT="$tmp/linux" P13_HOST_OS=Linux
  cmd_enable >/dev/null
  expect "linux lld -> target_os linux" \
    "$(grep -c "cfg(target_os = \"linux\")" "$OVERLAY")" "1"
  expect "linux lld -> fuse-ld=lld" "$(grep -c 'link-arg=-fuse-ld=lld' "$OVERLAY")" "1"
  cmd_check >/dev/null; expect "linux lld -> check clean" "$?" "0"

  # 5. check notices a stale overlay: the tool went away
  export P13_BIN_ROOT="$tmp/empty"
  cmd_check >/dev/null; expect "tool gone -> check fails" "$?" "1"

  # 6. disable is the documented revert, and is idempotent
  cmd_disable >/dev/null
  expect "disable removes the overlay" "$([ -f "$OVERLAY" ] && echo yes || echo no)" "no"
  cmd_disable >/dev/null; expect "disable is idempotent" "$?" "0"
  cmd_check >/dev/null; expect "stock after disable -> check clean" "$?" "0"

  # restore the caller's environment
  OVERLAY="$saved_overlay"
  if [ -n "$saved_root" ]; then export P13_BIN_ROOT="$saved_root"; else unset P13_BIN_ROOT; fi
  if [ -n "$saved_os" ]; then export P13_HOST_OS="$saved_os"; else unset P13_HOST_OS; fi
  unset P13_OVERLAY_PATH P13_TARGET_DIR

  if [ "$fails" -gt 0 ]; then
    say "p13-build-env selftest: $fails of $checks failed"
    rm -rf "$tmp"
    return 1
  fi
  say "p13-build-env selftest: $checks checks pass (no cargo, no toolchain change)"
  rm -rf "$tmp"
  return 0
}

# ---------------------------------------------------------------------------
CMD=info
JSON=0
DRY_RUN=0
FORCE=0
for arg in "$@"; do
  case "$arg" in
    info|enable|disable|check|selftest) CMD="$arg" ;;
    --json) JSON=1 ;;
    --dry-run) DRY_RUN=1 ;;
    --force) FORCE=1 ;;
    -h|--help) sed -n '2,30p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown argument: $arg (try --help)" ;;
  esac
done

case "$CMD" in
  info) cmd_info ;;
  enable) cmd_enable ;;
  disable) cmd_disable ;;
  check) cmd_check ;;
  selftest) cmd_selftest ;;
esac
