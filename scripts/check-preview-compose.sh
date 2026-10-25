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
#   2. Exactly two interfaces are published (web, api) — the database
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

# The base file requires the signing key, the at-rest key and the two
# machine-seam tokens since Wave 56, and the two database passwords plus the
# Garage and GlitchTip credentials since Wave 58; the overlay no longer
# renames them. The values are dummies: this asserts the merge and the
# posture, never a real deployment.
export JWT_SECRET=check-preview-not-a-real-secret
export MARULA_SECRET_KEY=check-preview-not-a-real-key
export MARULA_INTERNAL_TOKEN=check-preview-seam
export EMAIL_INTERNAL_TOKEN=check-preview-email-seam
export POSTGRES_PASSWORD=check-preview-owner
export MARULA_APP_PASSWORD=check-preview-app
export GARAGE_RPC_SECRET=check-preview-garage-rpc
export GLITCHTIP_SECRET_KEY=check-preview-glitchtip
export PREVIEW_CORS_ORIGIN=http://preview.invalid
export PREVIEW_PUBLIC_URL=http://preview.invalid
export PREVIEW_API_URL=http://preview.invalid:8080

echo "== 1. the overlay refuses to render without its required secrets =="
# A `?`-guarded variable that is unset makes `docker compose config` fail.
# That failure IS the assertion, so it is provoked deliberately. The guards
# live in the base file now, which is what makes a preview env inherit them.
if (unset JWT_SECRET; docker compose -f "$BASE" -f "$OVERLAY" config >/dev/null 2>&1); then
  fail "compose rendered with JWT_SECRET unset — the ?-guard is not load-bearing"
else
  pass "compose refuses to render without JWT_SECRET"
fi
if (unset MARULA_SECRET_KEY; docker compose -f "$BASE" -f "$OVERLAY" config >/dev/null 2>&1); then
  fail "compose rendered with MARULA_SECRET_KEY unset"
else
  pass "compose refuses to render without MARULA_SECRET_KEY"
fi

echo "== 2. merged config renders =="
# Rendered as JSON, not YAML: every assertion below parses it, and JSON costs
# no PyYAML dependency (a check that needs an uninstalled module reports a
# failure it cannot distinguish from a posture regression).
MERGED="$(mktemp)"
BASEJSON="$(mktemp)"
trap 'rm -f "$MERGED" "$BASEJSON"' EXIT
if ! docker compose -f "$BASE" -f "$OVERLAY" config --format json > "$MERGED" 2>/dev/null; then
  fail "merged config did not render"
  exit 1
fi
if ! docker compose -f "$BASE" config --format json > "$BASEJSON" 2>/dev/null; then
  fail "base config did not render"
  exit 1
fi
pass "merged config renders"

echo "== 3. the base's hardening survived the merge =="
# Asserted on the merged result, which is the only place it is true.
read -r CAPDROP SNO NEWPRIV <<<"$(python3 - "$MERGED" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
s = d.get("services", {})
def has(svc, key, want):
    v = (s.get(svc) or {}).get(key)
    return v is not None and want in (v if isinstance(v, list) else [v])
# Only the services the overlay names; the rest are untouched base services.
named = [n for n in ("core-api", "worker", "web", "postgres", "redis")]
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

echo "== 4. exactly two interfaces are published =="
python3 - "$MERGED" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
s = d.get("services", {})
published = {}
for name, svc in s.items():
    for p in (svc.get("ports") or []):
        host = p.get("host_ip") or "0.0.0.0"
        published.setdefault(name, []).append(f"{host}:{p.get('published')}")
loopback = ("127.0.0.1", "::1")
want = {"core-api", "web"}
ok = True

# Anything on a wildcard address is reachable by whoever reaches the host.
wild = {n for n, binds in published.items()
        if any(not b.startswith(loopback) for b in binds)}
if wild != want:
    print(f"  ✗ wildcard-published services are {sorted(wild)}, expected {sorted(want)}")
    ok = False
else:
    print("  ✓ only web and api are published off loopback")

# A loopback publish is a diagnostic surface for whoever has the shell:
# allowed, and a different thing from publishing on 0.0.0.0.
looped = sorted(set(published) - wild)
if looped:
    print(f"  ✓ loopback only for {', '.join(looped)}")

for leak in ("postgres", "worker", "migrate", "garage", "mailpit"):
    if leak in published:
        print(f"  ✗ {leak} is published at all — the overlay's !override is not load-bearing")
        ok = False
if "postgres" not in published and "worker" not in published:
    print("  ✓ postgres and the worker health port are unpublished")

# The two preview publishes must be 0.0.0.0: a preview that is unreachable
# is not a preview, and a narrower bind only works for its author.
for name in sorted(wild & want):
    for b in published[name]:
        if not b.startswith("0.0.0.0:"):
            print(f"  ✗ {name} publishes on {b}, not 0.0.0.0")
            ok = False
if ok and wild == want:
    print("  ✓ every preview interface is bound to 0.0.0.0")
sys.exit(0 if ok else 1)
PY
[ $? -eq 0 ] || FAILED=1

echo "== 5. no third-party service rode along =="
# The overlay must not add a database, cache or object store. A new
# dependency in a preview env is a supply-chain decision, not a merge detail.
if python3 - "$MERGED" "$BASEJSON" <<'PY'
import json, sys
base = json.load(open(sys.argv[2]))["services"]
merged = json.load(open(sys.argv[1]))["services"]
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
