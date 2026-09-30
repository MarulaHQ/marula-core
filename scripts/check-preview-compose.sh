#!/usr/bin/env bash
# Merged-posture check for the per-PR preview compose overlay.
#
# `trivy config` reads one file in isolation, so the overlay looks
# under-hardened on its own (see trivy.ignore.yaml for why that is declined
# rather than "fixed"). This script closes the loop the other way: it
# renders the MERGED config and asserts the posture on the merged result,
# which is what actually gets deployed.
#
# The assertions are the ones a static scan would have made, plus the two
# that only the merge can show:
#
#   1. The base's hardening survived the merge on every service.
#   2. Exactly three interfaces are published (web, api, ws) — the database
#      and cache are NOT, which a per-file scan could not tell either way.
#   3. The two required secrets are still required (a `?`-guard is a compose
#      parse error, so this is asserted by expecting the failure).
#
# Usage: scripts/check-preview-compose.sh
# Exits 0 when the merged posture is what the README claims.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

BASE=deploy/docker-compose.yml
OVERLAY=deploy/preview/compose.preview.yml
FAILED=0

fail() { echo "  ✗ $1"; FAILED=1; }
pass() { echo "  ✓ $1"; }

for f in "$BASE" "$OVERLAY"; do
  [ -f "$f" ] || { echo "missing $f"; exit 1; }
done

# The overlay's ?-guards demand these. Values are dummies: this asserts the
# merge and the posture, never a real deployment.
export PREVIEW_JWT_SECRET=check-preview-not-a-real-secret
export PREVIEW_MARULA_SECRET_KEY=check-preview-not-a-real-key
export PREVIEW_CORS_ORIGIN=http://preview.invalid
export PREVIEW_PUBLIC_URL=http://preview.invalid
export PREVIEW_API_URL=http://preview.invalid:8080

echo "== 1. the overlay refuses to render without its required secrets =="
# A `?`-guarded variable that is unset makes `docker compose config` fail.
# That failure IS the assertion, so it is provoked deliberately.
if (unset PREVIEW_JWT_SECRET; docker compose -f "$BASE" -f "$OVERLAY" config >/dev/null 2>&1); then
  fail "compose rendered with PREVIEW_JWT_SECRET unset — the ?-guard is not load-bearing"
else
  pass "compose refuses to render without PREVIEW_JWT_SECRET"
fi
if (unset PREVIEW_MARULA_SECRET_KEY; docker compose -f "$BASE" -f "$OVERLAY" config >/dev/null 2>&1); then
  fail "compose rendered with PREVIEW_MARULA_SECRET_KEY unset"
else
  pass "compose refuses to render without PREVIEW_MARULA_SECRET_KEY"
fi

echo "== 2. merged config renders =="
MERGED="$(mktemp)"
trap 'rm -f "$MERGED"' EXIT
if ! docker compose -f "$BASE" -f "$OVERLAY" config > "$MERGED" 2>/dev/null; then
  fail "merged config did not render"
  exit 1
fi
pass "merged config renders"

echo "== 3. the base's hardening survived the merge =="
# Asserted on the merged result, which is the only place it is true.
read -r CAPDROP SNO NEWPRIV <<<"$(python3 - "$MERGED" <<'PY'
import sys, yaml
d = yaml.safe_load(open(sys.argv[1]))
s = d.get("services", {})
def has(svc, key, want):
    v = (s.get(svc) or {}).get(key)
    return v is not None and want in (v if isinstance(v, list) else [v])
# Only the services the overlay names; the rest are untouched base services.
named = [n for n in ("core-api", "realtime", "worker", "web", "postgres", "redis")]
cap = all(has(n, "cap_drop", "ALL") for n in named)
def has_nnp(svc):
    # Compose normalises this to a bare "no-new-privileges" on output (the
    # ":true" form is input syntax), so accept either. Both mean the same
    # thing to the runtime.
    opts = (s.get(svc) or {}).get("security_opt") or []
    if isinstance(opts, str):
        opts = [opts]
    return any(str(o).split(":")[0].strip() == "no-new-privileges" for o in opts)
sno = all(has_nnp(n) for n in named)
print("yes" if cap else "no", "yes" if sno else "no", "")
PY
)"
[ "$CAPDROP" = "yes" ] && pass "cap_drop ALL on every overlay service" || fail "a service lost cap_drop ALL in the merge"
[ "$SNO" = "yes" ] && pass "no-new-privileges on every overlay service" || fail "a service lost no-new-privileges in the merge"

echo "== 4. exactly three interfaces are published =="
python3 - "$MERGED" <<'PY'
import sys, yaml
d = yaml.safe_load(open(sys.argv[1]))
s = d.get("services", {})
published = {}
for name, svc in s.items():
    for p in (svc.get("ports") or []):
        published.setdefault(name, []).append(f"{p.get('host_ip')}:{p.get('published')}")
want = {"core-api", "realtime", "web"}
got = set(published)
ok = True
if got != want:
    print(f"  ✗ published services are {sorted(got)}, expected {sorted(want)}")
    ok = False
else:
    print("  ✓ only web, api and ws are published")
for leak in ("postgres", "redis", "worker", "migrate", "garage", "mailpit"):
    if leak in got:
        print(f"  ✗ {leak} is published on the host — the overlay's !override is not load-bearing")
        ok = False
if ok:
    print("  ✓ postgres, redis and the worker health port are unpublished")
# The published ones must be on 0.0.0.0 (a preview that is unreachable is
# not a preview) and on their own ports.
for name, binds in published.items():
    for b in binds:
        if not b.startswith("0.0.0.0:"):
            print(f"  ✗ {name} publishes on {b}, not 0.0.0.0")
            ok = False
if ok:
    print("  ✓ every published interface is bound to 0.0.0.0")
sys.exit(0 if ok else 1)
PY
[ $? -eq 0 ] || FAILED=1

echo "== 5. no third-party service rode along =="
# The overlay must not add a database, cache or object store. A new
# dependency in a preview env is a supply-chain decision, not a merge detail.
if python3 - "$MERGED" <<'PY'
import sys, yaml
base = yaml.safe_load(open("deploy/docker-compose.yml"))["services"]
merged = yaml.safe_load(open(sys.argv[1]))["services"]
added = sorted(set(merged) - set(base))
if added:
    print(f"  ✗ the overlay added services: {added}")
    sys.exit(1)
print("  ✓ the overlay adds no new services")
PY
then :; else FAILED=1; fi

if [ "$FAILED" -ne 0 ]; then
  echo
  echo "preview compose posture check FAILED"
  exit 1
fi
echo
echo "preview compose posture OK — the merged config is what the README claims"
