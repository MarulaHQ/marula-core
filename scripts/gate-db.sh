#!/usr/bin/env bash
# gate-db.sh — the migrated database and Valkey a workspace gate needs (Wave 47
# lane A28 `p47-gate-db-prerequisite`).
#
# `cargo check --workspace --all-targets` compiles sqlx's `query!` macros against
# a LIVE database, and three worker tests need Valkey. Without them the check
# fails with ~667 `relation "..." does not exist` errors and the tests fail with
# `connect redis: Connection refused` — neither is a defect in the change under
# test. This script gives a group both services in one namespaced run, the same
# way scripts/e2e-stack.sh owns the runtime stack.
#
#   scripts/gate-db.sh up
#       Boot per-run Postgres (pgvector/pgvector:pg18) and Valkey
#       (valkey/valkey:8), create a run-keyed database, apply EVERY migration in
#       crates/core-db/migrations, write gate-db-<run>.env, and register the run
#       so scripts/dev-status.sh sees it. Prints the file to source.
#
#   scripts/gate-db.sh down [RUN]
#       Remove ONLY that run's containers, database and env file. Accepts the
#       run id or its bare digits. A down that removes nothing exits non-zero.
#
#   scripts/gate-db.sh doctor [--log FILE]
#       Report whether the environment is gate-ready. With --log, scan a failing
#       gate log for the two false-failure signatures and print the remedy
#       instead of leaving a lane to debug its own code.
#
#   scripts/gate-db.sh selftest
#       Prove the teardown honesty rules against a fake docker (no daemon).
#
# The migration set grows; nothing here hardcodes a count. `up` applies pending
# migrations and `doctor` compares the applied rows in `_sqlx_migrations` with
# the files on disk, so a new migration is "one pending" rather than a surprise.
#
# Both services are required for the group gate matrix in the private parallel-
# lanes reference: Postgres for sqlx compile-time macros, Valkey for the three
# worker tests that connect to REDIS_URL. Neither is optional.
#
# Run ids are `gatedb-<digits>`; containers are marula-gate-pg-<run> and
# marula-gate-redis-<run>. The registry and env files live under the SHARED root
# (the main checkout) so a run booted from a lane worktree is visible from main.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

gcd="$(git -C "$REPO_ROOT" rev-parse --git-common-dir 2>/dev/null || true)"
case "$gcd" in
  /*) SHARED_ROOT="$(dirname "$gcd")" ;;
  "") SHARED_ROOT="$REPO_ROOT" ;;
  *)  SHARED_ROOT="$(cd "$REPO_ROOT/$(dirname "$gcd")" && pwd)" ;;
esac

REGISTRY_DIR="${REGISTRY_DIR:-$SHARED_ROOT/.kilo/dev-registry}"
ENV_DIR="${GATE_DB_ENV_DIR:-$REPO_ROOT/.kilo}"
MIGRATIONS_DIR="$REPO_ROOT/crates/core-db/migrations"
DOCKER="${DOCKER:-docker}"
PG_IMAGE="${GATE_DB_PG_IMAGE:-pgvector/pgvector:pg18}"
REDIS_IMAGE="${GATE_DB_REDIS_IMAGE:-valkey/valkey:8}"

die() { echo "[gate-db] FAIL: $*" >&2; exit 1; }
note() { echo "[gate-db] $*"; }

new_run_id() {
  printf 'gatedb-%s' "$(date +%s)$RANDOM$RANDOM" | head -c 28
}

normalize_run_id() {
  case "${1:-}" in
    "") return 1 ;;
    *[!0-9]*) printf '%s' "$1" ;;
    *) printf 'gatedb-%s' "$1" ;;
  esac
}

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

container_exists() { "$DOCKER" inspect "$1" >/dev/null 2>&1; }
container_remove() { "$DOCKER" rm -f -v "$1" >/dev/null 2>&1; }

env_file_for() { printf '%s/gate-db-%s.env' "$ENV_DIR" "$1"; }
latest_env_file() { printf '%s/gate-db.env' "$ENV_DIR"; }

registry_file() { printf '%s/run-%s-pids.json' "$REGISTRY_DIR" "$1"; }

source_hint() {
  local run="$1" file
  file="$(env_file_for "$run")"
  echo "  set -a; . $file; set +a"
  if [ "$file" != "$(latest_env_file)" ]; then
    echo "  # or, when only one gate run is live: . $(latest_env_file)"
  fi
}

print_unmigrated_remedy() {
  local run="${1:-}"
  cat >&2 <<EOF
[gate-db] This is the unmigrated-database signature, not a defect in the code
[gate-db] under test. sqlx compile-time macros report every table as missing
[gate-db] when DATABASE_URL points at a database with no migrations applied.
[gate-db]
[gate-db] Fix the environment:
[gate-db]   bash scripts/gate-db.sh up
EOF
  if [ -n "$run" ]; then
    echo "[gate-db]   set -a; . $(env_file_for "$run"); set +a" >&2
  else
    echo "[gate-db]   set -a; . $(latest_env_file); set +a" >&2
  fi
  cat >&2 <<'EOF'
[gate-db]
[gate-db] Then rerun the cargo command. If the log also says
[gate-db] "connect redis: Connection refused", the same run supplies Valkey on
[gate-db] REDIS_URL; check `scripts/gate-db.sh doctor`.
EOF
}

print_redis_remedy() {
  cat >&2 <<EOF
[gate-db] Valkey is missing or unreachable. The worker tests that need it fail
[gate-db] with "connect redis: Connection refused" before reaching their own
[gate-db] assertions. Boot the run and export REDIS_URL:
[gate-db]   bash scripts/gate-db.sh up
[gate-db]   set -a; . $(latest_env_file); set +a
EOF
}

scan_log() {
  local log="$1" rel redis
  [ -f "$log" ] || die "no such log: $log"
  rel="$(grep -cE 'relation "?[A-Za-z0-9_]+"? does not exist' "$log" 2>/dev/null || true)"
  redis="$(grep -cE 'connect redis: Connection refused|Redis connection refused|Connection refused.*6379' "$log" 2>/dev/null || true)"
  echo "[gate-db] log scan: $log"
  echo "[gate-db]   relation-does-not-exist  ${rel:-0}"
  echo "[gate-db]   redis-connection-refused ${redis:-0}"
  if [ "${rel:-0}" -ge 5 ]; then
    print_unmigrated_remedy
    return 1
  fi
  if [ "${redis:-0}" -ge 1 ]; then
    print_redis_remedy
    return 1
  fi
  echo "[gate-db] no known false-failure signature in this log"
  return 0
}

migration_files() {
  find "$MIGRATIONS_DIR" -maxdepth 1 -type f -name '*.sql' | wc -l | tr -d ' '
}

applied_migrations() {
  "$DOCKER" exec "$1" psql -U postgres -d "$2" -tAc \
    "select count(*) from _sqlx_migrations" 2>/dev/null | tr -d ' '
}

require_tools() {
  command -v "$DOCKER" >/dev/null 2>&1 || die "docker is not available — install Docker Desktop or point DOCKER at a working CLI"
  command -v sqlx >/dev/null 2>&1 || die "sqlx-cli is not available — cargo install sqlx-cli --no-default-features --features postgres"
  command -v python3 >/dev/null 2>&1 || die "python3 is not available (used for port allocation)"
}

cmd_up() {
  require_tools
  [ -d "$MIGRATIONS_DIR" ] || die "no migrations at $MIGRATIONS_DIR"

  local run pg_port redis_port db_name pg_name redis_name
  run="$(new_run_id)"
  pg_port="$(free_port 35432)"
  redis_port="$(free_port 36379)"
  db_name="marula_gate_${run//[^a-z0-9]/_}"
  db_name="${db_name:0:63}"
  pg_name="marula-gate-pg-$run"
  redis_name="marula-gate-redis-$run"

  note "run $run (per-run DB, ports and container names)"
  container_remove "$pg_name"
  note "starting Postgres $pg_name on 127.0.0.1:$pg_port"
  "$DOCKER" run -d --name "$pg_name" \
    --label "marula.gate-db=1" --label "marula.gate-db.run=$run" \
    -e POSTGRES_USER=postgres -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=postgres \
    -p "127.0.0.1:$pg_port:5432" "$PG_IMAGE" >/dev/null \
    || die "could not start Postgres (docker down?)"

  local server_url="postgres://postgres:postgres@127.0.0.1:${pg_port}/postgres"
  for _ in $(seq 1 60); do
    "$DOCKER" exec "$pg_name" pg_isready -q -U postgres -d postgres >/dev/null 2>&1 && break
    sleep 1
  done
  "$DOCKER" exec "$pg_name" pg_isready -q -U postgres -d postgres >/dev/null 2>&1 \
    || { "$DOCKER" rm -f -v "$pg_name" >/dev/null 2>&1 || true; die "Postgres never became ready on port $pg_port"; }

  "$DOCKER" exec "$pg_name" psql -U postgres -c "CREATE DATABASE $db_name" >/dev/null 2>&1 \
    || "$DOCKER" exec "$pg_name" psql -U postgres -tAc \
         "select 1 from pg_database where datname='$db_name'" 2>/dev/null | grep -q 1 \
    || { "$DOCKER" rm -f -v "$pg_name" >/dev/null 2>&1 || true; die "could not create database $db_name"; }

  note "starting Valkey $redis_name on 127.0.0.1:$redis_port"
  container_remove "$redis_name"
  "$DOCKER" run -d --name "$redis_name" \
    --label "marula.gate-db=1" --label "marula.gate-db.run=$run" \
    -p "127.0.0.1:$redis_port:6379" "$REDIS_IMAGE" \
    valkey-server --maxmemory 128mb --maxmemory-policy noeviction >/dev/null \
    || { "$DOCKER" rm -f -v "$pg_name" >/dev/null 2>&1 || true; die "could not start Valkey"; }

  local database_url="postgres://postgres:postgres@127.0.0.1:${pg_port}/${db_name}"
  local redis_url="redis://127.0.0.1:${redis_port}/0"

  note "applying migrations from crates/core-db/migrations …"
  if ! DATABASE_URL="$database_url" sqlx migrate run --source "$MIGRATIONS_DIR" >/tmp/gate-db-migrate-$$.log 2>&1; then
    sed 's/^/    /' /tmp/gate-db-migrate-$$.log >&2 || true
    rm -f /tmp/gate-db-migrate-$$.log
    "$DOCKER" rm -f -v "$pg_name" "$redis_name" >/dev/null 2>&1 || true
    die "migrations failed — see the output above"
  fi
  rm -f /tmp/gate-db-migrate-$$.log

  local files applied
  files="$(migration_files)"
  applied="$(applied_migrations "$pg_name" "$db_name")"
  [ "${applied:-0}" = "$files" ] || \
    die "migration count mismatch: $applied applied, $files files — run doctor"

  local redis_ping
  redis_ping="$("$DOCKER" exec "$redis_name" valkey-cli ping 2>/dev/null || true)"
  [ "$redis_ping" = "PONG" ] || die "Valkey did not answer PING (got ${redis_ping:-nothing})"

  mkdir -p "$ENV_DIR" "$REGISTRY_DIR"
  local env_file latest
  env_file="$(env_file_for "$run")"
  latest="$(latest_env_file)"
  cat > "$env_file" <<EOF
# gate-db run $run — source this, do not commit it.
GATE_DB_RUN_ID=$run
DATABASE_URL=$database_url
PGHOST=127.0.0.1
PGPORT=$pg_port
PGUSER=postgres
PGPASSWORD=postgres
PGDATABASE=$db_name
REDIS_URL=$redis_url
GATE_DB_MIGRATIONS_APPLIED=$applied
# A sourced env file cannot express "unset", and a stale SQLX_OFFLINE=1 would
# hide the live schema this run exists to provide.
unset SQLX_OFFLINE
EOF
  chmod 600 "$env_file"
  cp "$env_file" "$latest"
  chmod 600 "$latest"

  python3 - "$env_file" "$REGISTRY_DIR" "$run" "$pg_name" "$redis_name" "$pg_port" "$redis_port" <<'PY'
import json, os, sys, time
envfile, regdir, run_id, pg, redis, pg_port, redis_port = sys.argv[1:8]
env = {}
for line in open(envfile):
    line = line.strip()
    if "=" in line and not line.startswith("#"):
        k, v = line.split("=", 1)
        env[k] = v
reg = {
    "run_id": run_id,
    "kind": "gate-db",
    "started_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
    "env_fingerprint": f"gate-db-{run_id}",
    "env": env,
    "services": [
        {"name": "postgres", "source": "docker", "container": pg, "port": pg_port},
        {"name": "valkey", "source": "docker", "container": redis, "port": redis_port},
    ],
}
os.makedirs(regdir, exist_ok=True)
with open(os.path.join(regdir, f"run-{run_id}-pids.json"), "w") as f:
    json.dump(reg, f, indent=2)
PY

  note "run $run ready — Postgres ${applied}/${files} migrations, Valkey PONG"
  note "source the env file before any workspace cargo command:"
  source_hint "$run"
  note "teardown: bash scripts/gate-db.sh down $run"
  note "doctor:   bash scripts/gate-db.sh doctor"
}

cmd_down() {
  local raw run rc=0 removed=0 pg_name redis_name
  raw="${1:-}"
  if [ -z "$raw" ]; then
    raw="$(grep -m1 '^GATE_DB_RUN_ID=' "$(latest_env_file)" 2>/dev/null | cut -d= -f2 | tr -d '"' || true)"
    [ -n "$raw" ] || die "no run id given and $(latest_env_file) holds none — pass one, e.g. bash scripts/gate-db.sh down gatedb-1790000000000000000"
    note "run id $raw (from $(latest_env_file))"
  fi
  run="$(normalize_run_id "$raw")"
  [ "$run" = "$raw" ] || note "run id $raw normalised to $run"
  pg_name="marula-gate-pg-$run"
  redis_name="marula-gate-redis-$run"

  note "teardown run $run"
  for name in "$pg_name" "$redis_name"; do
    if container_exists "$name"; then
      if container_remove "$name"; then
        note "removed container $name"
        removed=$((removed + 1))
      else
        echo "[gate-db] WARN container $name exists but could not be removed" >&2
      fi
    else
      note "already gone: $name"
    fi
  done
  if [ "$removed" -eq 0 ]; then
    echo "[gate-db] FAIL: nothing matched $pg_name or $redis_name — no container was removed" >&2
    echo "[gate-db]       see what is live with: $DOCKER ps --filter name=marula-gate" >&2
    rc=1
  fi

  local file
  file="$(env_file_for "$run")"
  if [ -f "$file" ]; then rm -f "$file"; note "removed $file"; fi
  if [ -f "$(latest_env_file)" ]; then
    local frun
    frun="$(grep -m1 '^GATE_DB_RUN_ID=' "$(latest_env_file)" | cut -d= -f2 | tr -d '"' || true)"
    if [ "$frun" = "$run" ]; then
      rm -f "$(latest_env_file)"
      note "removed $(latest_env_file) (run matched)"
    else
      note "kept $(latest_env_file) (belongs to run $frun)"
    fi
  fi
  rm -f "$(registry_file "$run")"
  [ "$rc" -eq 0 ] && note "run $run torn down"
  return "$rc"
}

cmd_doctor() {
  local log="" run="" env_file latest db_url redis_url pg_name redis_name
  while [ $# -gt 0 ]; do
    case "$1" in
      --log) log="${2:-}"; shift 2 ;;
      *) die "doctor: unknown argument $1" ;;
    esac
  done
  if [ -n "$log" ]; then
    scan_log "$log"; return $?
  fi

  latest="$(latest_env_file)"
  if [ -f "$latest" ]; then
    run="$(grep -m1 '^GATE_DB_RUN_ID=' "$latest" | cut -d= -f2 | tr -d '"' || true)"
    env_file="$latest"
  else
    echo "[gate-db] no env file at $latest"
  fi
  echo "[gate-db] doctor — run ${run:-none}"
  command -v "$DOCKER" >/dev/null 2>&1 && echo "  docker       ok" || echo "  docker       MISSING"
  command -v sqlx >/dev/null 2>&1 && echo "  sqlx-cli     ok" || echo "  sqlx-cli     MISSING (cargo install sqlx-cli --no-default-features --features postgres)"

  if [ -n "$env_file" ]; then
    # shellcheck disable=SC1090
    . "$env_file"
    db_url="${DATABASE_URL:-}"
    redis_url="${REDIS_URL:-}"
    pg_name="marula-gate-pg-$run"
    redis_name="marula-gate-redis-$run"
    if container_exists "$pg_name"; then
      local applied files
      applied="$(applied_migrations "$pg_name" "${PGDATABASE:-postgres}" || true)"
      files="$(migration_files)"
      echo "  postgres     up ($pg_name), migrations ${applied:-?}/${files}"
      [ "${applied:-0}" = "$files" ] || echo "  postgres     PENDING migrations — run: bash scripts/gate-db.sh up"
    else
      echo "  postgres     DOWN or not this run's container ($pg_name)"
    fi
    if container_exists "$redis_name"; then
      echo "  valkey       up ($redis_name), PING $("$DOCKER" exec "$redis_name" valkey-cli ping 2>/dev/null || echo 'no answer')"
    else
      echo "  valkey       DOWN or not this run's container ($redis_name)"
    fi
    if command -v psql >/dev/null 2>&1 && [ -n "$db_url" ]; then
      psql "$db_url" -tAc "select 1" >/dev/null 2>&1 && echo "  DATABASE_URL reachable" || echo "  DATABASE_URL NOT reachable"
    fi
    if [ -n "$redis_url" ]; then
      echo "  REDIS_URL    $redis_url"
    else
      echo "  REDIS_URL    unset"
    fi
    echo "  source       set -a; . $env_file; set +a"
  fi

  if [ -n "$run" ] && container_exists "marula-gate-pg-$run" && container_exists "marula-gate-redis-$run"; then
    echo "[gate-db] gate-ready"
    return 0
  fi
  echo "[gate-db] not gate-ready — fix the lines above, then rerun doctor"
  return 1
}

selftest_teardown() {
  local tmp failures=0 checks=0
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/gate-db-selftest.XXXXXX")" || return 1
  trap "rm -rf '$tmp'" EXIT
  cat > "$tmp/docker" <<'FAKE'
#!/usr/bin/env bash
state="${FAKE_DOCKER_STATE:?}"
[ -n "${FAKE_DOCKER_LOG:-}" ] && printf '%s\n' "$*" >> "$FAKE_DOCKER_LOG"
case "${1:-}" in
  inspect) grep -qx "${2:-}" "$state" ;;
  rm)      name="${@: -1}"
           if grep -qx "$name" "$state"; then grep -vx "$name" "$state" > "$state.tmp"; mv "$state.tmp" "$state"; echo "$name"; exit 0; fi
           exit 0 ;;
  ps)      cat "$state" ;;
  *)       exit 0 ;;
esac
FAKE
  chmod +x "$tmp/docker"

  run_case() {
    local label="$1" want="$2" state="$3" arg="$4"
    : > "$tmp/state"
    if [ -n "$state" ]; then printf '%s\n' $state > "$tmp/state"; fi
    local rc=0
    DOCKER="$tmp/docker" FAKE_DOCKER_STATE="$tmp/state" \
      GATE_DB_ENV_DIR="$tmp/env" REGISTRY_DIR="$tmp/reg" \
      bash "$0" down "$arg" >/dev/null 2>&1 || rc=$?
    checks=$((checks + 1))
    if [ "$rc" != "$want" ]; then
      echo "  FAIL  $label: exit $rc, wanted $want"
      failures=$((failures + 1))
    fi
  }

  run_case removes_one 0 "marula-gate-pg-gatedb-1" gatedb-1
  run_case leaves_siblings 1 "marula-gate-pg-gatedb-2" gatedb-1
  run_case missing_is_a_failure 1 "" gatedb-1
  run_case bare_digits_normalise 0 "marula-gate-pg-gatedb-3" 3

  if [ "$failures" -ne 0 ]; then
    echo "gate-db selftest: FAIL ($failures of $checks)" >&2
    return 1
  fi
  echo "gate-db selftest: ok ($checks checks)"
}

case "${1:-help}" in
  up) cmd_up ;;
  down) shift; cmd_down "${1:-}" ;;
  doctor) shift; cmd_doctor "$@" ;;
  selftest) selftest_teardown ;;
  help|-h|--help) sed -n '1,55p' "$0" ;;
  *) die "unknown command '${1}' — up | down [RUN] | doctor [--log FILE] | selftest" ;;
esac
