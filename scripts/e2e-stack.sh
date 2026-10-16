#!/usr/bin/env bash
# E2E demo stack (Wave 14 lane 8 `p14-e2e-ci`; per-run orchestration Wave 37
# lane 1 `p37-env-orchestration` — design/REPORT-lane-env-instability.md §3.1–3.2).
#
# Boots the exact stack the Playwright suite runs against, fully namespaced
# PER RUN so parallel lanes/sessions never collide (no fixed ports 5432/8080/
# 8081/3000, no shared container names, no shared DB):
#
#   scripts/e2e-stack.sh              # up: migrate → seed → boot api+ws+redis,
#                                     #     write e2e/.e2e-stack.env for Playwright
#   scripts/e2e-stack.sh down [RUN]   # teardown ONE run (default: the local env
#                                     # file's run id) — only its own containers
#                                     # and PIDs, never a parallel lane's DB
#   E2E_FOREGROUND=1 scripts/e2e-stack.sh
#                                     # attach + EXIT trap tears down ONLY this run
#   E2E_KEEP=1 …                      # keep the PG container after teardown
#   E2E_FOLLOW=1 …                    # stay attached and stream the core-api +
#                                     #   realtime logs (Ctrl-C stops following;
#                                     #   the stack keeps running)
#
# `up` RETURNS as soon as core-api answers its health check. core-api and
# realtime are detached with their own stdio (`</dev/null`, logs under
# .kilo/e2e-logs/<run>/) so piping this script's output can never block — the
# old shape left both daemons holding the caller's stdout pipe, which stranded
# any `… up | tail` invocation forever (Wave 47 lane A7).
#
# `up` narrates itself: every long stage prints a label up front and its elapsed
# time when it finishes, the builds stream cargo's compile lines, and the run
# ends by printing the tail of each service log — so a boot is never silent. For
# a live view, run it again with E2E_FOLLOW=1 (or `tail -f` the log paths it
# prints; it prints them as soon as they exist).
#
# The run registry and the service logs live under the SHARED root — the main
# checkout, resolved through a worktree's `.git` link (see SHARED_ROOT below) —
# NOT beside whichever checkout ran the script, so one `scripts/dev-status.sh`
# run from `main` sees every worktree lane's stack.
#
# Every run gets: a unique run id, its own scratch Postgres container + DB on
# a random high port, its own scratch Valkey on a random high port, random API
# and WS ports, unique container names keyed by run id, a fresh 64-hex JWT
# secret + at-rest key, exported full env (SQLX_OFFLINE, REDIS_URL,
# CORS_ORIGIN) — and the run record dropped in the SHARED root's
# .kilo/dev-registry/ for scripts/dev-status.sh (see SHARED_ROOT below).
#
# Full-suite determinism (report §3.2): before boot, the seed stage is
# mechanical — `seed-demo --force` then truncate auth_lockouts/auth_attempt_log
# and reset per-user onboarding.*/tours.* rows — so every suite run starts from
# the identical demo org ("LANING reseed protocol", no longer tribal).
#
# CI note: with E2E_PG_CONTAINER or CI set, the legacy hosted-DB mode is kept
# (fixed env, no teardown trap — the API must survive into the test steps).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ENV_FILE="e2e/.e2e-stack.env"   # per-checkout: Playwright reads it from its own tree

# Shared root = the MAIN checkout, even when this script runs from a linked
# worktree. `git rev-parse --git-common-dir` resolves through the worktree's
# .git link; it may come back absolute (worktree) or relative like `.git`
# (main). The registry and service logs hang off this, so a lane's stack stays
# visible to `scripts/dev-status.sh` wherever that is run from.
gcd="$(git -C "$REPO_ROOT" rev-parse --git-common-dir 2>/dev/null || true)"
case "$gcd" in
  /*) SHARED_ROOT="$(dirname "$gcd")" ;;
  "") SHARED_ROOT="$REPO_ROOT" ;;
  *)  SHARED_ROOT="$(cd "$REPO_ROOT/$(dirname "$gcd")" && pwd)" ;;
esac
REGISTRY_DIR="${REGISTRY_DIR:-$SHARED_ROOT/.kilo/dev-registry}"
LOG_ROOT="${LOG_ROOT:-$SHARED_ROOT/.kilo/e2e-logs}"

die() { echo "[e2e-stack] FAIL: $*" >&2; exit 1; }

# ---------- stage narration ----------
# Every slow stage announces itself before it runs and reports its elapsed time
# when it finishes, so a boot never looks frozen between the container start
# and the final summary. `eval` is used only with the fixed literals below.
run_stage() { # run_stage <label> <command-string>
  # The status is returned explicitly: bash suppresses errexit for a whole
  # function body when the call sits in an `if`/`&&`, so relying on `set -e`
  # alone would let a failed stage continue silently.
  local label="$1" cmd="$2" t0=$SECONDS rc=0
  echo "[e2e-stack] $label …"
  eval "$cmd" || rc=$?
  if [ "$rc" -eq 0 ]; then
    echo "[e2e-stack] $label — done in $((SECONDS - t0))s"
  else
    echo "[e2e-stack] FAIL: $label (exit $rc)" >&2
  fi
  return "$rc"
}

# ---------- run id ----------
new_run_id() {
  printf 'e2e-%s' "$(date +%s)$RANDOM$RANDOM" | head -c 24
}

# ---------- port allocation: find a free high port ----------
free_port() {
  python3 - "$1" <<'PY'
import socket, sys
lo = int(sys.argv[1])
for p in range(lo, lo + 2000):
    try:
        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        s.bind(("127.0.0.1", p)); s.close()
        print(p); sys.exit(0)
    except OSError:
        continue
sys.exit(1)
PY
}

# ---------- registry (read by dev-status.sh) ----------
registry_drop() { # remove one run's PID entry after teardown
  rm -f "$REGISTRY_DIR/run-$1-pids.json" 2>/dev/null || true
}

# ---------- teardown (run-id-keyed) ----------
down_run() { # down_run <run_id>
  local run="$1" f
  echo "[e2e-stack] teardown run $run"
  f="$REGISTRY_DIR/run-$run-pids.json"
  if [ -f "$f" ]; then
    python3 - "$f" <<'PY'
import json, sys
try:
    d = json.load(open(sys.argv[1]))
    for svc in d.get("services", []):
        pid, name = svc.get("pid"), svc.get("name", "?")
        if pid and (svc.get("source") == "local"):
            try:
                import os, signal
                os.kill(int(pid), signal.SIGTERM)
                print(f"[e2e-stack] stopped {name} (pid {pid})")
            except (ProcessLookupError, ValueError):
                pass
except Exception:
    pass
PY
  fi
  # containers carry the run id in the NAME — decouple only what we own.
  for name in "marula-e2e-pg-$run" "marula-e2e-redis-$run"; do
    docker rm -f "$name" >/dev/null 2>&1 && echo "[e2e-stack] removed container $name" || true
  done
  # shared scratch PG: drop THIS run's DATABASE only (the shared container
  # stays for other runs). The name is run-id keyed, so it can never match a
  # different lane's database.
  if [ -n "${MARULA_SHARED_PG_CONTAINER:-}" ] && command -v docker >/dev/null 2>&1; then
    local shared="${MARULA_SHARED_PG_CONTAINER}"
    local dbname="marula_e2e_${run//[^a-z0-9]/_}"
    if docker exec "$shared" psql -U postgres -tAc \
        "select 1 from pg_database where datname='$dbname'" 2>/dev/null | grep -q 1; then
      if docker exec "$shared" psql -U postgres -c "DROP DATABASE $dbname" >/dev/null 2>&1; then
        echo "[e2e-stack] dropped database $dbname on shared pg $shared"
      else
        echo "[e2e-stack] WARN: could not drop database $dbname on shared pg $shared" >&2
      fi
    fi
  fi
  # env file: only remove it when it belongs to THIS run (never another's).
  if [ -f "$ENV_FILE" ]; then
    local frun
    frun="$(grep -m1 '^MARULA_RUN_ID=' "$ENV_FILE" | cut -d= -f2 | tr -d '"' || true)"
    if [ "$frun" = "$run" ]; then
      rm -f "$ENV_FILE"
      echo "[e2e-stack] removed $ENV_FILE (run matched)"
    else
      echo "[e2e-stack] kept $ENV_FILE (belongs to run $frun, not $run)"
    fi
  fi
  registry_drop "$run"
  local log_dir="$LOG_ROOT/$run"
  if [ -d "$log_dir" ]; then
    echo "[e2e-stack] kept service logs at $log_dir"
  fi
  echo "[e2e-stack] run $run torn down"
}

# ########## dispatch ##########
case "${1:-up}" in
  down)
    # explicit teardown: RUN comes from arg, else from the env file
    local_run="$(grep -m1 '^MARULA_RUN_ID=' "$ENV_FILE" 2>/dev/null | cut -d= -f2 | tr -d '"' || true)"
    down_run "${2:-$local_run}"
    exit 0
    ;;
  help|-h|--help)
    sed -n '1,28p' "$0"; exit 0 ;;
esac

# ########## mode selection ##########
if [ -n "${E2E_PG_CONTAINER:-}" ] || [ -n "${CI:-}" ]; then
  # Legacy hosted-DB mode (CI): env is fixed, no per-run namespacing.
  RUN_ID="${MARULA_RUN_ID:-ci}"
  DB_NAME="$(echo "${DATABASE_URL:-}" | sed 's|.*/||' || true)"
  PG_PORT=5432
  API_PORT=8080
  WS_PORT=8081
  WEB_PORT="${E2E_WEB_PORT:-3000}"
  export DATABASE_URL="${DATABASE_URL:-postgres://postgres:postgres@localhost:5432/marula_test}"
  export BIND_ADDR="${BIND_ADDR:-0.0.0.0:${API_PORT}}"
  export JWT_SECRET="${JWT_SECRET:-e2e-demo-secret-not-for-production}"
  export WEB_BASE_URL="${WEB_BASE_URL:-http://localhost:${WEB_PORT}}"
  export CORS_ORIGIN="${CORS_ORIGIN:-http://localhost:${WEB_PORT}}"
  echo "[e2e-stack] hosted mode (CI/E2E_PG_CONTAINER): fixed env, no per-run isolation"
else
  RUN_ID="$(new_run_id)"
  PG_PORT="$(free_port 25432)"
  API_PORT="$(free_port 28480)"
  WS_PORT="$(free_port 29481)"
  WEB_PORT="${E2E_WEB_PORT:-$(free_port 3000)}"
  DB_NAME="marula_e2e_${RUN_ID//[^a-z0-9]/_}"
  DB_NAME="${DB_NAME:0:63}"
  export JWT_SECRET="$(openssl rand -hex 32 2>/dev/null || true)"
  [ ${#JWT_SECRET} -eq 64 ] || die "openssl rand -hex 32 unavailable"
  export MARULA_SECRET_KEY="$JWT_SECRET" # same dev seal key as the seed
  REDIS_PORT="$(free_port 26379)"
  export REDIS_URL="redis://localhost:${REDIS_PORT}/0"
  export CORS_ORIGIN="http://localhost:${WEB_PORT}"
  export BIND_ADDR="0.0.0.0:${API_PORT}"
  export WEB_BASE_URL="http://localhost:${WEB_PORT}"
   export MARULA_DEV_TOOLS=1
   export SQLX_OFFLINE=1
   # Where the demo's attachment bytes live. The seeder needs this or it writes
   # only metadata rows and every download 404s — the api reads whatever
   # OBJECT_STORE_LOCAL_DIR points at, so the seed and the api must agree. This
   # is the same pairing `scripts/demo-up.sh` sets (Wave 47 lanes B1/B3).
   export OBJECT_STORE_LOCAL_DIR="${OBJECT_STORE_LOCAL_DIR:-./data/objects}"
   MODE=isolated
   echo "[e2e-stack] run $RUN_ID (isolated mode — unique DB/ports/names)"
   echo "[e2e-stack] object store dir $OBJECT_STORE_LOCAL_DIR"
fi

pg_ready() { pg_isready -q -d "$1" 2>/dev/null; }

if [ "${MODE:-}" = "isolated" ]; then
  ########## scratch Postgres: per-run container + per-run DB ##########
  export DATABASE_URL="postgres://postgres:postgres@localhost:${PG_PORT}/${DB_NAME}"
  docker rm -f "marula-e2e-pg-$RUN_ID" >/dev/null 2>&1 || true
  echo "[e2e-stack] starting scratch Postgres marula-e2e-pg-$RUN_ID on port $PG_PORT"
  docker run -d --name "marula-e2e-pg-$RUN_ID" \
    -e POSTGRES_USER=postgres -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=marula_test \
    -p "$PG_PORT:5432" pgvector/pgvector:pg18 >/dev/null \
    || die "could not start scratch Postgres (docker down?)"
  # `pg_isready` reports the SERVER, not the database: a connection to a DB that
  # does not exist still returns "accepting connections" (it logs a FATAL and
  # moves on). So wait on `postgres` here, then create the per-run DB loudly —
  # the image only initializes POSTGRES_DB.
  PG_SERVER_URL="postgres://postgres:postgres@localhost:${PG_PORT}/postgres"
  for _ in $(seq 1 40); do
    pg_ready "$PG_SERVER_URL" && break; sleep 1
  done
  pg_ready "$PG_SERVER_URL" || die "scratch Postgres never became ready (port $PG_PORT)"
  if ! docker exec "marula-e2e-pg-$RUN_ID" psql -U postgres -c "CREATE DATABASE $DB_NAME" \
      >/dev/null 2>&1; then
    # Already existing is fine (this run booted before); anything else is fatal.
    docker exec "marula-e2e-pg-$RUN_ID" psql -U postgres -tAc \
      "select 1 from pg_database where datname='$DB_NAME'" 2>/dev/null | grep -q 1 \
      || die "could not create database $DB_NAME on marula-e2e-pg-$RUN_ID"
  fi
  # NOTE: no EXIT trap removing the container — the default `up` mode leaves
  # the stack running for Playwright; teardown is `e2e-stack.sh down $RUN_ID`
  # (keyed to this run id) or E2E_FOREGROUND's own trap.

  ########## scratch Valkey (redis name kept — REDIS_URL contract) ##########
  docker rm -f "marula-e2e-redis-$RUN_ID" >/dev/null 2>&1 || true
  echo "[e2e-stack] starting scratch Valkey marula-e2e-redis-$RUN_ID on port $REDIS_PORT"
  docker run -d --name "marula-e2e-redis-$RUN_ID" \
    -p "$REDIS_PORT:6379" valkey/valkey:8-alpine \
    valkey-server --maxmemory 128mb --maxmemory-policy noeviction >/dev/null \
    || die "could not start scratch Valkey"
fi

########## migrate + deterministic demo seed (mechanical reseed) ##########
run_stage "migrate schema" 'cargo run -q -p marula-core-cli -- migrate'
run_stage "seed the deterministic demo org (seed-demo --force)" \
  'MARULA_DEV_TOOLS=1 cargo run -q -p marula-core-cli -- seed-demo --force'

echo "[e2e-stack] reset volatile state (auth gate + per-user onboarding/tours) …"
if [ -n "${E2E_PG_CONTAINER:-}" ]; then
  PG_EXEC=(docker exec "${E2E_PG_CONTAINER}")
else
  PG_EXEC=(docker exec "marula-e2e-pg-$RUN_ID")
fi
"${PG_EXEC[@]}" psql -U postgres -d "$(echo "$DATABASE_URL" | sed 's|.*/||')" -q -c \
  'TRUNCATE auth_lockouts; TRUNCATE auth_attempt_log;
   DELETE FROM settings
    WHERE scope = '"'"'user'"'"' AND (key LIKE '"'"'onboarding.%'"'"' OR key LIKE '"'"'tours.%'"'"');' \
  || die "could not reset volatile demo state"

########## core-api ##########
# Service stdio goes to per-run log files with stdin from /dev/null: the
# daemons must not inherit this script's stdout, or a caller that pipes `up`
# into a pager waits on an fd the daemons hold open forever.
LOG_DIR="$LOG_ROOT/$RUN_ID"
mkdir -p "$LOG_DIR"
echo "[e2e-stack] service logs: $LOG_DIR/{core-api,realtime}.log"
# The release builds are the slow part of a boot; cargo runs un-quiet here so
# its compile lines stream and the wait is visible.
run_stage "build core-api (release — E2E latency needs the optimized build)" \
  'cargo build -p core-api --release'
BIN="$(cargo metadata -q --no-deps --format-version 1 | python3 -c \
  'import json,sys; print(json.load(sys.stdin)["target_directory"])')/release/core-api"
echo "[e2e-stack] boot core-api on ${BIND_ADDR}"
"$BIN" >"$LOG_DIR/core-api.log" 2>&1 </dev/null &
API_PID=$!
disown "$API_PID" 2>/dev/null || true
# realtime (ws) service — same JWT secret as core-api (token validation).
run_stage "build realtime (release)" 'cargo build -p realtime --release'
WS_BIN="$(cargo metadata -q --no-deps --format-version 1 | python3 -c \
  'import json,sys; print(json.load(sys.stdin)["target_directory"])')/release/realtime"
echo "[e2e-stack] boot realtime on 0.0.0.0:${WS_PORT}"
BIND_ADDR="0.0.0.0:${WS_PORT}" "$WS_BIN" >"$LOG_DIR/realtime.log" 2>&1 </dev/null &
WS_PID=$!
disown "$WS_PID" 2>/dev/null || true

API_HOST="$(echo "$BIND_ADDR" | sed 's/^[0-9]*://; s/^0\.0\.0\.0/localhost/')"
echo "[e2e-stack] waiting for core-api health on ${API_HOST} (up to 60s) …"
for _ in $(seq 1 60); do
  curl -fsS "http://${API_HOST}/api/v1/health" >/dev/null 2>&1 && break
  sleep 1
done
curl -fsS "http://${API_HOST}/api/v1/health" >/dev/null \
  || die "core-api never became healthy on ${API_HOST} — see $LOG_DIR/core-api.log"
echo "[e2e-stack] core-api healthy — demo env ready (owner@demo.local / demo-password-123)"

# ########## env file for Playwright (kills NEXT_PUBLIC_API_URL drift) ##########
cat > "$ENV_FILE" <<EOF
MARULA_RUN_ID=$RUN_ID
E2E_API_URL=http://${API_HOST}/
E2E_API_PORT=$API_PORT
E2E_WEB_PORT=$WEB_PORT
E2E_WEB_URL=http://localhost:$WEB_PORT
E2E_WS_PORT=$WS_PORT
E2E_WS_URL=ws://localhost:$WS_PORT/ws
E2E_DATABASE_URL=$DATABASE_URL
DATABASE_URL=$DATABASE_URL
BIND_ADDR=$BIND_ADDR
JWT_SECRET=$JWT_SECRET
MARULA_SECRET_KEY=$MARULA_SECRET_KEY
REDIS_URL=$REDIS_URL
CORS_ORIGIN=$CORS_ORIGIN
OBJECT_STORE_LOCAL_DIR=$OBJECT_STORE_LOCAL_DIR
EOF
chmod 600 "$ENV_FILE"
echo "[e2e-stack] wrote $ENV_FILE (run $RUN_ID; shared with Playwright + e2e-build-web)"

# ########## pid registry for dev-status.sh ##########
FINGERPRINT="$(printf '%s' "$JWT_SECRET" | cut -c1-8)-$(echo "$DATABASE_URL" | sed 's|.*/||')-$API_PORT"
python3 - "$ENV_FILE" "$API_PID" "$WS_PID" "$REGISTRY_DIR" "$RUN_ID" "$FINGERPRINT" "$LOG_DIR" <<'PY' || true
import json, os, sys, time
envfile, api, ws, regdir, run_id, fingerprint, log_dir = sys.argv[1:8]
env = {}
for line in open(envfile):
    line = line.strip()
    if "=" in line:
        k, v = line.split("=", 1)
        env[k] = v
reg = {
    "run_id": run_id,
    "started_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
    "env_fingerprint": fingerprint,
    "log_dir": log_dir,
    "env": env,
    "services": [
        {"name": "core-api", "pid": int(api), "source": "local", "port": env.get("E2E_API_PORT", ""),
         "log": os.path.join(log_dir, "core-api.log")},
        {"name": "realtime", "pid": int(ws), "source": "local", "port": env.get("E2E_WS_PORT", ""),
         "log": os.path.join(log_dir, "realtime.log")},
    ],
}
os.makedirs(regdir, exist_ok=True)
with open(os.path.join(regdir, f"run-{run_id}-pids.json"), "w") as f:
    json.dump(reg, f, indent=2)
PY
echo "[e2e-stack] registered run in $REGISTRY_DIR — check scripts/dev-status.sh"

########## console summary — a boot is never silent ##########
echo "[e2e-stack] run $RUN_ID ready — api http://${API_HOST}/  ws :${WS_PORT}  web :${WEB_PORT}"
for _svc in core-api realtime; do
  echo "[e2e-stack] --- $_svc log (last 5 lines) ---"
  tail -n 5 "$LOG_DIR/$_svc.log" 2>/dev/null | sed 's/^/    /' || true
done

if [ "${E2E_FOLLOW:-0}" = "1" ]; then
  # Live view: stay attached and stream both logs. Ctrl-C stops following ONLY —
  # the stack keeps running; teardown stays `e2e-stack.sh down $RUN_ID`.
  echo "[e2e-stack] following service logs — Ctrl-C stops following (stack keeps running)"
  tail -n 0 -f "$LOG_DIR/core-api.log" "$LOG_DIR/realtime.log" || true
  exit 0
fi

if [ "${E2E_FOREGROUND:-0}" = "1" ]; then
  # attach mode: EXIT trap tears down ONLY this run id.
  trap 'down_run "$RUN_ID"' EXIT
  wait "$API_PID"
fi
