#!/usr/bin/env bash
# Process registry for agents (report §3.5): list the running Meridian stack
# — pids, ports, env fingerprint — so a lane stops guessing which process it
# may kill. Sources of truth:
#   1. .kilo/dev-registry/run-*-pids.json — per-run entries written by
#      scripts/e2e-stack.sh
#   2. live docker containers matching marula-e2e-* (image pgvector/valkey)
#   3. listening ports held by the registered processes (re-checked live)
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)" 2>/dev/null
cd "$REPO_ROOT"

alive() { kill -0 "$1" 2>/dev/null && echo yes || echo no; }

echo "== registered e2e-stack runs (.kilo/dev-registry/) =="
if compgen -G ".kilo/dev-registry/run-*-pids.json" >/dev/null; then
  for f in .kilo/dev-registry/run-*-pids.json; do
    python3 - "$f" <<'PY'
import os, json, sys
d = json.load(open(sys.argv[1]))
env = d.get("env", {})
print(f"\nrun {d.get('run_id','?')}  started {d.get('started_at','?')}  "
      f"fingerprint {d.get('env_fingerprint','?')}")
print(f"  DATABASE_URL : {env.get('E2E_DATABASE_URL','?')}")
print(f"  JWT_SECRET   : {env.get('JWT_SECRET','?')[:12]}… (fingerprint field — keep secret)")
for s in d.get("services", []):
    pid = s.get("pid")
    state = "gone"
    try:
        os.kill(int(pid), 0)
        state = f"alive pid {pid}"
    except (ProcessLookupError, TypeError, ValueError):
        pass
    except PermissionError:
        state = f"alive pid {pid} (foreign)"
    print(f"  {s.get('name', '?'):10} port={s.get('port', '?'):6} {state}")
PY
  done
else
  echo "  (none)"
fi

echo
echo "== live marula-e2e-* containers =="
if command -v docker >/dev/null 2>&1; then
  docker ps --filter "name=marula-e2e" --format \
    "table {{.Names}}\t{{.Image}}\t{{.Ports}}\t{{.Status}}" 2>/dev/null \
    | { read -r header; echo "  $header"; while read -r l; do echo "  $l"; done; }
  echo
  echo "To inspect:  docker logs marula-e2e-pg-<run-id>"
else
  echo "  (docker unavailable)"
fi

echo
echo "== other common Meridian processes (core-api / realtime / next standalone) =="
ps -axo pid,command | grep -E "release/(core-api|realtime)|\.next/standalone/server\.js|marula-core-cli" \
  | grep -v grep | sed 's/^/  /' || echo "  (none)"
echo
echo "Teardown: bash scripts/e2e-stack.sh down <RUN_ID>   # keyed to that run id only"
