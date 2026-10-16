#!/usr/bin/env bash
# repro-shared-target-poison.sh — deterministic reproduction of the shared
# cargo target dir poisoning (Wave 47 lane A17), plus the proof the guard fixes
# it.  Run this twice: unchanged config (the failure) and with the guard (green).
#
#   scripts/repro-shared-target-poison.sh                    # all three modes
#   scripts/repro-shared-target-poison.sh --mode=baseline    # poison reproduces
#   scripts/repro-shared-target-poison.sh --mode=trap        # cargo clean needs --release
#   scripts/repro-shared-target-poison.sh --mode=guard       # the guard prevents it
#   scripts/repro-shared-target-poison.sh --variant=one-tree # see VARIANTS
#   KEEP=1 scripts/repro-shared-target-poison.sh             # keep the scratch trees
#
# WHAT IT PROVES
#   baseline  Two trees share one target dir.  The peer builds release first;
#             the current tree's dependency source is then OLDER than the peer's
#             recorded mtime (so cargo calls the peer's rlib fresh) while the
#             crate that uses it is NEWER (so cargo recompiles it).  Rustc then
#             compiles against an rlib that predates the constant and reports a
#             phantom "cannot find value FLAG" — while `cargo check` passes,
#             which is the signature the lane recorded on 2026-10-16.
#   trap      `cargo clean -p dep -p app` WITHOUT `--release` leaves the poisoned
#             release rlib in place: the release build still fails.  Only
#             `cargo clean -p … --release` clears it.  This is the trap the A17
#             brief requires the guard to cover, asserted on the command line.
#   guard     The guard classifies the dir as foreign, resets the workspace
#             members FOR THAT PROFILE (carrying --release), and the same build
#             that failed in `baseline` now compiles from this tree's sources.
#             A second check after `mark` reports SAFE, so the guard does not
#             reset on every boot.
#
# VARIANTS
#   two-tree  (default) the peer phase builds from a second checkout path, which
#             is what a lane worktree actually is.  Whether the artifact names
#             collide is cargo's business; if the sandbox says they do not, use
#             one-tree.
#   one-tree  the peer phase builds at the SAME path with older content, which
#             makes the collision unconditional.  The guard phase then removes
#             the stamp instead of finding a foreign one.
#
# Exit 0 = every assertion held.  1 = an assertion broke (the text says which).
set -uo pipefail

MODE=all
VARIANT=two-tree
for arg in "$@"; do
  case "$arg" in
    --mode=*) MODE="${arg#--mode=}" ;;
    --variant=*) VARIANT="${arg#--variant=}" ;;
    -h|--help) sed -n '1,40p' "$0"; exit 0 ;;
    *) echo "repro: unknown argument $arg" >&2; exit 2 ;;
  esac
done
[ "$MODE" = "all" ] || case "$MODE" in baseline|trap|guard) ;; *) echo "repro: bad --mode=$MODE" >&2; exit 2 ;; esac
case "$VARIANT" in two-tree|one-tree) ;; *) echo "repro: bad --variant=$VARIANT" >&2; exit 2 ;; esac

REPO="$(cd "$(dirname "$0")/.." && pwd)"
GUARD="$REPO/scripts/lane-target-guard.py"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/marula-target-poison.XXXXXX")"
SHARED="$WORK/shared-target"
PEER="$WORK/peer-checkout"
LANE="$WORK/lane-checkout"
[ "$VARIANT" = "one-tree" ] && PEER="$LANE"
[ "${KEEP:-0}" = "1" ] || trap 'rm -rf "$WORK"' EXIT

export CARGO_TARGET_DIR="$SHARED"
export CARGO_TERM_COLOR=never
FAILURES=0

say()  { echo "repro: $*"; }
pass() { echo "repro:   PASS  $*"; }
fail() { echo "repro:   FAIL  $*"; FAILURES=$((FAILURES + 1)); }

# write_tree <dir> <have-flag|no-flag>
write_tree() {
  local dir="$1" variant="$2"
  rm -rf "$dir"
  mkdir -p "$dir/crates/dep/src" "$dir/crates/app/src"
  cat > "$dir/Cargo.toml" <<'EOF'
[workspace]
resolver = "2"
members = ["crates/dep", "crates/app"]
EOF
  cat > "$dir/crates/dep/Cargo.toml" <<'EOF'
[package]
name = "dep"
version = "0.1.0"
edition = "2021"
EOF
  cat > "$dir/crates/app/Cargo.toml" <<'EOF'
[package]
name = "app"
version = "0.1.0"
edition = "2021"

[dependencies]
dep = { path = "../dep" }
EOF
  if [ "$variant" = "have-flag" ]; then
    # The constant the current tree's app references, absent from the peer build.
    cat > "$dir/crates/dep/src/lib.rs" <<'EOF'
pub mod types {
    pub const FLAG: u8 = 7;
}
EOF
    cat > "$dir/crates/app/src/lib.rs" <<'EOF'
pub fn answer() -> u8 {
    dep::types::FLAG
}
EOF
  else
    cat > "$dir/crates/dep/src/lib.rs" <<'EOF'
pub mod types {
    pub const OTHER: u8 = 3;
}
EOF
    cat > "$dir/crates/app/src/lib.rs" <<'EOF'
pub fn answer() -> u8 {
    0
}
EOF
  fi
}

build() { # build <dir> <check|build>
  local dir="$1" verb="$2"
  if [ "$verb" = "build" ]; then
    (cd "$dir" && cargo build --release --offline -q) 
  else
    (cd "$dir" && cargo check --offline -q)
  fi
}

build_log() { # build_log <dir> <logfile> — release build, log captured
  local dir="$1" log="$2"
  (cd "$dir" && cargo build --release --offline) >"$log" 2>&1
}

# The poisoning move: the current tree's dep source already exists but with an
# mtime older than the peer's build, so cargo calls the peer's rlib fresh; the
# crate that references the constant is touched so cargo recompiles it.
age_dep() {
  local dir="$1"
  touch -t 200101010000 "$dir/crates/dep/src/lib.rs" "$dir/crates/dep/Cargo.toml"
  touch "$dir/crates/app/src/lib.rs" "$dir/crates/app/Cargo.toml"
}

reset_shared() { rm -rf "$SHARED"; }

# ==========================================================================
run_baseline() {
  say "mode baseline — poison the shared dir the recorded way"
  reset_shared
  write_tree "$PEER" no-flag
  if ! build "$PEER" build; then fail "peer release build did not succeed"; return; fi
  pass "peer (no FLAG) built release into the shared dir"

  write_tree "$LANE" have-flag
  age_dep "$LANE"
  if build "$LANE" check; then pass "cargo check passes in the current tree (the recorded signature)"; else
    fail "cargo check failed — the construct is wrong, not the poisoning"; fi

  if build_log "$LANE" "$WORK/baseline.log"; then
    fail "release build SUCCEEDED — poisoning did not reproduce (try --variant=one-tree)"
  else
    if grep -q "FLAG" "$WORK/baseline.log"; then
      pass "release build failed on the phantom symbol the current tree defines:"
      grep -m2 -E "error(\[E[0-9]+\])?: .*FLAG" "$WORK/baseline.log" | sed 's/^/repro:       /'
    else
      fail "release build failed for another reason — see $WORK/baseline.log"
      tail -5 "$WORK/baseline.log" | sed 's/^/repro:       /'
    fi
  fi
}

# ==========================================================================
run_trap() {
  say "mode trap — cargo clean -p without --release leaves the poisoned release rlib"
  reset_shared
  write_tree "$PEER" no-flag
  build "$PEER" build || { fail "peer release build did not succeed"; return; }
  write_tree "$LANE" have-flag
  age_dep "$LANE"

  (cd "$LANE" && cargo clean -p dep -p app) >/dev/null 2>&1
  if build_log "$LANE" "$WORK/trap.log"; then
    fail "build succeeded after a dev-profile clean — the trap did not reproduce"
  else
    pass "still failing after 'cargo clean -p dep -p app' (dev profile only)"
  fi

  (cd "$LANE" && cargo clean -p dep -p app --release) >/dev/null 2>&1
  if build_log "$LANE" "$WORK/trap2.log"; then
    pass "'cargo clean -p dep -p app --release' clears it — the same build now succeeds"
  else
    fail "release-profile clean did not fix the build — see $WORK/trap2.log"
  fi
}

# ==========================================================================
run_guard() {
  say "mode guard — the guard detects the foreign artifact before the build"
  reset_shared
  write_tree "$PEER" no-flag
  build "$PEER" build || { fail "peer release build did not succeed"; return; }
  if [ "$VARIANT" = "two-tree" ]; then
    python3 "$GUARD" mark --profile release --repo "$PEER" --note "repro: peer" >/dev/null \
      || fail "guard mark failed for the peer checkout"
  else
    # Same path, so the stamp would name this worktree: drop it, which is the
    # guard's conservative 'somebody built here and did not say who' case.
    rm -f "$SHARED/.marula-lane-stamp.json"
  fi

  write_tree "$LANE" have-flag
  age_dep "$LANE"

  local verdict_log="$WORK/verdict.log"
  python3 "$GUARD" check --profile release --repo "$LANE" --wait 0 >"$verdict_log" 2>&1
  local rc=$?
  if [ "$rc" -ne 0 ]; then
    pass "guard refused the dir before the build:"
    grep -m3 -E "verdict|FIX" "$verdict_log" | sed 's/^/repro:       /'
  else
    fail "guard called the dir safe — it should have flagged the foreign artifacts"
  fi
  if grep -q -- "--release" "$verdict_log"; then
    pass "the reported fix carries --release (the release-profile trap above)"
  else
    fail "the reported fix is missing --release"
  fi

  python3 "$GUARD" check --profile release --fix --wait 0 --repo "$LANE" >"$WORK/fix.log" 2>&1 \
    || fail "guard --fix exited non-zero — see $WORK/fix.log"

  if build_log "$LANE" "$WORK/guard.log"; then
    pass "the release build that failed in baseline now succeeds"
  else
    fail "release build still failed after the guard's reset — see $WORK/guard.log"
  fi

  python3 "$GUARD" mark --profile release --repo "$LANE" --note "repro: lane" >/dev/null
  python3 "$GUARD" check --profile release --repo "$LANE" --wait 0 >"$WORK/after.log" 2>&1 \
    || fail "check after mark exited non-zero"
  if grep -q "SAFE" "$WORK/after.log"; then
    pass "a second check after mark reports SAFE (no reset on every boot)"
  else
    fail "check after mark did not report SAFE — see $WORK/after.log"
  fi
}

# ==========================================================================
echo "repro: scratch $WORK  variant $VARIANT  shared $SHARED"
case "$MODE" in
  all)      run_baseline; run_trap; run_guard ;;
  baseline) run_baseline ;;
  trap)     run_trap ;;
  guard)    run_guard ;;
esac

if [ "$FAILURES" -eq 0 ]; then
  echo "repro: OK — every assertion held (variant $VARIANT, mode $MODE)"
  exit 0
fi
echo "repro: $FAILURES assertion(s) failed (variant $VARIANT, mode $MODE)"
exit 1
