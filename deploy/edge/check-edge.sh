#!/usr/bin/env sh
# Edge/TLS config validation — the light checks, runnable without booting
# anything.
#
# Why a script and not a checklist: the three TLS modes are three Caddyfiles
# that MUST route the same table, one compose overlay whose port overrides
# MUST win, and two copies of Cloudflare's published ranges (the default
# TRUSTED_PROXIES in the overlay and the origin firewall helper) that MUST
# stay equal to the versioned constant core-api carries. Every one of those
# is a drift waiting to happen between two config files that never run
# together until the day they do.
#
# Run it:   deploy/edge/check-edge.sh
# Requires: python3, docker compose (skipped with a warning when absent),
#           helm (skipped with a warning when absent)
# Does NOT: start a container, build an image, or touch the network.

set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
EDGE="$ROOT/deploy/edge"
FAIL=0
PASS=0

ok()   { PASS=$((PASS + 1)); printf '  ok   %s\n' "$*"; }
bad()  { FAIL=$((FAIL + 1)); printf '  FAIL %s\n' "$*"; }
skip() { printf '  skip %s\n' "$*"; }
head() { printf '\n== %s\n' "$*"; }

have() { command -v "$1" >/dev/null 2>&1; }

[ -f "$EDGE/marula-routes.caddy" ] || { echo "run from the repo: deploy/edge/check-edge.sh" >&2; exit 2; }

# ---------------------------------------------------------------------------
head "Caddyfiles"
for mode in direct cf-origin cf-dns01; do
	file="$EDGE/Caddyfile.$mode"
	[ -f "$file" ] || { bad "Caddyfile.$mode is missing"; continue; }
	ok "Caddyfile.$mode exists"
done

python3 - "$EDGE" <<'PY' || FAIL=$((FAIL + 1))
import re, sys, pathlib
edge = pathlib.Path(sys.argv[1])
problems = []

routes = (edge / "marula-routes.caddy").read_text()
modes = {m: (edge / f"Caddyfile.{m}").read_text()
         for m in ("direct", "cf-origin", "cf-dns01")}

# The shared table every mode must import.
# The `tls` stanza is the one place the modes differ, and each mode's shape
# is asserted exactly: direct mode has NO tls directive at all (automatic
# HTTPS is Caddy's default for a real hostname, and a bare `tls` line does
# not parse — the directive requires an argument), cf-origin names a
# certificate pair, cf-dns01 names a DNS provider.
tls_expected = {"direct": 0, "cf-origin": 1, "cf-dns01": 1}
for name, text in modes.items():
    if "import marula-routes.caddy" not in text:
        problems.append(f"{name}: does not import marula-routes.caddy")
    if "https://{$MARULA_DOMAIN" not in text:
        problems.append(f"{name}: site address is not https://{{$MARULA_DOMAIN...}}")
    if "admin 127.0.0.1:2019" not in text:
        problems.append(f"{name}: admin API is not pinned to IPv4 loopback "
                        "(busybox wget resolves `localhost` to ::1 first, so the "
                        "container healthcheck would never pass)")
    tls = re.findall(r"^\s*tls(?:\s|$)", text, re.M)
    if len(tls) != tls_expected[name]:
        problems.append(f"{name}: expected {tls_expected[name]} `tls` stanza(s), found {len(tls)}")
    if name == "direct" and re.search(r"^\s*tls\s*$", text, re.M):
        problems.append("direct: a bare `tls` line is a parse error — delete it")

def one(name, needle):
    if needle not in text_for(name):
        problems.append(f"{name}: missing {needle!r}")

def text_for(name):
    return modes[name]

one("direct", "acme_ca")
one("cf-origin", "CADDY_CERTS_DIR")
one("cf-dns01", "CADDY_CERTS_DIR") if False else None
one("cf-origin", "origin.crt")
one("cf-origin", "origin.key")
one("cf-dns01", "dns cloudflare")
one("cf-dns01", "propagation_delay")

# The routing table itself: the seams, the body ceiling, and the one path
# that must never be routed.
for needle in ("max_size {$MARULA_MAX_UPLOAD_MB:25}MiB",
               "reverse_proxy @api {$CORE_API_UPSTREAM:core-api:8080}",
               "reverse_proxy {$WEB_UPSTREAM:web:3000}",
               "trusted_proxies private_ranges"):
    if needle not in routes:
        problems.append(f"marula-routes.caddy: missing {needle!r}")
if re.search(r"(^|\s)internal\b", routes):
    problems.append("marula-routes.caddy: mentions 'internal' — keep /internal unrouted")
if "path /api /api/* /openapi.json" not in routes:
    problems.append("marula-routes.caddy: api matcher does not cover /api, /api/*, /openapi.json")
# /healthz must be a REWRITE onto core-api's real liveness route, not a
# passthrough: core-api registers {base}/health, so a bare /healthz is a 404
# with an empty body, and a probe that always fails is worse than no probe.
# MiB, not MB: core-api converts with mb * 1024 * 1024 while Caddy's MB is
# decimal, so `MB` here caps the proxy ~1.2 MB below the API's limit.
if "max_size {$MARULA_MAX_UPLOAD_MB:25}MB" in routes:
    problems.append("marula-routes.caddy: body ceiling uses MB (decimal); "
                    "core-api's MARULA_MAX_UPLOAD_MB is MiB (mb * 1024 * 1024)")
if "path /healthz" not in routes or "rewrite * /api/health" not in routes:
    problems.append("marula-routes.caddy: /healthz is not rewritten onto /api/health")
if "path /api /api/* /openapi.json /healthz" in routes:
    problems.append("marula-routes.caddy: /healthz is passed through to core-api, which 404s it")

# Balanced braces in every file (a Caddyfile missing one fails at parse).
for name, text in list(modes.items()) + [("marula-routes.caddy", routes)]:
    if text.count("{") != text.count("}"):
        problems.append(f"{name}: unbalanced braces ({text.count('{')} open, {text.count('}')} close)")

if problems:
    for p in problems:
        print(f"  FAIL {p}")
    sys.exit(1)
print(f"  ok   3 mode files + the shared route table are structurally consistent")
PY

head "Cloudflare ranges: overlay, firewall helper, and core-api agree"
python3 - "$ROOT" <<'PY' || FAIL=$((FAIL + 1))
import re, sys, pathlib
root = pathlib.Path(sys.argv[1])
problems = []

src = (root / "crates/core-api/src/client_ip.rs").read_text()
match = re.search(r"const CLOUDFLARE_IP_RANGES: &\[&str\] = &\[(.*?)\];", src, re.S)
if not match:
    print("  FAIL could not find the CLOUDFLARE_IP_RANGES constant in core-api")
    sys.exit(1)
core = set(re.findall(r'"([^"]+)"', match.group(1)))

overlay = set(re.findall(r"\b\d{1,3}(?:\.\d{1,3}){3}/\d{1,2}\b"
                         r"|\b[0-9a-f:]{3,}/\d{1,3}\b",
                         (root / "deploy/edge/compose.edge.yml").read_text(), re.I))
helper = set(re.findall(r"\b\d{1,3}(?:\.\d{1,3}){3}/\d{1,2}\b"
                        r"|\b[0-9a-f:]{3,}/\d{1,3}\b",
                        (root / "deploy/edge/origin-lockdown.sh").read_text(), re.I))

# Cloudflare's ranges must be identical in all three places: the constant
# is the baseline, and a config that lags it by one range is an origin that
# is either locked out or unlocked.
private = {"127.0.0.0/8", "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16",
           "::1/128", "fc00::/7", "fe80::/10"}
for name, got in (("compose.edge.yml", overlay), ("origin-lockdown.sh", helper)):
    missing = core - got
    extra = got - core - private - {"0.0.0.0/32"}
    if missing:
        problems.append(f"{name}: missing {len(missing)} Cloudflare range(s): {sorted(missing)[:4]}")
    if extra:
        problems.append(f"{name}: {len(extra)} CIDR(s) that are neither Cloudflare's nor the documented compose ranges: {sorted(extra)[:4]}")

if problems:
    for p in problems:
        print(f"  FAIL {p}")
    sys.exit(1)
print(f"  ok   all {len(core)} Cloudflare ranges match across core-api, the overlay and the firewall helper")
PY

# ---------------------------------------------------------------------------
head "Shell"
if have shellcheck; then
	if shellcheck -s sh "$EDGE/origin-lockdown.sh" && shellcheck -s sh "$0"; then
		ok "shellcheck clean (origin-lockdown.sh, check-edge.sh)"
	else
		bad "shellcheck reported problems"
	fi
else
	skip "shellcheck not installed"
fi
for f in "$EDGE/origin-lockdown.sh" "$0"; do
	if sh -n "$f" 2>/dev/null; then
		ok "sh -n $(basename "$f")"
	else
		bad "sh -n $(basename "$f")"
	fi
done

head "Origin lockdown plan"
if "$EDGE/origin-lockdown.sh" --plan --backend nft >/dev/null 2>&1; then
	ok "--plan renders an nft ruleset"
else
	bad "--plan failed"
fi
if "$EDGE/origin-lockdown.sh" --direct >/dev/null 2>&1; then
	ok "--direct explains Direct mode without touching the host"
else
	bad "--direct failed"
fi

# ---------------------------------------------------------------------------
head "Compose"
if have docker; then
	run_cfg() {
		label=$1; shift
		if out=$(cd "$ROOT" && env "$@" docker compose -f deploy/docker-compose.yml -f deploy/edge/compose.edge.yml --profile edge config 2>&1); then
			ok "$label interpolates"
		else
			bad "$label: $out"
		fi
	}
	run_cfg "direct mode"  MARULA_DOMAIN=marula.example.com ACME_EMAIL=ops@example.com
	run_cfg "cf-origin"     MARULA_DOMAIN=marula.example.com MARULA_TLS_MODE=cf-origin
	run_cfg "cf-dns01"      MARULA_DOMAIN=marula.example.com MARULA_TLS_MODE=cf-dns01 CF_API_TOKEN=tok

	if out=$(cd "$ROOT" && docker compose -f deploy/docker-compose.yml -f deploy/edge/compose.edge.yml --profile edge config 2>&1); then
		bad "compose started without MARULA_DOMAIN — it should refused"
	else
		case "$out" in
		*MARULA_DOMAIN*) ok "compose refuses to start without MARULA_DOMAIN" ;;
		*) bad "the failure does not name MARULA_DOMAIN: $out" ;;
		esac
	fi

	if (cd "$ROOT" && PREVIEW_JWT_SECRET=j PREVIEW_MARULA_SECRET_KEY=k \
		PREVIEW_CORS_ORIGIN=https://pr-1.preview.example.com \
		PREVIEW_PUBLIC_URL=https://pr-1.preview.example.com \
		MARULA_DOMAIN=pr-1.preview.example.com \
		docker compose -f deploy/docker-compose.yml \
		               -f deploy/preview/compose.preview.yml \
		               -f deploy/edge/compose.edge.yml --profile edge config >/dev/null 2>&1); then
		ok "preview + edge layering interpolates"
	else
		bad "preview + edge layering failed"
	fi
else
	skip "docker compose not available"
fi

head "The overlay actually takes the ports back"
python3 - "$ROOT" <<'PY' || FAIL=$((FAIL + 1))
import re, sys, pathlib
src = (pathlib.Path(sys.argv[1]) / "deploy/edge/compose.edge.yml").read_text()
problems = []
for svc in ("core-api", "web", "postgres", "worker"):
    block = re.search(rf"^  {re.escape(svc)}:\n((?:    .*\n|\n)*?)(?=^  \S|^\S|\Z)", src, re.M)
    if not block:
        problems.append(f"no {svc} block in the overlay")
    elif "!override []" not in block.group(1):
        problems.append(f"{svc}: no `ports: !override []` — its base publish stays reachable")
if "edgedata:" not in src:
    problems.append("the edgedata volume for the certificate store is not declared")
if problems:
    for p in problems:
        print(f"  FAIL {p}")
    sys.exit(1)
print("  ok   core-api, web, postgres and worker are all pulled back")
PY

head "Certificate material"
if [ -d "$EDGE/certs" ] && [ -f "$EDGE/certs/.gitignore" ]; then
	ok "certs/ exists and ignores itself"
	for f in origin.crt origin.key; do
		if [ -f "$EDGE/certs/$f" ]; then
			ok "certs/$f present"
		else
			skip "certs/$f not present (needed for cf-origin only)"
		fi
	done
else
	bad "certs/ or its .gitignore is missing — mounted material could be committed"
fi

head "Dockerfile pins"
python3 - "$EDGE" <<'PY' || FAIL=$((FAIL + 1))
import re, sys, pathlib
text = (pathlib.Path(sys.argv[1]) / "Dockerfile.caddy-dns").read_text()
problems = []
for stage in re.findall(r"^FROM (\S+)", text, re.M):
    if "@sha256:" not in stage:
        problems.append(f"unpinned base: {stage}")
if "github.com/caddy-dns/cloudflare@" not in text:
    problems.append("the DNS module is not pinned to a tag")
if problems:
    for p in problems:
        print(f"  FAIL {p}")
    sys.exit(1)
print("  ok   both build stages are digest-pinned and the DNS module is tag-pinned")
PY

# ---------------------------------------------------------------------------
head "Helm"
if have helm; then
	vals=$(mktemp /tmp/marula-edge-values.XXXXXX.yaml)
	cat >"$vals" <<'EOF'
ingress:
  enabled: true
  className: nginx
  host: marula.example.com
  tlsSecretName: marula-tls
coreApi:
  trustedProxies: "173.245.48.0/20,104.16.0.0/13"
web:
  nextPublicApiUrl: https://marula.example.com
EOF
	if helm lint "$ROOT/deploy/helm/marula" >/dev/null 2>&1; then
		ok "helm lint"
	else
		bad "helm lint failed"
	fi
	if helm template marula "$ROOT/deploy/helm/marula" -f "$vals" --set digestPinningEnforced=false \
		--set nonMarulaDigestPinningEnforced=false >/tmp/marula-edge-template.yaml 2>&1; then
		ok "helm template renders with the ingress on"
		# Parse the Ingress document, not the whole render: the worker's
		# readinessProbe legitimately carries `path: /healthz`, so a grep
		# over the render cannot tell the ingress's routes apart from its —
		# it reported a pass for a route that was gone.
		if python3 "$EDGE/check-ingress.py" /tmp/marula-edge-template.yaml; then
			ok "the ingress routes exactly /api, /openapi.json and / (no /healthz)"
		else
			bad "the ingress paths are wrong (see above)"
		fi
		for ann in proxy-body-size proxy-read-timeout proxy-send-timeout proxy-buffering; do
			if grep -q "$ann" /tmp/marula-edge-template.yaml; then
				ok "ingress annotation $ann"
			else
				bad "ingress annotation $ann missing"
			fi
		done
	else
		bad "helm template failed: $(tail -2 /tmp/marula-edge-template.yaml)"
	fi
	rm -f "$vals" /tmp/marula-edge-template.yaml
else
	skip "helm not available"
fi

# ---------------------------------------------------------------------------
printf '\n%d passed, %d failed\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ] || exit 1
printf 'Light checks clean. The heavy legs (a real caddy validate per mode,\n'
printf 'the image build, and an edge boot in front of a running stack) are\n'
printf 'recorded in internal/design/qa-p47-edge-tls/ and are re-runnable from there.\n'
