#!/usr/bin/env sh
# Assert that the edge is the only thing listening on a non-loopback address.
#
# Why a script: the overlay reclaims published ports by REPLACING the base
# file's port lists, and compose only applies that when a container is
# recreated. Two failures follow, and neither is visible in a file:
#
#   1. A profile slot the overlay forgot. The base file publishes on every
#      interface for a laptop; the overlay has to name every one of those
#      services, and a slot added later is silent until someone scans a
#      public host.
#   2. A container that was never recreated. `docker compose restart` does
#      not re-read config, so a stack that had the overlay added by restart
#      still holds the base publishes while the proxy runs beside them.
#
# The default mode catches (1) on the MERGED config, which is the only place
# the answer is true. `--live` catches (2) on the containers that are
# actually up, and prints the one command that fixes it.
#
# Usage:
#   deploy/edge/check-published-ports.sh          # merged config, all profiles
#   deploy/edge/check-published-ports.sh --live   # the running stack
#
# Requires: docker compose (no daemon for the default mode), python3.
# Costs: nothing — no image is built and no container is started.

set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$ROOT"

BASE=deploy/docker-compose.yml
EDGE=deploy/edge/compose.edge.yml
# Every profile, so a slot's publishes are in the answer. A profile that is
# off still renders, which is what makes the check complete rather than
# "complete for the profiles I happened to try".
PROFILES="--profile edge --profile stage2 --profile observability --profile ai --profile backup"
# deploy/dev.env carries every required secret, and none of them is used
# for anything but interpolating the file. A real operator's values would do
# the same; these are the ones in the repository, so the check needs no
# environment of its own.
export MARULA_DOMAIN=check.invalid
export ACME_EMAIL=ops@check.invalid

fail() { printf '  FAIL %s\n' "$*"; }
ok()   { printf '  ok   %s\n' "$*"; }

mode=${1:-config}
case "$mode" in
config|--live) : ;;
*)
	echo "usage: $0 [--live]" >&2
	exit 2
	;;
esac

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

if [ "$mode" = config ]; then
	# shellcheck disable=SC2086
	if ! docker compose --env-file deploy/dev.env -f "$BASE" $PROFILES config --format json >"$tmpdir/base.json" 2>"$tmpdir/err"; then
		echo "the base file did not render:" >&2
		cat "$tmpdir/err" >&2
		exit 1
	fi
	# shellcheck disable=SC2086
	if ! docker compose --env-file deploy/dev.env -f "$BASE" -f "$EDGE" $PROFILES config --format json >"$tmpdir/merged.json" 2>"$tmpdir/err"; then
		echo "the merged config did not render:" >&2
		cat "$tmpdir/err" >&2
		exit 1
	fi
	python3 - "$tmpdir/base.json" "$tmpdir/merged.json" <<'PY' || exit 1
import json
import sys

base = json.load(open(sys.argv[1]))["services"]
merged = json.load(open(sys.argv[2]))["services"]


def hosts(service):
    out = []
    for p in service.get("ports") or []:
        host = p.get("host_ip") or "0.0.0.0"
        out.append((host, str(p.get("published")), str(p.get("target"))))
    return out


def loopback(host):
    return host in ("127.0.0.1", "::1", "localhost")


problems = []

# Every service the base publishes is a service this overlay has to answer
# for. A slot added to the base and forgotten here is the whole finding.
publishing = {name for name, svc in base.items() if hosts(svc)}
for name in sorted(publishing):
    if name == "edge":
        continue
    kept = hosts(merged.get(name) or {})
    leaked = [b for b in kept if not loopback(b[0])]
    if leaked:
        shown = ", ".join(f"{h}:{pub}->{tgt}" for h, pub, tgt in leaked)
        problems.append(
            f"{name} still publishes on a non-loopback address ({shown}) — "
            "add the service to the overlay's reclaim list"
        )

# The four roles nobody inspects from a browser are unpublished outright.
for name in ("core-api", "web", "worker", "postgres"):
    if hosts(merged.get(name) or {}):
        problems.append(
            f"{name} is published — the overlay's `ports: !override []` is gone"
        )

# And the front door is the only wildcard listener, on both ports.
edge = hosts(merged.get("edge") or {})
wild = [b for b in edge if not loopback(b[0])]
for port in ("80", "443"):
    if not any(b[1] == port for b in wild):
        problems.append(f"edge does not publish {port} on a wildcard address")

if problems:
    for p in problems:
        print(f"  FAIL {p}")
    sys.exit(1)

print(f"  ok   {len(publishing) - 1} base publishes reclaimed: "
      "unpublished or loopback-only")
print("  ok   core-api, web, worker and postgres are unpublished")
print("  ok   edge is the only listener on a non-loopback address (80, 443)")
PY
	exit 0
fi

# ---- --live ---------------------------------------------------------------
# Read the containers that are actually up. `ps` is scoped to this project
# name (`marula`), so a lane's own stack elsewhere is not reported as a leak.
# shellcheck disable=SC2086
if ! docker compose --env-file deploy/dev.env -f "$BASE" -f "$EDGE" $PROFILES ps --format json >"$tmpdir/ps.json" 2>"$tmpdir/err"; then
	echo "docker compose ps failed:" >&2
	cat "$tmpdir/err" >&2
	exit 1
fi
# shellcheck disable=SC2086
live_pid=$(python3 - "$tmpdir/ps.json" <<'PY'
import json
import sys

raw = open(sys.argv[1]).read().strip()
if not raw:
    rows = []
else:
    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError:
        # Compose has emitted both a JSON array and one object per line.
        parsed = [json.loads(line) for line in raw.splitlines() if line.strip()]
    rows = parsed if isinstance(parsed, list) else [parsed]

if not rows:
    print("empty")
    raise SystemExit(0)

problems = []
edge_seen = []
for row in rows:
    service = row.get("Service") or row.get("service") or "?"
    name = row.get("Name") or row.get("name") or service
    leaked = []
    for pub in row.get("Publishers") or []:
        url = pub.get("URL") or ""
        published = pub.get("PublishedPort") or 0
        if not published or url in ("", "127.0.0.1", "::1", "localhost"):
            continue
        if service == "edge":
            edge_seen.append(str(published))
            continue
        # The same publish shows up once per address family (0.0.0.0 and ::),
        # which is one finding, not two.
        binding = f"{published} -> {pub.get('TargetPort')}/tcp"
        if binding not in leaked:
            leaked.append(binding)
    if leaked:
        problems.append((service, name, leaked))

for service, name, leaked in problems:
    print(f"  FAIL {name} ({service}) publishes {', '.join(leaked)} "
          "on a non-loopback address")
    print("  FIX: docker compose --env-file deploy/dev.env "
          "-f deploy/docker-compose.yml -f deploy/edge/compose.edge.yml "
          f"--profile edge up -d --force-recreate {service}")

if problems:
    print("  That port is reachable from another host while the edge is the thing")
    print("  the deployment trusts. Either this stack was started without the")
    print("  overlay, or its containers were never recreated after it joined the")
    print("  command line — `docker compose restart` does not re-read config.")
    raise SystemExit(1)

if edge_seen:
    print(f"  ok   edge publishes {', '.join(sorted(set(edge_seen)))} on a wildcard address")
else:
    print("  ok   no container publishes on a non-loopback address")
print(f"  ok   {len(rows)} running container(s) checked: every other listener is "
      "unpublished or loopback-only")
PY
) || {
	echo "$live_pid"
	exit 1
}
if [ "$live_pid" = "empty" ]; then
	echo "  note no running stack under project name 'marula' — nothing to read."
	exit 0
fi
echo "$live_pid"
