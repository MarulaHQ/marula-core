#!/usr/bin/env bash
# Boundary checks for marula-core.
#
# This repository carries the AGPL open core and is published via the
# squash-export (see the org home's scripts/export-public-core.sh). These
# checks are the mechanical proof of the three boundary rules stated in the
# org home:
#
#   1. Nothing proprietary lands in the core.
#   2. The core carries no entitlement checks.
#   3. The core has no dependency on any repository outside itself.
#
# They are the evidence for the "enforced by CI" claim. If a rule has no check
# here, the claim is not being made for it. The enforcing job lives in
# .github/workflows/ci.yml.disabled and is enabled at the public launch.
#
# A line may opt out with a trailing marker:
#     // boundary-allow: reason
# Seams (an interface the commercial layer implements) are allowed. Gating
# (a check that refuses service) is not.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

failed=0

fail() {
  echo "::error file=$1::$2"
  failed=1
}

# Files and directories that are not product code.
SOURCE_GLOBS=(
  --include='*.rs' --include='*.ts' --include='*.tsx'
  --include='*.js' --include='*.mjs' --include='*.cjs'
  --include='*.py' --include='*.sql'
)

# grep -r rather than git grep: this must see uncommitted work, so a violation
# is caught on the branch that introduces it and not only after it merges.
# Excludes are spelled so the vendored, generated and agent-side directories
# are skipped (per-lane tasklists, graphs and worktrees are not product code).
EXCLUDES=(
  --exclude-dir=.git --exclude-dir=node_modules --exclude-dir=target
  --exclude-dir=dist --exclude-dir=.next --exclude-dir=.turbo
  --exclude-dir=.kilo --exclude-dir=graft --exclude-dir=graphify-out
  --exclude-dir=docs --exclude-dir=resources
  --exclude='*.md' --exclude='*.lock' --exclude='*.svg' --exclude='*.png'
)

echo "== marula-core boundary checks =="

# --- Rule 3: no dependency on a repository outside this one --------------------
#
# Every dependency that names a location is resolved against the containing
# manifest and must land inside this repository and exist.
#   npm side: file:/link:/portal: resolve like the package manager does;
#     workspace:<name|rel-path> resolves against the manifests found in this
#     repository, so a phantom member fails here rather than at install time.
#   cargo side: `path = "..."` specifiers resolve manifest-relative.
# In-repo wiring (plugins/connectors/* consuming packages/sdk by file:, crates
# referencing their siblings by path) stays legal; anything that silently
# reaches across the repo boundary is a failure. desktop/resources/web is
# excluded as an untracked build well: it ships READY-MADE resources whose
# dependency aliases cannot be resolved from source.

find . -name package.json -o -name Cargo.toml 2>/dev/null \
  | grep -vE '\.git/|node_modules|/\.next/|/\.turbo/|/\.kilo/|graft/|graphify-out/|/resources/web/' \
  | sort > /tmp/boundary-manifests.$$

python3 - /tmp/boundary-manifests.$$ <<'PYEOF'
import os, re, sys

root = os.getcwd()
manifests = [m for m in open(sys.argv[1]).read().splitlines() if m]
statuses = list()

def outside(p):
    rel = os.path.relpath(os.path.normpath(p), root)
    return rel == os.pardir or rel.startswith(os.pardir + os.sep)

names = set()
for m in manifests:
    if m.endswith("package.json"):
        txt = open(m, encoding="utf-8", errors="replace").read(65536)
        got = re.search(r'"name"\s*:\s*"([^"]+)"', txt)
        if got: names.add(got.group(1))

npm = re.compile(r'"(file|link|portal|workspace):([^"]*)"')
cargo = re.compile(r'(?<=[\s{(])path\s*=\s*"([^"]*)"')
npmkey = re.compile(r'"([^"]+)"\s*:\s*"(?:file|link|portal|workspace):')

for m in manifests:
    base = os.path.dirname(m) or "."
    try:
        txt = open(m, encoding="utf-8", errors="replace").read(1048576)
    except OSError as e:
        statuses.append(f"::error file={m}::unreadable: {e}")
        continue
    pats = [npm] if m.endswith("package.json") else [cargo]
    for pat in pats:
        is_npm = pat is npm
        for match in pat.finditer(txt):
            value = match.group(2) if is_npm else match.group(1)
            proto = match.group(1) if is_npm else "path"
            line = txt[:match.start()].count("\n") + 1
            if proto == "workspace" and value.startswith("."):
                rel_like = True
                resolved = os.path.normpath(os.path.join(base, value))
            elif proto == "workspace":
                rel_like = False
                key = npmkey.search(txt[:match.end()])
                key_name = key.group(1) if key else None
                if key_name not in names:
                    statuses.append(f"::error file={m},line={line}::workspace dependency does not resolve to a manifest in this repository: \"{key_name or 'unknown'}\": \"workspace:{value}\"")
                continue
            else:
                rel_like = False
                resolved = os.path.normpath(os.path.join(base, value))
            if outside(resolved):
                statuses.append(f"::error file={m},line={line}::dependency resolves outside marula-core: \"{proto}:{value}\"")
            elif not os.path.lexists(resolved):
                statuses.append(f"::error file={m},line={line}::dependency path does not exist: \"{proto}:{value}\"")

for s in statuses:
    print(s)
if statuses:
    print("Boundary dependency resolution FAILED.")
    sys.exit(1)
PYEOF
dep_rc=$?
[ "$dep_rc" -ne 0 ] && failed=1
rm -f /tmp/boundary-manifests.$$

# Symlinks that escape the repository. macOS/runner spelling differs between
# pwd's logical path and readlink -f's physical path for the same directory,
# so both spellings of the root are accepted as "inside".
root_phys="$(readlink -f "$ROOT" 2>/dev/null || true)"
while IFS= read -r link; do
  target="$(readlink -f "$link" 2>/dev/null || true)"
  case "$target" in
    "$ROOT"/*|"$root_phys"/*|"") ;;
    *) echo "::error file=$link::symlink escapes the repository: $target"; failed=1 ;;
  esac
done < <(find . -type l \( -not -path './.git/*' -not -path '*node_modules/*' \
              -not -path '*/.next/*' \) 2>/dev/null)

# The Cargo workspace must actually resolve. A member that does not exist makes
# every cargo invocation fail, which silently turns the Rust gate into a no-op.
missing=""
if [ -f Cargo.toml ]; then
  missing=""
  while IFS= read -r member; do
    [ -n "$member" ] || continue
    [ -d "$member" ] || missing="$missing $member"
  done < <(awk '/^members *= *\[/,/^\]/' Cargo.toml \
               | grep -E '^[[:space:]]*"' | grep -oE '"[^"]+"' | tr -d '"')
fi
if [ -n "${missing:-}" ]; then
  fail "Cargo.toml" "workspace members do not exist:$missing"
fi

# --- Rules 1 and 2: source-level prohibitions --------------------------------
# The control plane is a separate repository. A core file that knows its name,
# or reaches for its private endpoints, is a leak of the closed product.
#
# A lapsed commercial licence may never gate, degrade or phone-home-lock an
# AGPL feature. Gating is the failure mode; a seam is not — so an interface that
# the commercial layer implements is allowed, and a check that refuses service
# is not. A file containing the marker `boundary-allow: reason` is exempt from
# the source scans below, and every exemption is greppable for review.
scan() { # scan <extended-regex> <message>
  local pattern="$1" message="$2"
  local hits
  hits=$(grep -rEn "$pattern" "${SOURCE_GLOBS[@]}" "${EXCLUDES[@]}" . 2>/dev/null || true)
  [ -n "$hits" ] || return 0

  # A file opts out of the source scans with a marker anywhere in it, so a
  # trait declaration and its marker do not have to sit on the same line.
  local filtered="" hit path lineno
  while IFS= read -r hit; do
    path="${hit%%:*}"
    if grep -q 'boundary-allow' "$path" 2>/dev/null; then
      continue
    fi
    rest="${hit#*:}"
    lineno="${rest%%:*}"
    echo "::error file=${path#./},line=${lineno}::${message}"
    echo "  ${hit#./}"
    filtered=1
  done <<< "$hits"

  if [ -n "$filtered" ]; then
    failed=1
    echo ""
  fi
}

scan 'marula[-_]hq' "References the proprietary control-plane repository" || true
scan 'MarlinHofmeyer|licence-server|license-server|entitlement-service' \
     "References a proprietary service" || true
scan 'require_entitlement|assert_entitled|check_entitlement|is_entitled|is_licensed|require_license|assert_licensed|license_required|check_license' \
     "Entitlement check in the open core" || true
scan 'phone_home|phone-home|phoneHome|reach_home' \
     "Phone-home call in the open core" || true
scan 'CloudPro|CloudBusiness|MarulaCloud|MarulaEnterprise|MarulaCustom' \
     "Commercial tier vocabulary in the open core" || true

if [ "$failed" -ne 0 ]; then
  echo ""
  echo "Boundary check FAILED. The open core must build and run with no commercial licence."
  exit 1
fi

echo "All boundary checks passed."
