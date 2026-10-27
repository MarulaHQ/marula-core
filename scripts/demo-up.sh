#!/usr/bin/env bash
# demo-up.sh — the one-command localhost demo boot (Wave 47 lane B1,
# `p47-demo-up-script`).
#
# Replaces the retired machine-local `dev-up.local.sh` (gitignored, stale:
# it probed `pg_isready -U meridian` while compose sets the `marula` role,
# wrote logs to /tmp/meridian-dev, and predated MARULA_DEV_TOOLS). Every
# role it had is absorbed here: idempotent bring-up, stop, reset, port
# awareness, and a disk gate.
#
# What one run does, in order:
#   1. preflight (docker daemon, cargo, pnpm, ports, disk)
#   2. docker compose bring-up of postgres (15432) + redis (16379) from
#      deploy/docker-compose.yml — the ports the self-host reference pins
#   3. marula-core-cli migrate  (as the table OWNER role, never marula_app)
#   4. MARULA_DEV_TOOLS=1 marula-core-cli seed-demo --force  (first boot
#      only — a second `up` never wipes operator edits; `reset` reseeds)
#   5. cargo build + boot core-api (8080) / worker (8082)
#   6. web dev server on 3000 (pnpm --filter @marula-core/web dev; pass
#      --web prod for a next build + standalone start instead)
#   7. print the four demo logins, the BYOK checklist URL and the dev-ops
#      URL
#
# Usage:
#   scripts/demo-up.sh                 # up (the default)
#   scripts/demo-up.sh --ai            # up + the local Ollama ai profile
#   scripts/demo-up.sh stop            # stop the three app processes
#   scripts/demo-up.sh stop --infra    # ... and the compose postgres/redis
#   scripts/demo-up.sh reset           # reseed the demo org (keeps the stack)
#   scripts/demo-up.sh reset --fresh   # destroy the DB volume, re-up, reseed
#   scripts/demo-up.sh help            # the usage block
#
# Idempotency: compose `up -d` and the port probes make a second run a
# no-op that reports what is already up. Nothing is double-booted; the
# demo org is never reseeded by `up` once it exists.
#
# Secrets posture: nothing secret is written anywhere. The native API and
# worker leave JWT_SECRET and MARULA_SECRET_KEY UNSET so every process uses
# the documented dev fallback (MARULA_DEV_TOOLS=1, pinned dev secret — the
# same derivation the desktop test app and the compose dev values use), which
# keeps the API and the seeder on one key. The compose file this script
# starts postgres and redis from requires the four secrets to interpolate, so
# the calls below pass `--env-file deploy/dev.env`: those values reach the
# containers only, and postgres/redis read none of them. A local JWT_SECRET
# that is not 64 hex chars makes core-api refuse to boot, so the preflight
# fails with that message instead of a half-up stack.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

COMPOSE_FILE="deploy/docker-compose.yml"
# The base file ships no secret value and refuses to render without four of
# them (Wave 56). This script starts only postgres and redis from compose, so
# the published dev values are enough for interpolation; the native API and
# worker it launches afterwards read the exported environment, not this file.
compose() { docker compose --env-file "$REPO_ROOT/deploy/dev.env" -f "$COMPOSE_FILE" "$@"; }
STATE_DIR="$REPO_ROOT/.demo-up"
LOGDIR="$STATE_DIR/logs"
PIDSFILE="$STATE_DIR/pids"
OLLAMA_OVERRIDE="$STATE_DIR/ollama-host.yml"

PG_PORT=15432
REDIS_PORT=16379
API_PORT=8080
WORKER_PORT=8082
WEB_PORT=3000

DB_OWNER="postgres://marula:marula@localhost:${PG_PORT}/marula"
# The role is minted by migration 0114 with this literal, so the demo keeps
# working out of the box; an operator who rotated it (see deploy/README.md)
# still gets a working demo by exporting the new value.
DB_APP="postgres://marula_app:${MARULA_APP_PASSWORD:-marula_app}@localhost:${PG_PORT}/marula"
REDIS_URL="redis://localhost:${REDIS_PORT}/0"
API_LOCAL="http://127.0.0.1:${API_PORT}"
API_BROWSER="http://localhost:${API_PORT}"
WEB_URL="http://localhost:${WEB_PORT}"

# The dev-tools gate every demo surface needs (seed-demo refuses without
# it; core-api falls back to the pinned dev secret with it).
export MARULA_DEV_TOOLS=1
# The repo ships the .sqlx cache; compile without a live database.
export SQLX_OFFLINE=1
# Keep the API's local object store inside the gitignored state dir, not
# the working tree.
export OBJECT_STORE=local
export OBJECT_STORE_LOCAL_DIR="$STATE_DIR/objects"
export EMAIL_TRANSPORT=none
export CORS_ORIGIN="$WEB_URL"

AI=0
WEB_MODE=dev
WITH_INFRA=0
FRESH=0

log()  { echo "[demo-up] $*"; }
warn() { echo "[demo-up] warning: $*" >&2; }
die()  { echo "[demo-up] FAIL: $*" >&2; echo "[demo-up] the stack may be half-up — logs: $LOGDIR, stop with: scripts/demo-up.sh stop" >&2; exit 1; }

usage() {
  sed -n '25,31p' "$0" | sed 's/^# \{0,1\}//'
}

# ---------- preflight helpers ----------

port_busy() { lsof -nP -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1; }

port_holder() { # prints "<pid> <command>" for the listener on the port, if any
  lsof -nP -iTCP:"$1" -sTCP:LISTEN -Fpc 2>/dev/null \
    | awk '/^p/{pid=substr($0,2)} /^c/{if (pid != "") {print pid, substr($0,2); exit}}'
}

our_process() { # is this listener one of ours? (only these binaries are ever stopped)
  case "$1" in
    core-api|worker|next-server|next) return 0 ;;
    *) return 1 ;;
  esac
}

wait_port() { # wait_port <port> <name> <tries>
  local port="$1" name="$2" tries="${3:-60}"
  for _ in $(seq 1 "$tries"); do
    port_busy "$port" && return 0
    sleep 1
  done
  warn "$name never opened :$port — see $LOGDIR/${name}.log"
  return 1
}

# -P (POSIX format): a long device name wraps `df` output on Linux and
# would silently misread the available column.
free_gb() { df -kP "$REPO_ROOT" | tail -1 | awk '{print int($4/1048576)}'; }

preflight() {
  local missing=()
  command -v docker  >/dev/null 2>&1 || missing+=("docker (https://docs.docker.com/get-docker/)")
  command -v cargo   >/dev/null 2>&1 || missing+=("cargo (https://rustup.rs)")
  command -v pnpm    >/dev/null 2>&1 || missing+=("pnpm (corepack enable pnpm)")
  command -v lsof    >/dev/null 2>&1 || missing+=("lsof")
  command -v python3 >/dev/null 2>&1 || missing+=("python3")
  if [ "${#missing[@]}" -gt 0 ]; then
    die "missing required tools: ${missing[*]}"
  fi
  docker info >/dev/null 2>&1 \
    || die "the docker daemon is not running — start Docker Desktop (or the docker service) and re-run"
  # A stray non-64-hex JWT_SECRET makes core-api refuse to boot (fail-closed
  # by design); catch it here with the fix, not as a dead api process.
  if [ -n "${JWT_SECRET:-}" ]; then
    if ! printf '%s' "$JWT_SECRET" | grep -Eq '^[0-9a-fA-F]{64}$'; then
      die "JWT_SECRET is set but is not 64 hex chars — core-api refuses such a secret. Fix: unset JWT_SECRET (the demo uses the pinned dev fallback), or export JWT_SECRET=\$(openssl rand -hex 32) with MARULA_SECRET_KEY set to the same value"
    fi
    if [ -z "${MARULA_SECRET_KEY:-}" ] || [ "$MARULA_SECRET_KEY" != "$JWT_SECRET" ]; then
      warn "JWT_SECRET is set but MARULA_SECRET_KEY is not the same value — sealed demo secrets (webhook signing, alert HMAC) may stay unreadable by the worker"
    fi
  fi
  local gb; gb="$(free_gb)"
  if [ "$gb" -lt 10 ]; then
    die "only ${gb} GB free on this volume — the Rust + web builds need room. Reclaim with scripts/dev-clean.sh (dry-run by default; --go applies)"
  fi
}

ensure_infra() {
  # postgres + redis only — the Rust services and web run natively so a
  # demo boot does not pay a container image build for code already on disk.
  if [ "$AI" = "1" ]; then
    write_ollama_override
    log "starting postgres + redis + the ai profile (ollama)"
    (cd "$REPO_ROOT" && compose -f "$OLLAMA_OVERRIDE" --profile ai up -d postgres redis ollama) \
      || die "docker compose could not start postgres/redis/ollama — is Docker running? (docker info)"
  else
    log "starting postgres + redis"
    (cd "$REPO_ROOT" && compose up -d postgres redis) \
      || die "docker compose could not start postgres/redis — is Docker running? (docker info)"
  fi
  for _ in $(seq 1 40); do
    # the corrected probe: compose's POSTGRES_USER is `marula`, not the
    # pre-rebrand `meridian` the retired dev-up.local.sh probed.
    if (cd "$REPO_ROOT" && compose exec -T postgres pg_isready -U marula -d marula >/dev/null 2>&1); then
      return 0
    fi
    sleep 1
  done
  die "postgres never became ready on :$PG_PORT — check: docker compose --env-file deploy/dev.env -f deploy/docker-compose.yml logs postgres"
}

write_ollama_override() {
  # The shipped compose keeps ollama unpublished on purpose (an
  # unauthenticated inference port is a remote-code-execution offer). The
  # demo stack runs core-api NATIVELY, so it cannot reach the compose
  # network; publish the port on LOOPBACK only, via this runtime override
  # in the gitignored state dir. Nothing outside this host can connect.
  mkdir -p "$STATE_DIR"
  cat > "$OLLAMA_OVERRIDE" <<'EOF'
# Generated by scripts/demo-up.sh --ai (Wave 47 lane B1). Loopback-only
# publish of the ai-profile ollama slot so the natively-run core-api can
# reach it. The shipped compose deliberately publishes nothing; this file
# binds 127.0.0.1 so nothing off-host can connect.
services:
  ollama:
    ports:
      - "127.0.0.1:11434:11434"
EOF
}

# ---------- database ----------

demo_seeded() {
  (cd "$REPO_ROOT" && compose exec -T postgres \
    psql -U marula -d marula -tAc "SELECT COUNT(*) FROM users WHERE email LIKE '%@demo.local'" 2>/dev/null) \
    | grep -q '[1-9]'
}

migrate() {
  log "applying schema migrations (owner role)"
  (cd "$REPO_ROOT" && DATABASE_URL="$DB_OWNER" cargo run -q -p marula-core-cli -- migrate) \
    || die "migrate failed — migrations must run as the table owner (marula), not the RLS-scoped app role; full log above"
}

seed() {
  log "seeding the deterministic demo org"
  (cd "$REPO_ROOT" && DATABASE_URL="$DB_OWNER" cargo run -q -p marula-core-cli -- seed-demo --force) \
    || die "seed-demo failed — the demo org could not be written; full log above"
}

reseed() {
  log "reseeding the demo org (seed-demo --force)"
  (cd "$REPO_ROOT" && DATABASE_URL="$DB_OWNER" cargo run -q -p marula-core-cli -- seed-demo --force) \
    || die "reseed failed — full log above"
  # The deterministic-reseed protocol from scripts/e2e-stack.sh: clear the
  # volatile per-user state so every demo starts from the same login.
  (cd "$REPO_ROOT" && compose exec -T postgres \
    psql -U marula -d marula -q -c \
    "TRUNCATE auth_lockouts; TRUNCATE auth_attempt_log;
     DELETE FROM settings WHERE scope = 'user' AND (key LIKE 'onboarding.%' OR key LIKE 'tours.%');") \
    || die "could not reset volatile demo state (lockouts / onboarding / tours)"
  log "volatile state reset (auth lockouts, onboarding, tours)"
}

# ---------- build + processes ----------

target_dir() {
  cargo metadata -q --no-deps --format-version 1 2>/dev/null \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])'
}

ensure_web_deps() {
  # A fresh clone has no node_modules; the one-command promise covers the
  # install. Worktree-local, so it never touches another lane's install.
  if [ ! -d "$REPO_ROOT/node_modules" ]; then
    log "installing workspace dependencies (pnpm install — first run takes minutes)"
    (cd "$REPO_ROOT" && pnpm install --frozen-lockfile) \
      || die "pnpm install failed — run it by hand inside $REPO_ROOT and re-run the script"
  fi
}

build_services() {
  ensure_web_deps
  log "building core-api + worker (first run compiles — expect minutes)"
  (cd "$REPO_ROOT" && cargo build -q -p core-api -p worker) \
    || die "cargo build failed — full log above"
  # No kit pre-build: the app resolves @marula-core/ui from src and transpiles
  # it (2026-10-19), so nothing has to be built for the web app to import it.
}

spawn() { # spawn <name> <port> <cmd...>
  local name="$1" port="$2"; shift 2
  mkdir -p "$LOGDIR"
  if port_busy "$port"; then
    log "$name already listening on :$port — skipping (idempotent re-run)"
    return 0
  fi
  ( "$@" >"$LOGDIR/$name.log" 2>&1 & echo "$name $! $port" >>"$PIDSFILE" )
  log "$name starting on :$port (log: $LOGDIR/$name.log)"
}

boot_backend() {
  mkdir -p "$STATE_DIR"
  : > "$PIDSFILE"
  chmod 700 "$STATE_DIR"

  local dir api worker
  dir="$(target_dir)/debug"
  api="$dir/core-api"; worker="$dir/worker"
  for b in "$api" "$worker"; do
    [ -x "$b" ] || die "expected binary missing after build: $b"
  done

  spawn "core-api" "$API_PORT" env \
    DATABASE_URL="$DB_APP" REDIS_URL="$REDIS_URL" BIND_ADDR="0.0.0.0:${API_PORT}" \
    CORS_ORIGIN="$CORS_ORIGIN" OBJECT_STORE=local \
    OBJECT_STORE_LOCAL_DIR="$OBJECT_STORE_LOCAL_DIR" \
    EMAIL_TRANSPORT=none "$api"

  spawn "worker" "$WORKER_PORT" env \
    DATABASE_URL="$DB_OWNER" REDIS_URL="$REDIS_URL" BIND_ADDR="0.0.0.0:${WORKER_PORT}" \
    CORE_API_URL="$API_LOCAL" EMAIL_TRANSPORT=none "$worker"

  # The write surface gates everything else: wait for it by name.
  for _ in $(seq 1 90); do
    curl -fsS "$API_LOCAL/api/v1/health" >/dev/null 2>&1 && break
    sleep 1
  done
  curl -fsS "$API_LOCAL/api/v1/health" >/dev/null 2>&1 \
    || die "core-api never answered /api/v1/health on :$API_PORT — log: $LOGDIR/core-api.log"
  log "core-api healthy ($API_LOCAL/api/v1/health)"
  # The worker answers /healthz once its loops are live, which lands a few
  # seconds after core-api; retry before warning so a healthy boot is quiet.
  for _ in $(seq 1 15); do
    curl -fsS "http://127.0.0.1:${WORKER_PORT}/healthz" >/dev/null 2>&1 && break
    sleep 1
  done
  if curl -fsS "http://127.0.0.1:${WORKER_PORT}/healthz" >/dev/null 2>&1; then
    log "worker healthy (http://127.0.0.1:${WORKER_PORT}/healthz)"
  else
    warn "worker :${WORKER_PORT}/healthz did not answer within 15s — log: $LOGDIR/worker.log"
  fi
}

boot_web() {
  if port_busy "$WEB_PORT"; then
    log "web already listening on :$WEB_PORT — skipping (idempotent re-run)"
    return 0
  fi
  mkdir -p "$LOGDIR"
  if [ "$WEB_MODE" = "prod" ]; then
    log "building the web console (next build — first run takes minutes)"
    (cd "$REPO_ROOT" && NEXT_PUBLIC_API_URL="$API_BROWSER" pnpm --filter @marula-core/web build) \
      || die "the web build failed — run: pnpm --filter @marula-core/web build"
    ( NEXT_PUBLIC_API_URL="$API_BROWSER" pnpm --filter @marula-core/web start \
        >"$LOGDIR/web.log" 2>&1 & echo "web $! $WEB_PORT" >>"$PIDSFILE" )
  else
    ( NEXT_PUBLIC_API_URL="$API_BROWSER" pnpm --filter @marula-core/web dev \
        >"$LOGDIR/web.log" 2>&1 & echo "web $! $WEB_PORT" >>"$PIDSFILE" )
  fi
  log "web starting on :$WEB_PORT (mode: $WEB_MODE, log: $LOGDIR/web.log)"
  wait_port "$WEB_PORT" web 180 || die "web never opened :$WEB_PORT — log: $LOGDIR/web.log"
}

# ---------- summary ----------

print_summary() {
  cat <<EOF

[demo-up] the demo stack is up.

  web console        $WEB_URL
  core-api           $API_BROWSER   (spec: $API_BROWSER/openapi.json)
  worker health      http://127.0.0.1:$WORKER_PORT/healthz

  demo logins (password: demo-password-123):
    owner@demo.local        Fiona Owner       — full admin
    dispatcher@demo.local   Dmitri Dispatcher — board + scheduling
    tech@demo.local         Tess Technician   — field work
    bookkeeper@demo.local   Bea Bookkeeper    — billing

  before a prospect is in the room:
    BYOK checklist    $WEB_URL/settings/ai   — org switch, provider, key, per-surface grants
    dev ops           $WEB_URL/dev           — seed / reset org / time-travel

  logs                $LOGDIR/{core-api,worker,web}.log
  reseed              scripts/demo-up.sh reset
  stop                scripts/demo-up.sh stop
EOF
  if [ "$AI" = "1" ]; then
    cat <<EOF

  ai profile         ollama on 127.0.0.1:11434 (loopback only — the shipped
                     compose publishes nothing, on purpose)
  pull the models (the first inference after a pull is slow — it loads them):
    docker compose --env-file deploy/dev.env -f deploy/docker-compose.yml \\
      --profile ai exec ollama ollama pull whisper
  the embedding model is fetched by the profile's ollama-model job; the
  bundled default is embeddinggemma-2:270m-q8_0-text
  then in $WEB_URL/settings/ai set the base URL to http://127.0.0.1:11434
  walkthrough: docs/site/how-to/local-demo-stack.md
EOF
  fi
  # The sealed demo secrets (alert HMAC, webhook signing) are printed by
  # the seeder itself, once, at seed time — never repeat them into logs.
}

# ---------- commands ----------

bring_up() { # bring_up <seed:0|1>
  local do_seed="$1"
  ensure_infra
  migrate
  if [ "$do_seed" = "1" ]; then
    if demo_seeded; then
      log "demo org already seeded — keeping existing data (reseed: scripts/demo-up.sh reset)"
    else
      seed
    fi
  fi
  build_services
  boot_backend
  boot_web
}

stop_all() { # stop_all [quiet]
  local quiet="${1:-0}" name pid port holder cmd
  if [ -f "$PIDSFILE" ]; then
    while read -r name pid port; do
      [ -n "${pid:-}" ] || continue
      if kill "$pid" 2>/dev/null && [ "$quiet" != "1" ]; then
        log "stopped $name (pid $pid, :$port)"
      fi
    done < "$PIDSFILE"
    rm -f "$PIDSFILE"
  elif [ "$quiet" != "1" ]; then
    log "no pid file ($PIDSFILE) — sweeping the demo ports for our processes"
  fi
  # Orphan sweep: only ever stop OUR binaries on the demo ports. Another
  # lane's stack (or the desktop app) holding a port is reported, not killed.
  for port in "$API_PORT" "$WORKER_PORT" "$WEB_PORT"; do
    holder="$(port_holder "$port" || true)"
    [ -n "$holder" ] || continue
    pid="${holder%% *}"; cmd="${holder#* }"
    if our_process "$cmd"; then
      kill "$pid" 2>/dev/null && [ "$quiet" != "1" ] && log "stopped orphan $cmd (pid $pid, :$port)"
    else
      [ "$quiet" = "1" ] || warn ":$port is held by $cmd (pid $pid) — not one of ours, left running"
    fi
  done
  if [ "$WITH_INFRA" = "1" ]; then
    log "stopping compose postgres + redis"
    (cd "$REPO_ROOT" && compose down) \
      || warn "compose down failed — docker may be down"
  fi
}

cmd_up() {
  preflight
  bring_up 1
  print_summary
}

cmd_reset() {
  preflight
  if [ "$FRESH" = "1" ]; then
    warn "DESTROYING the postgres volume (all data). Ctrl-C within 5 seconds to abort."
    sleep 5
    stop_all 1
    (cd "$REPO_ROOT" && compose down -v) \
      || die "compose down -v failed — docker may be down"
    rm -rf "$STATE_DIR/objects"
  fi
  bring_up 0
  reseed
  print_summary
}

cmd_stop() {
  stop_all
  log "stopped the demo processes (compose infra left running; also stop it with: scripts/demo-up.sh stop --infra)"
}

# ---------- dispatch ----------

CMD="${1:-up}"
[ $# -gt 0 ] && shift
case "$CMD" in
  up|stop|reset) ;;
  help|-h|--help) usage; exit 0 ;;
  *) echo "[demo-up] FAIL: unknown subcommand '$CMD' (try: up | stop | reset | help)" >&2; exit 1 ;;
esac

while [ $# -gt 0 ]; do
  case "$1" in
    --ai) AI=1 ;;
    --web) WEB_MODE="${2:-}"; [ "$WEB_MODE" = "dev" ] || [ "$WEB_MODE" = "prod" ] || die "--web takes dev|prod" ;;
    --web=*) WEB_MODE="${1#--web=}"; [ "$WEB_MODE" = "dev" ] || [ "$WEB_MODE" = "prod" ] || die "--web takes dev|prod" ;;
    --infra) WITH_INFRA=1 ;;
    --fresh) FRESH=1 ;;
    *) die "unknown flag '$1' for '$CMD' (try: scripts/demo-up.sh help)" ;;
  esac
  shift
done

mkdir -p "$STATE_DIR"

case "$CMD" in
  up)    cmd_up ;;
  reset) cmd_reset ;;
  stop)  cmd_stop ;;
esac
