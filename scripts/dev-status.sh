#!/usr/bin/env bash
# Process registry for agents (report §3.5): list the running Marula stack
# — pids, ports, env fingerprint — so a lane stops guessing which process it
# may kill. Sources of truth:
#   1. run-*-pids.json entries written by scripts/e2e-stack.sh, read from the
#      SHARED root's .kilo/dev-registry/ (the main checkout) PLUS any linked
#      worktree's own registry — so a stack booted inside a lane worktree is
#      still visible from here.
#   2. live docker containers matching marula-e2e-* (image pgvector/valkey)
#   3. listening ports held by the registered processes (re-checked live)
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)" 2>/dev/null
cd "$REPO_ROOT"

# Same shared-root resolution as scripts/e2e-stack.sh (see SHARED_ROOT there):
# from a worktree, `git rev-parse --git-common-dir` resolves through the .git
# link to the main checkout, where the registry and service logs live.
gcd="$(git -C "$REPO_ROOT" rev-parse --git-common-dir 2>/dev/null || true)"
case "$gcd" in
  /*) SHARED_ROOT="$(dirname "$gcd")" ;;
  "") SHARED_ROOT="$REPO_ROOT" ;;
  *)  SHARED_ROOT="$(cd "$REPO_ROOT/$(dirname "$gcd")" && pwd)" ;;
esac

alive() { kill -0 "$1" 2>/dev/null && echo yes || echo no; }

echo "== registered e2e-stack runs (run-*-pids.json) =="
reg_files=()
for pat in "$SHARED_ROOT/.kilo/dev-registry/run-"*-pids.json \
           "$SHARED_ROOT/.kilo/worktrees/"*"/.kilo/dev-registry/run-"*-pids.json; do
  for f in $pat; do
    [ -f "$f" ] && reg_files+=("$f")
  done
done
if [ "${#reg_files[@]}" -gt 0 ]; then
  for f in "${reg_files[@]}"; do
    origin="$SHARED_ROOT"
    case "$f" in
      */worktrees/*)
        origin="worktree $(echo "$f" | sed "s|^$SHARED_ROOT/.kilo/worktrees/||; s|/.kilo/dev-registry/.*||")" ;;
    esac
    python3 - "$f" "$origin" <<'PY'
import os, json, sys
d = json.load(open(sys.argv[1]))
origin = sys.argv[2]
env = d.get("env", {})
print(f"\nrun {d.get('run_id','?')}  started {d.get('started_at','?')}  "
      f"fingerprint {d.get('env_fingerprint','?')}  [{origin}]")
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
if d.get("log_dir"):
    print(f"  logs         : {d['log_dir']}")
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
  echo "Service logs:  $SHARED_ROOT/.kilo/e2e-logs/<run-id>/{core-api,realtime}.log"
else
  echo "  (docker unavailable)"
fi

echo
echo "== other common Marula processes (core-api / realtime / next standalone) =="
ps -axo pid,command | grep -E "release/(core-api|realtime)|\.next/standalone/server\.js|marula-core-cli" \
  | grep -v grep | sed 's/^/  /' || echo "  (none)"
echo
echo "Teardown: bash scripts/e2e-stack.sh down <RUN_ID>   # keyed to that run id only"
