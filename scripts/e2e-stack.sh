#!/usr/bin/env bash
# E2E demo stack (Wave 14 lane 8 `p14-e2e-ci`; per-run orchestration Wave 37
# lane 1 `p37-env-orchestration` — design/REPORT-lane-env-instability.md §3.1–3.2).
#
# Boots the exact stack the Playwright suite runs against, fully namespaced
# PER RUN so parallel lanes/sessions never collide (no fixed ports 5432/8080/
# 3000, no shared container names, no shared DB):
#
#   scripts/e2e-stack.sh              # up: migrate → seed → boot api+worker+redis,
#                                     #     write e2e/.e2e-stack.env for Playwright
#   scripts/e2e-stack.sh down [RUN]   # teardown ONE run (default: the local env
#                                     # file's run id) — only its own containers
#                                     # and PIDs, never a parallel lane's DB
#   scripts/e2e-stack.sh selftest     # teardown honesty: proves a `down` that
#                                     # removed nothing cannot report success
#                                     # (fake docker, no daemon, no stack)
#   E2E_FOREGROUND=1 scripts/e2e-stack.sh
#                                     # attach + EXIT trap tears down ONLY this run
#   E2E_KEEP=1 …                      # keep the PG container after teardown
#   E2E_FOLLOW=1 …                    # stay attached and stream the core-api +
#                                     #   worker logs (Ctrl-C stops following;
#                                     #   the stack keeps running)
#   E2E_DB_PROFILE=pooler …           # also boot PgBouncer for this run and
#                                     #   point the services at it (direct is
#                                     #   the default; the LISTEN connection and
#                                     #   the migrations stay direct)
#   E2E_DB_IMAGE=extensions …         # boot the overlay image that carries
#                                     #   pg_cron, hypopg and vchord instead of
#                                     #   the stock pgvector one, and assert the
#                                     #   extensions are present and used
#
# Run ids: the canonical shape is `e2e-<digits>` (new_run_id below), and the
# containers are `marula-e2e-pg-<RUN_ID>` / `marula-e2e-redis-<RUN_ID>`. `down`
# accepts the bare digits and normalises them, and it fails loudly when it
# removes nothing — `docker rm -f` exits 0 for a container that does not exist,
# which is how five orphaned stacks once "tore down" while still running.
#
# `up` RETURNS once core-api and the worker are both healthy. Both
# detach with their own stdio (`</dev/null`, logs under
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
# and worker ports, unique container names keyed by run id, a fresh 64-hex JWT
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
ENV_FILE="${E2E_ENV_FILE:-e2e/.e2e-stack.env}"   # per-checkout: Playwright reads it from its own tree
SELF="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"   # absolute, for the self-test's child runs
# Everything below (cargo, the guard, the relative env-file path) is relative to
# the checkout that owns this script; running it from anywhere else used to
# depend on the caller's cwd. Same shape as scripts/dev-status.sh.
cd "$REPO_ROOT"

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

# ---------- owner identity (read by `scripts/dev-status.sh --reap`) ----------
# A run outlives the command that booted it, so the registry records who owns
# it: the agent session id and the pid of the long-lived ancestor that dies
# with the lane. Without that, a sweep cannot tell "owner gone" (reap it) from
# "owner idle and thinking" (never touch it) — the distinction the 2026-10-17
# leak turned on, when a cancelled lane left a Postgres, a Valkey and two
# services holding three ports for five hours while the evidence to reclaim
# them sat in the registry the whole time.
session_id() {
  printf '%s' "${MARULA_SESSION_ID:-${OPENCODE_SESSION_ID:-}}"
}

# The highest ancestor below pid 1 is the desktop app, the terminal or the
# service process the shell was spawned from, and it lives exactly as long as
# the session does. MARULA_OWNER_PID overrides the walk: that is how the
# self-test and the gate rehearse a dead owner without killing a real one.
owner_pid() {
  if [ -n "${MARULA_OWNER_PID:-}" ]; then printf '%s' "$MARULA_OWNER_PID"; return 0; fi
  local pid="$$" next hops=0
  while [ "$hops" -lt 16 ]; do
    next="$(ps -o ppid= -p "$pid" 2>/dev/null | tr -d ' ')"
    case "$next" in ''|*[!0-9]*) break ;; esac
    if [ "$next" -le 1 ]; then break; fi
    pid="$next"
    hops=$((hops + 1))
  done
  printf '%s' "$pid"
}

owner_cmd() {
  ps -o command= -p "$1" 2>/dev/null | head -1 | cut -c1-120 || true
}

OWNER_SESSION="$(session_id)"
OWNER_PID="$(owner_pid)"
OWNER_CMD="$(owner_cmd "$OWNER_PID")"
CHECKOUT_BRANCH="$(git -C "$REPO_ROOT" rev-parse --abbrev-ref HEAD 2>/dev/null || true)"
# Optional, and only ever recorded when the lane exports it: an evidence
# directory the stall check reads as a second activity signal next to commits.
# Made absolute here because the sweep runs from another checkout.
EVIDENCE_DIR="${MARULA_EVIDENCE_DIR:-}"
case "$EVIDENCE_DIR" in
  "") ;;
  /*) ;;
  *)  EVIDENCE_DIR="$REPO_ROOT/$EVIDENCE_DIR" ;;
esac

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

# ---------- docker, with a seam the self-test can point elsewhere ----------
DOCKER="${DOCKER:-docker}"
container_exists() { "$DOCKER" inspect "$1" >/dev/null 2>&1; }
# `-v` drops the container's ANONYMOUS volumes (both images declare VOLUME, so
# every `docker run` here creates one — 224.3MB of Postgres data across this
# lane's two runs, which `rm -f` alone leaves behind for a prune to find later).
# Named volumes are never touched by -v, so only this run's own scratch state goes.
container_remove() { "$DOCKER" rm -f -v "$1" >/dev/null 2>&1; }

# ---------- run id ----------
# The canonical shape is the registry's own `e2e-<digits>` (new_run_id below),
# and the containers are marula-e2e-pg-<RUN_ID>. Nothing documented that, so
# `down 17921729901030224669` was a silent no-op and read as "already gone".
# Accept the bare digits too, and never guess at a non-numeric id (`ci`).
normalize_run_id() {
  case "${1:-}" in
    "") return 1 ;;
    *[!0-9]*) printf '%s' "$1" ;;
    *) printf 'e2e-%s' "$1" ;;
  esac
}

# ---------- teardown (run-id-keyed) ----------
# `docker rm -f` exits 0 when the container does not exist (verified 2026-10-16:
# `docker rm -f marula-e2e-pg-17921729901030224669; echo $?` printed 0), so the
# old `rm -f … && echo "removed container …"` guard reported success for work it
# never did. Five orphaned stacks printed "removed container …" and "run …
# torn down" while removing nothing, the registry reported zero runs, and the
# leak stayed invisible. Existence is now checked explicitly, and a teardown
# that matched nothing fails instead of printing a success line.
remove_run_containers() { # remove_run_containers <run_id>; non-zero when nothing matched
  local run="$1" name removed=0 missing=""
  for name in "marula-e2e-pg-$run" "marula-e2e-redis-$run" "marula-e2e-pgbouncer-$run"; do
    if container_exists "$name"; then
      if container_remove "$name"; then
        echo "[e2e-stack] removed container $name"
        removed=$((removed + 1))
      else
        echo "[e2e-stack] WARN container $name exists but could not be removed" >&2
      fi
    else
      missing="$missing $name"
    fi
  done
  if [ "$removed" -eq 0 ]; then
    echo "[e2e-stack] FAIL: nothing matched marula-e2e-pg-$run or marula-e2e-redis-$run — no container was removed" >&2
    echo "[e2e-stack]       run ids are e2e-<digits> (bare digits are accepted); see what is live with: $DOCKER ps --filter name=marula-e2e" >&2
    return 1
  fi
  if [ -n "$missing" ]; then
    echo "[e2e-stack] note: already gone:$missing"
  fi
  return 0
}

# Per-run network, used only by the pooler profile: two containers resolve each
# other by name on a user-defined bridge, and the default bridge does not.
remove_run_network() { # remove_run_network <run_id>
  local net="marula-e2e-net-$1"
  if "$DOCKER" network inspect "$net" >/dev/null 2>&1; then
    if "$DOCKER" network rm "$net" >/dev/null 2>&1; then
      echo "[e2e-stack] removed network $net"
    else
      echo "[e2e-stack] WARN network $net exists but could not be removed" >&2
    fi
  fi
}

down_run() { # down_run <run_id>; non-zero when it removed nothing
  local run="$1" f rc=0
  echo "[e2e-stack] teardown run $run"
  echo "[e2e-stack] looks for containers marula-e2e-pg-$run and marula-e2e-redis-$run"
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
  else
    echo "[e2e-stack] no registry entry $f (the boot never registered, or it was removed)"
  fi
  remove_run_containers "$run" || rc=1
  remove_run_network "$run"
  # shared scratch PG: drop THIS run's DATABASE only (the shared container
  # stays for other runs). The name is run-id keyed, so it can never match a
  # different lane's database.
  if [ -n "${MARULA_SHARED_PG_CONTAINER:-}" ] && command -v "$DOCKER" >/dev/null 2>&1; then
    local shared="${MARULA_SHARED_PG_CONTAINER}"
    local dbname="marula_e2e_${run//[^a-z0-9]/_}"
    if "$DOCKER" exec "$shared" psql -U postgres -tAc \
        "select 1 from pg_database where datname='$dbname'" 2>/dev/null | grep -q 1; then
      if "$DOCKER" exec "$shared" psql -U postgres -c "DROP DATABASE $dbname" >/dev/null 2>&1; then
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
  if [ "$rc" -eq 0 ]; then
    echo "[e2e-stack] run $run torn down"
  else
    echo "[e2e-stack] run $run NOT torn down — no container carried that run id" >&2
  fi
  return "$rc"
}

# ---------- self-test: a teardown that removed nothing cannot look successful ----
# Runs the real `down` path against a fake docker whose `rm -f` behaves like the
# real one (exit 0 for a container that does not exist), so the assertions are
# about this script's honesty, not about a daemon.
selftest_teardown() {
  local tmp failures=0 checks=0
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/e2e-stack-selftest.XXXXXX")" || return 1
  # the path is embedded: the trap outlives the local variable
  trap "rm -rf '$tmp'" EXIT
  cat > "$tmp/docker" <<'FAKE'
#!/usr/bin/env bash
# Fake docker: only what the teardown path uses. `rm -f` on a missing container
# exits 0, exactly like the real CLI, which is the behaviour that hid the leak.
state="${FAKE_DOCKER_STATE:?}"
[ -n "${FAKE_DOCKER_LOG:-}" ] && printf '%s\n' "$*" >> "$FAKE_DOCKER_LOG"
case "${1:-}" in
  inspect) grep -qx "${2:-}" "$state" ;;
  rm)      name="${@: -1}"   # `rm -f -v <name>`: the name is the last argument
           if grep -qx "$name" "$state"; then grep -vx "$name" "$state" > "$state.tmp"; mv "$state.tmp" "$state"; echo "$name"; exit 0; fi
           exit 0 ;;
  network) exit 1 ;;          # the fake daemon has no networks
  ps)      cat "$state" ;;
  *)       exit 0 ;;
esac
FAKE
  chmod +x "$tmp/docker"

  run_case() { # run_case <label> <expect_rc> <state-lines> <down-arg> [grep-for]
    local label="$1" want="$2" state="$3" arg="$4" needle="${5:-}"
    : > "$tmp/state"
    if [ -n "$state" ]; then printf '%s\n' $state > "$tmp/state"; fi
    # `|| rc=$?` on purpose: the child exits non-zero in most of these cases and
    # this script runs under `set -e`.
    local out rc=0
    : > "$tmp/argv.log"
    out="$(DOCKER="$tmp/docker" FAKE_DOCKER_STATE="$tmp/state" FAKE_DOCKER_LOG="$tmp/argv.log" \
           E2E_ENV_FILE="$tmp/no-such.env" REGISTRY_DIR="$tmp/reg" LOG_ROOT="$tmp/logs" \
           bash "$SELF" down "$arg" 2>&1)" || rc=$?
    checks=$((checks + 1))
    if [ "$rc" != "$want" ]; then
      echo "  FAIL  $label: exit $rc, wanted $want"
      printf '%s\n' "$out" | sed 's/^/          /'
      failures=$((failures + 1))
      return
    fi
    if [ -n "$needle" ] && ! printf '%s' "$out" | grep -q "$needle"; then
      echo "  FAIL  $label: output does not mention '$needle'"
      printf '%s\n' "$out" | sed 's/^/          /'
      failures=$((failures + 1))
      return
    fi
    echo "  PASS  $label"
  }

  echo "e2e-stack self-test: teardown honesty (fake docker, no daemon, no stack)"
  run_case "both containers exist — removed, exit 0" 0 \
    "marula-e2e-pg-e2e-1111 marula-e2e-redis-e2e-1111" "e2e-1111" "removed container marula-e2e-pg-e2e-1111"
  run_case "removed nothing — non-zero and it says what it looked for" 1 \
    "" "e2e-2222" "FAIL: nothing matched marula-e2e-pg-e2e-2222"
  # The old code printed "removed container …" here; that line must not come back.
  local out2
  out2="$(DOCKER="$tmp/docker" FAKE_DOCKER_STATE="$tmp/state" \
          E2E_ENV_FILE="$tmp/no-such.env" REGISTRY_DIR="$tmp/reg" LOG_ROOT="$tmp/logs" \
          bash "$SELF" down e2e-2222 2>&1)" || true
  checks=$((checks + 1))
  if printf '%s' "$out2" | grep -qE "removed container|^\[e2e-stack\] run .* torn down$"; then
    echo "  FAIL  a teardown that removed nothing printed a success line"
    failures=$((failures + 1))
  else
    echo "  PASS  a teardown that removed nothing printed no success line"
  fi
  run_case "bare digits normalise to e2e-<digits>" 0 \
    "marula-e2e-pg-e2e-3333 marula-e2e-redis-e2e-3333" "3333" "removed container marula-e2e-pg-e2e-3333"
  run_case "idempotent second teardown still fails loudly" 1 \
    "" "e2e-1111" "NOT torn down"
  # The anonymous volume each `docker run` leaves behind is this run's too: the
  # teardown must ask for it (`rm -f -v`), or a prune finds 224MB of Postgres
  # data per boot long after the lane is gone.
  printf '%s\n' marula-e2e-pg-e2e-4444 marula-e2e-redis-e2e-4444 > "$tmp/state"
  : > "$tmp/argv.log"
  DOCKER="$tmp/docker" FAKE_DOCKER_STATE="$tmp/state" FAKE_DOCKER_LOG="$tmp/argv.log" \
    E2E_ENV_FILE="$tmp/no-such.env" REGISTRY_DIR="$tmp/reg" LOG_ROOT="$tmp/logs" \
    bash "$SELF" down e2e-4444 >/dev/null 2>&1 || true
  checks=$((checks + 1))
  if grep -q -- "-f -v marula-e2e-pg-e2e-4444" "$tmp/argv.log"; then
    echo "  PASS  teardown drops the container's anonymous volumes (-v)"
  else
    echo "  FAIL  teardown did not pass -v to docker rm"
    failures=$((failures + 1))
  fi
  run_case "no id and no env file — refuses instead of guessing" 1 \
    "" "" "pass one, e.g."

  if [ "$failures" -eq 0 ]; then
    echo "e2e-stack self-test: $checks checks pass"
    return 0
  fi
  echo "e2e-stack self-test: $failures of $checks failed"
  return 1
}

# ---------- self-test: the owner identity a sweep reads -------------------
# The reaper's whole decision is "is the recorded owner still alive", so the
# recording is provable without a stack. The env override is the rehearsal hook
# `scripts/dev-status.sh --reap` is gated against; the ancestor walk is what a
# real boot records; the session id is whichever name the agent exported.
selftest_owner() {
  local failures=0 checks=0 got

  check() { # check <label> <got> <want>
    checks=$((checks + 1))
    if [ "$2" = "$3" ]; then
      echo "  PASS  $1"
    else
      echo "  FAIL  $1: got '$2', want '$3'"
      failures=$((failures + 1))
    fi
  }

  echo "e2e-stack self-test: owner identity (no stack)"
  got="$(MARULA_OWNER_PID=424242; owner_pid)"
  check "MARULA_OWNER_PID overrides the walk" "$got" "424242"

  got="$(owner_pid)"
  if [ "$got" != "$$" ] && kill -0 "$got" 2>/dev/null; then
    check "the walk records a live ancestor, not the transient shell" "alive" "alive"
  else
    check "the walk records a live ancestor, not the transient shell" "pid $got" "a live pid other than $$"
  fi

  got="$(MARULA_SESSION_ID=ses_override OPENCODE_SESSION_ID=ses_env; session_id)"
  check "MARULA_SESSION_ID wins over the harness name" "$got" "ses_override"
  got="$(unset MARULA_SESSION_ID; OPENCODE_SESSION_ID=ses_env; session_id)"
  check "OPENCODE_SESSION_ID is the fallback" "$got" "ses_env"

  if [ -n "$(owner_cmd "$(owner_pid)")" ]; then
    check "the owner command is recorded for the human reading the registry" "set" "set"
  else
    check "the owner command is recorded for the human reading the registry" "empty" "set"
  fi

  if [ "$failures" -eq 0 ]; then
    echo "e2e-stack self-test: $checks owner checks pass"
    return 0
  fi
  echo "e2e-stack self-test: $failures of $checks owner checks failed"
  return 1
}

# ########## dispatch ##########
case "${1:-up}" in
  down)
    # explicit teardown: RUN comes from the arg (e2e-<digits> or bare digits),
    # else from this checkout's env file.
    raw_run="${2:-}"
    if [ -z "$raw_run" ]; then
      raw_run="$(grep -m1 '^MARULA_RUN_ID=' "$ENV_FILE" 2>/dev/null | cut -d= -f2 | tr -d '"' || true)"
      if [ -z "$raw_run" ]; then
        die "no run id given and $ENV_FILE holds none — pass one, e.g. bash scripts/e2e-stack.sh down e2e-17921729901030224669"
      fi
      echo "[e2e-stack] run id $raw_run (from $ENV_FILE)"
    fi
    run_id="$(normalize_run_id "$raw_run")"
    if [ "$run_id" != "$raw_run" ]; then
      echo "[e2e-stack] run id $raw_run normalised to $run_id"
    fi
    if down_run "$run_id"; then exit 0; else exit $?; fi
    ;;
  selftest)
    rc=0
    selftest_teardown || rc=1
    selftest_owner || rc=1
    exit "$rc"
    ;;
  help|-h|--help)
    sed -n '1,28p' "$0"; exit 0 ;;
esac

# ########## mode selection ##########
if [ -n "${E2E_PG_CONTAINER:-}" ] || [ -n "${CI:-}" ]; then
  # Legacy hosted-DB mode (CI): env is fixed, no per-run namespacing.
  case "${E2E_DB_PROFILE:-direct}${E2E_DB_IMAGE:-stock}" in
    *pooler*|*extensions*)
      die "E2E_DB_PROFILE=pooler and E2E_DB_IMAGE=extensions belong to the isolated stack: they start containers this mode does not own (hosted mode uses E2E_PG_CONTAINER)" ;;
  esac
  RUN_ID="${MARULA_RUN_ID:-ci}"
  DB_NAME="$(echo "${DATABASE_URL:-}" | sed 's|.*/||' || true)"
  PG_PORT=5432
  API_PORT=8080
  WEB_PORT="${E2E_WEB_PORT:-3000}"
  export DATABASE_URL="${DATABASE_URL:-postgres://postgres:postgres@localhost:5432/marula_test}"
  export BIND_ADDR="${BIND_ADDR:-0.0.0.0:${API_PORT}}"
  export JWT_SECRET="${JWT_SECRET:-e2e-demo-secret-not-for-production}"
  export MARULA_INTERNAL_TOKEN="${MARULA_INTERNAL_TOKEN:-e2e-internal-token-not-for-production}"
  # core-api refuses boot without BOTH machine-seam tokens: the bearer above
  # (SLA, automation, webhook, dedupe, workflow-node calls) and this
  # worker↔API send/effect token (Wave 54 lane S3d, bearer half in Wave 55).
  # The worker gets the same values from this process env.
  export EMAIL_INTERNAL_TOKEN="${EMAIL_INTERNAL_TOKEN:-e2e-email-internal-not-for-production}"
  export WEB_BASE_URL="${WEB_BASE_URL:-http://localhost:${WEB_PORT}}"
  export CORS_ORIGIN="${CORS_ORIGIN:-http://localhost:${WEB_PORT}}"
  WORKER_PORT="${WORKER_PORT:-8082}"
  echo "[e2e-stack] hosted mode (CI/E2E_PG_CONTAINER): fixed env, no per-run isolation"
else
  RUN_ID="$(new_run_id)"
  PG_PORT="$(free_port 25432)"
  API_PORT="$(free_port 28480)"
  WEB_PORT="${E2E_WEB_PORT:-$(free_port 3000)}"
  DB_NAME="marula_e2e_${RUN_ID//[^a-z0-9]/_}"
  DB_NAME="${DB_NAME:0:63}"
  export JWT_SECRET="$(openssl rand -hex 32 2>/dev/null || true)"
  [ ${#JWT_SECRET} -eq 64 ] || die "openssl rand -hex 32 unavailable"
  export MARULA_SECRET_KEY="$JWT_SECRET" # same dev seal key as the seed
  # The bearer half of the worker↔api machine seam (Wave 43 lane 20, item G;
  # required by the boot check since Wave 55). The worker's node endpoint and
  # `workflow.resume` consumer need it — without it no lane could execute a
  # workflow — and core-api now refuses boot without it outside the dev
  # profile. Pair it with EMAIL_INTERNAL_TOKEN below, which core-api refuses
  # boot without the same way.
  export MARULA_INTERNAL_TOKEN="$(openssl rand -hex 32 2>/dev/null || true)"
  [ ${#MARULA_INTERNAL_TOKEN} -eq 64 ] || die "openssl rand -hex 32 unavailable"
  REDIS_PORT="$(free_port 26379)"
  WORKER_PORT="$(free_port 28882)"
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

# ---------- database topology (Wave 62 lane p62-credentials-edge) ----------
# P9's pooler claim was documented and never booted by a stack, so a change that
# broke it could ship green. `E2E_DB_PROFILE=pooler` starts PgBouncer for THIS
# run on its own port, in front of the run's own Postgres, and points the two
# services at it. `direct` is the default and is unchanged.
#
# What stays direct, deliberately: migrations and the demo seed (sqlx migrations
# take a session-scoped advisory lock, which transaction pooling cannot hold),
# `MARULA_TEST_DATABASE_URL` (the DB suites own whole-table behaviour and must
# not be shaped by pool settings), and `MARULA_LISTEN_DATABASE_URL` (a
# subscription cannot ride a transaction pooler either — the same reason the
# compose stack sets it).
DB_PROFILE="${E2E_DB_PROFILE:-direct}"
case "$DB_PROFILE" in
  direct|pooler) ;;
  *) die "E2E_DB_PROFILE must be direct or pooler (got '$DB_PROFILE')" ;;
esac
DB_IMAGE="${E2E_DB_IMAGE:-stock}"
case "$DB_IMAGE" in
  stock) PG_IMAGE="pgvector/pgvector:pg18" ;;
  extensions) PG_IMAGE="marula/postgres-extensions:18" ;;
  *) die "E2E_DB_IMAGE must be stock or extensions (got '$DB_IMAGE')" ;;
esac
if [ "$DB_IMAGE" = "extensions" ]; then
  # The overlay exists to make this mode satisfiable (vchord preloaded, the ANN
  # index built by the migrate step). Selecting it here is what turns the image
  # from "installed" into "used", and it is why the assertion script can require
  # the vchordrq index instead of accepting its absence as a deployment choice.
  export MARULA_RETRIEVAL_MODE=index
fi
if [ "${MODE:-}" = "isolated" ]; then
  echo "[e2e-stack] database topology: image ${DB_IMAGE} (${PG_IMAGE}), profile ${DB_PROFILE}"
fi

pg_ready() { pg_isready -q -d "$1" 2>/dev/null; }

if [ "${MODE:-}" = "isolated" ]; then
  ########## scratch Postgres: per-run container + per-run DB ##########
  # The migrate/seed half and every DB suite dial this URL; only the two booted
  # services use the pooled one below, when the pooler profile is on.
  export DATABASE_URL="postgres://postgres:postgres@localhost:${PG_PORT}/${DB_NAME}"
  DIRECT_DATABASE_URL="$DATABASE_URL"
  docker rm -f "marula-e2e-pg-$RUN_ID" >/dev/null 2>&1 || true
  NETWORK_ARGS=()
  if [ "$DB_PROFILE" = "pooler" ]; then
    # A per-run network, so PgBouncer resolves the Postgres container by name.
    # The default bridge has no DNS; the run id in the name keeps it this run's.
    "$DOCKER" network rm "marula-e2e-net-$RUN_ID" >/dev/null 2>&1 || true
    "$DOCKER" network create "marula-e2e-net-$RUN_ID" >/dev/null \
      || die "could not create network marula-e2e-net-$RUN_ID"
    NETWORK_ARGS=(--network "marula-e2e-net-$RUN_ID")
  fi
  # The overlay image is built from deploy/postgres (it installs PGDG packages
  # and a pinned VectorChord deb), so a host that has not built it yet gets it
  # built here rather than a confusing "image not found" from `docker run`.
  if [ "$DB_IMAGE" = "extensions" ] && ! "$DOCKER" image inspect "$PG_IMAGE" >/dev/null 2>&1; then
    run_stage "build ${PG_IMAGE} (deploy/postgres)" \
      "$DOCKER build -t $PG_IMAGE deploy/postgres" \
      || die "could not build $PG_IMAGE — build deploy/postgres or drop E2E_DB_IMAGE=extensions"
  fi
  echo "[e2e-stack] starting scratch Postgres marula-e2e-pg-$RUN_ID on port $PG_PORT ($PG_IMAGE)"
  # Same instrumentation the compose stack runs (deploy/docker-compose.yml):
  # the query mix and the slow tail are what migration 0145 reads, so the
  # test database has to collect them too. Keep the two lists in step.
  #
  # The extension image adds the two libraries it carries and the ones the
  # overlay preloads: `pg_cron` refuses to be CREATEd without the preload, and
  # `vchord` cannot maintain its index without it (deploy/postgres/compose.extensions.yml).
  PG_SERVER_ARGS=(
    -c "shared_preload_libraries=pg_stat_statements,auto_explain"
    -c "pg_stat_statements.max=10000"
    -c "auto_explain.log_min_duration=200ms"
    -c "auto_explain.log_analyze=on"
    -c "auto_explain.log_buffers=on"
    -c "auto_explain.log_format=json"
    -c "io_method=worker"
  )
  if [ "$DB_IMAGE" = "extensions" ]; then
    PG_SERVER_ARGS[1]="shared_preload_libraries=pg_stat_statements,auto_explain,pg_cron,vchord"
    PG_SERVER_ARGS+=(-c "cron.database_name=$DB_NAME")
  fi
  docker run -d --name "marula-e2e-pg-$RUN_ID" \
    ${NETWORK_ARGS[@]+"${NETWORK_ARGS[@]}"} \
    -e POSTGRES_USER=postgres -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=marula_test \
    -p "$PG_PORT:5432" "$PG_IMAGE" \
    "${PG_SERVER_ARGS[@]}" >/dev/null \
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

  ########## PgBouncer (P9's claim, actually booted) ##########
  # `deploy/pooler/pgbouncer.ini` is the shipped config and stays the source of
  # truth for the [pgbouncer] half: this reads the file and replaces ONLY the
  # [databases] section (its `host=postgres` is the compose name; this run's
  # database lives on a container named for the run). A local ini with the
  # load-bearing settings retyped by hand would be a second source of truth for
  # pool_mode/server_reset_query/max_prepared_statements — exactly the drift the
  # pooled deployment must not have.
  if [ "$DB_PROFILE" = "pooler" ]; then
    POOLER_PORT="$(free_port 26432)"
    POOLER_DIR="$LOG_ROOT/$RUN_ID/pooler"
    mkdir -p "$POOLER_DIR"
    {
      echo "[databases]"
      echo "$DB_NAME = host=marula-e2e-pg-$RUN_ID port=5432 dbname=$DB_NAME"
      # Everything from [pgbouncer] on, verbatim; the shipped [databases] block
      # is dropped because it names the compose host.
      awk '/^\[databases\]/{skip=1; next} /^\[pgbouncer\]/{skip=0} !skip' \
        "$REPO_ROOT/deploy/pooler/pgbouncer.ini"
    } > "$POOLER_DIR/pgbouncer.ini"
    # The auth file is rendered, never checked in (F65), by the same script the
    # compose stack runs. The passwords are this scratch container's own values.
    POSTGRES_PASSWORD=postgres MARULA_APP_PASSWORD=marula_app \
      sh "$REPO_ROOT/deploy/pooler/render-userlist.sh" "$POOLER_DIR/userlist.txt" >/dev/null 2>&1 \
      || die "could not render the PgBouncer auth file (deploy/pooler/render-userlist.sh)"
    docker rm -f "marula-e2e-pgbouncer-$RUN_ID" >/dev/null 2>&1 || true
    echo "[e2e-stack] starting PgBouncer marula-e2e-pgbouncer-$RUN_ID on 127.0.0.1:$POOLER_PORT (fronting marula-e2e-pg-$RUN_ID)"
    docker run -d --name "marula-e2e-pgbouncer-$RUN_ID" \
      --network "marula-e2e-net-$RUN_ID" \
      -p "127.0.0.1:${POOLER_PORT}:6432" \
      -v "$POOLER_DIR/pgbouncer.ini:/etc/pgbouncer/pgbouncer.ini:ro" \
      -v "$POOLER_DIR/userlist.txt:/auth/userlist.txt:ro" \
      dhi.io/pgbouncer:1.26@sha256:17157120154cd111541281b39e9ad846e567e7fc1ea2e964abdc8c4bf4547fff \
      >/dev/null || die "could not start PgBouncer (docker down, or the pinned image is unavailable)"
    POOLED_DATABASE_URL="postgres://postgres:postgres@127.0.0.1:${POOLER_PORT}/${DB_NAME}"
    # Wait for the pooler's own listener, not for the database: a pooler that
    # has not started would otherwise be reported by the services as a database
    # failure with a misleading message.
    pooler_ready=0
    for _ in $(seq 1 30); do
      if python3 - "$POOLER_PORT" <<'PY'
import socket, sys
sock = socket.socket()
sock.settimeout(0.5)
try:
    sock.connect(("127.0.0.1", int(sys.argv[1])))
except OSError:
    raise SystemExit(1)
raise SystemExit(0)
PY
      then pooler_ready=1; break; fi
      sleep 1
    done
    [ "$pooler_ready" -eq 1 ] || die "PgBouncer never accepted a connection on 127.0.0.1:$POOLER_PORT — see docker logs marula-e2e-pgbouncer-$RUN_ID"
    echo "[e2e-stack] pooler ready — services dial $POOLED_DATABASE_URL (LISTEN stays direct)"
  fi

  ########## scratch Valkey (redis name kept — REDIS_URL contract) ##########
  docker rm -f "marula-e2e-redis-$RUN_ID" >/dev/null 2>&1 || true
  echo "[e2e-stack] starting scratch Valkey marula-e2e-redis-$RUN_ID on port $REDIS_PORT"
  docker run -d --name "marula-e2e-redis-$RUN_ID" \
    -p "$REDIS_PORT:6379" valkey/valkey:8-alpine \
    valkey-server --maxmemory 128mb --maxmemory-policy noeviction >/dev/null \
    || die "could not start scratch Valkey"

  ########## failed-boot hygiene (Wave 47 lane A17) ##########
  # The registry entry is written only AFTER core-api answers its health check,
  # so a boot that dies before that point (a build error, a port conflict, an
  # api that never comes up) leaves this run's own containers running with
  # nothing that names them: `scripts/dev-status.sh` can only report registered
  # runs, and the pair outlives the lane. That is the shape that made five
  # container pairs look orphaned on 2026-10-16. Remove ONLY this run id's
  # containers, only when the boot never became ready, and honour E2E_KEEP.
  UP_COMPLETE=0
  cleanup_failed_boot() {
    local rc=$?
    if [ "$rc" -ne 0 ] && [ "${UP_COMPLETE:-0}" -eq 0 ]; then
      if [ "${E2E_KEEP:-0}" = "1" ]; then
        echo "[e2e-stack] FAIL: run $RUN_ID never became ready (exit $rc); E2E_KEEP=1 keeps marula-e2e-pg-$RUN_ID (port $PG_PORT) and marula-e2e-redis-$RUN_ID (port $REDIS_PORT)" >&2
      else
        echo "[e2e-stack] FAIL: run $RUN_ID never became ready (exit $rc) — removing its own containers (${RUN_ID} only)" >&2
        docker rm -f "marula-e2e-pg-$RUN_ID" "marula-e2e-redis-$RUN_ID" "marula-e2e-pgbouncer-$RUN_ID" >/dev/null 2>&1 || true
        "$DOCKER" network rm "marula-e2e-net-$RUN_ID" >/dev/null 2>&1 || true
      fi
    fi
  }
  trap cleanup_failed_boot EXIT
fi

########## migrate + deterministic demo seed (mechanical reseed) ##########
# The CLI runs in the DEV profile here (plain `cargo run`), so the dev half of
# the shared dir gets the same provenance check the release builds get below.
# A debug artifact from another tree poisons a debug build exactly the way a
# release one poisons a release build — same relative dep-info, same mtime trap.
if [ "${MARULA_LANE_GUARD:-1}" != "0" ]; then
  run_stage "shared-target guard (dev profile)" \
    "python3 scripts/lane-target-guard.py check --profile debug --fix" \
    || die "shared-target guard refused the migrate/seed build — read its reason above, run the fix it prints, or set MARULA_LANE_GUARD=0 to override deliberately"
fi
run_stage "migrate schema" 'cargo run -q -p marula-core-cli -- migrate'
run_stage "seed the deterministic demo org (seed-demo --force)" \
  'MARULA_DEV_TOOLS=1 cargo run -q -p marula-core-cli -- seed-demo --force'

########## the overlay image actually carries — and uses — the extensions ##########
# P8's image and migration 0145/0153 degrade quietly when pg_cron, hypopg or
# vchord are absent, which is the right product behaviour and a useless proof:
# a stack booted on the stock image looks the same as one booted on the overlay.
# So when this run asked for the overlay, the extensions are asserted present
# AND used on the database the migrations just ran against (F71's sibling: the
# gate has to exercise the thing, not take the lane's word).
if [ "$DB_IMAGE" = "extensions" ]; then
  run_stage "assert the extensions are present and used (deploy/postgres)" \
    "bash $REPO_ROOT/scripts/check-postgres-extensions.sh --container marula-e2e-pg-$RUN_ID --db $DB_NAME" \
    || die "the overlay image is booted but an extension is missing or unused — see the FAIL lines above"
fi

########## isolated test database (no service attached) ##########
# Some DB suites assert on whole-table behaviour rather than on rows they own:
# `fetch_unpublished` polls every unpublished outbox row with SKIP LOCKED, so
# with the stack's worker attached to the same database it relays rows between
# the test's insert and its fetch, and the test legitimately sees a subset.
# MARULA_TEST_DATABASE_URL is the override those harnesses already try first
# (tests/common/mod.rs, core-projects, csat), so give them a database nothing
# polls. TEMPLATE copies schema + demo rows without a second seed run, and the
# copy is taken before core-api or the worker start, which is the only moment
# the template can have no other connections.
TEST_DB_NAME="${DB_NAME}_test"
if ! docker exec "marula-e2e-pg-$RUN_ID" psql -U postgres -d postgres \
      -c "CREATE DATABASE \"$TEST_DB_NAME\" TEMPLATE \"$DB_NAME\"" >/dev/null 2>&1; then
  # Already existing is fine (re-running `up` against a kept container).
  docker exec "marula-e2e-pg-$RUN_ID" psql -U postgres -tAc \
    "select 1 from pg_database where datname='$TEST_DB_NAME'" 2>/dev/null | grep -q 1 \
    || die "could not create isolated test database $TEST_DB_NAME"
fi
TEST_DATABASE_URL="postgres://postgres:postgres@localhost:${PG_PORT}/${TEST_DB_NAME}"
export MARULA_TEST_DATABASE_URL="$TEST_DATABASE_URL"
echo "[e2e-stack] isolated test database ready ($TEST_DB_NAME, no worker attached)"
if [ "${MARULA_LANE_GUARD:-1}" != "0" ]; then
  run_stage "mark the shared target (dev provenance)" \
    "python3 scripts/lane-target-guard.py mark --profile debug --note e2e-stack:cli-migrate+seed"
fi

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
# Past this point the SERVICES use the pooled URL when the pooler profile is on.
# Everything above (migrations, the seed, the DB resets) stayed direct, and
# `MARULA_TEST_DATABASE_URL` stays direct for the DB suites.
if [ "$DB_PROFILE" = "pooler" ]; then
  export DATABASE_URL="$POOLED_DATABASE_URL"
  export MARULA_LISTEN_DATABASE_URL="$DIRECT_DATABASE_URL"
  # A transaction pooler cannot keep a named statement between transactions
  # (server_reset_query = DISCARD ALL), so sqlx must not cache the names either
  # — the matching half of `max_prepared_statements = 0` in the pooler's config.
  export MARULA_DB_STATEMENT_CACHE=0
  echo "[e2e-stack] services dial the pooler; LISTEN and the DB suites stay direct"
fi
# Service stdio goes to per-run log files with stdin from /dev/null: the
# daemons must not inherit this script's stdout, or a caller that pipes `up`
# into a pager waits on an fd the daemons hold open forever.
LOG_DIR="$LOG_ROOT/$RUN_ID"
mkdir -p "$LOG_DIR"
echo "[e2e-stack] service logs: $LOG_DIR/{core-api,worker}.log"

########## shared cargo target guard (Wave 47 lane A17) ##########
# Every checkout on this machine compiles into ONE target dir (.cargo/config.toml
# → disk guardrails DX6), and cargo records dep-info paths RELATIVE to the
# workspace root, so an artifact a peer worktree built from a different source
# state resolves against THIS tree's files and passes the mtime freshness check.
# The result is a build that links code its sources never produced — the recorded
# signatures are a phantom `cannot find value …` and a `built for newer 'macOS'`
# linker error, both charged to innocent crates. The guard resets the workspace
# members FOR THIS PROFILE when the dir's last stamped build came from another
# tree, then `mark`s the dir after the builds. Release is explicit on purpose:
# `cargo clean -p …` WITHOUT `--release` does not clear a poisoned release
# artifact (proven 2026-10-16, .trail/learnings/).
# Profile of the core-api (and plugin-runtime worker) build. Release is the
# default because E2E latency is part of what the stack measures. `debug` is for
# the one case that needs a dev-only compile-time branch: the loopback egress
# allowance (F64) exists only in a debug build, so the calendar feed spec cannot
# run against a release binary. Nothing else changes — same ports, same DB,
# same seed.
API_PROFILE="${E2E_API_PROFILE:-release}"
case "$API_PROFILE" in
  release|debug) ;;
  *) die "E2E_API_PROFILE must be release or debug (got '$API_PROFILE')" ;;
esac
API_CARGO_FLAGS=""
[ "$API_PROFILE" = "release" ] && API_CARGO_FLAGS="--release"

if [ "${MARULA_LANE_GUARD:-1}" != "0" ]; then
  run_stage "shared-target guard (${API_PROFILE})" \
    "python3 scripts/lane-target-guard.py check --profile ${API_PROFILE} --fix" \
    || die "shared-target guard refused this build — read its reason above, run the fix it prints, or set MARULA_LANE_GUARD=0 to override deliberately"
fi

# The release builds are the slow part of a boot; cargo runs un-quiet here so
# its compile lines stream and the wait is visible.
run_stage "build core-api (${API_PROFILE} — E2E latency wants the optimized build; debug is opt-in)" \
  "cargo build -p core-api ${API_CARGO_FLAGS}"
BIN="$(cargo metadata -q --no-deps --format-version 1 | python3 -c \
  'import json,sys; print(json.load(sys.stdin)["target_directory"])')/${API_PROFILE}/core-api"
# The plugin runtime's worker is a bin target inside `plugin-host`, and
# `PluginPool` resolves it as a sibling of the core-api binary (`ENV_WORKER_BIN`
# overrides that). A stack that builds only core-api and worker therefore makes
# every module-bearing plugin invocation refuse with `plugin_worker_failed` —
# correct behaviour for a missing binary, and indistinguishable in a run from a
# plugin that is genuinely broken. Built here for the same reason `worker` is.
run_stage "build marula-plugin-worker (${API_PROFILE})" \
  "cargo build -p plugin-host --bin marula-plugin-worker ${API_CARGO_FLAGS}"
echo "[e2e-stack] boot core-api on ${BIND_ADDR}"
"$BIN" >"$LOG_DIR/core-api.log" 2>&1 </dev/null &
API_PID=$!
disown "$API_PID" 2>/dev/null || true
# The core-api release binary is now built from THIS tree's sources: claim the
# shared dir so the next lane's guard sees whose artifacts these are.
if [ "${MARULA_LANE_GUARD:-1}" != "0" ]; then
  run_stage "mark the shared target (${API_PROFILE} provenance)" \
    "python3 scripts/lane-target-guard.py mark --profile ${API_PROFILE} --note e2e-stack:core-api"
fi

API_HOST="$(echo "$BIND_ADDR" | sed 's/^[0-9]*://; s/^0\.0\.0\.0/localhost/')"
echo "[e2e-stack] waiting for core-api health on ${API_HOST} (up to 60s) …"
for _ in $(seq 1 60); do
  curl -fsS "http://${API_HOST}/api/v1/health" >/dev/null 2>&1 && break
  sleep 1
done
curl -fsS "http://${API_HOST}/api/v1/health" >/dev/null \
  || die "core-api never became healthy on ${API_HOST} — see $LOG_DIR/core-api.log"
echo "[e2e-stack] core-api healthy — demo env ready (owner@demo.local / demo-password-123)"

########## worker ##########
# The executor's home (Wave 43 lane 20, item G). The workflow stream
# consumer, the job queue that runs `workflow.resume`, and the `/api/internal`
# node endpoint all live behind the worker: without it booted, an event-
# triggered workflow has nobody to match the envelope, enqueue the run, or
# apply a node — which is why every workflow run before this lane needed a
# hand-rolled boot. Same shared target guard as the other release builds.
if [ "${MARULA_LANE_GUARD:-1}" != "0" ]; then
  run_stage "shared-target guard (release, worker)" \
    "python3 scripts/lane-target-guard.py check --profile release --fix" \
    || die "shared-target guard refused the worker build — read its reason above"
fi
run_stage "build worker (release)" 'cargo build -p worker --release'
WORKER_BIN="$(cargo metadata -q --no-deps --format-version 1 | python3 -c \
  'import json,sys; print(json.load(sys.stdin)["target_directory"])')/release/worker"
echo "[e2e-stack] boot worker on 0.0.0.0:${WORKER_PORT}"
# BOTH URLs point at this run's core-api. `CORE_API_URL` feeds the email and
# time loops; `INTERNAL_API_URL` is the seam the WORKFLOW executor posts node
# effects through (worker/effects.rs), so a stale default of 127.0.0.1:8080
# would make every node unreachable on a per-run port.
CORE_API_URL="http://${API_HOST}" \
  INTERNAL_API_URL="http://${API_HOST}" \
  BIND_ADDR="0.0.0.0:${WORKER_PORT}" \
  "$WORKER_BIN" >"$LOG_DIR/worker.log" 2>&1 </dev/null &
WORKER_PID=$!
disown "$WORKER_PID" 2>/dev/null || true
echo "[e2e-stack] waiting for worker health on 0.0.0.0:${WORKER_PORT} (up to 30s) …"
for _ in $(seq 1 30); do
  curl -fsS "http://localhost:${WORKER_PORT}/healthz" >/dev/null 2>&1 && break
  sleep 1
done
curl -fsS "http://localhost:${WORKER_PORT}/healthz" >/dev/null \
  || die "worker never became healthy on :${WORKER_PORT} — see $LOG_DIR/worker.log"
echo "[e2e-stack] worker healthy — workflows can trigger and execute on this stack"

# ########## env file for Playwright (kills NEXT_PUBLIC_API_URL drift) ##########
# `DATABASE_URL` here is the DIRECT database: Playwright and the DB suites query
# rows, they do not exercise the pooler. `E2E_APP_DATABASE_URL` is what the two
# booted services dial, so a spec that wants to prove the pooler is in the path
# can compare the two instead of guessing.
E2E_DIRECT_DATABASE_URL="${DIRECT_DATABASE_URL:-$DATABASE_URL}"
E2E_APP_DATABASE_URL="$DATABASE_URL"
cat > "$ENV_FILE" <<EOF
MARULA_RUN_ID=$RUN_ID
E2E_API_PROFILE=$API_PROFILE
E2E_DB_PROFILE=$DB_PROFILE
E2E_DB_IMAGE=$DB_IMAGE
E2E_API_URL=http://${API_HOST}/
E2E_API_PORT=$API_PORT
E2E_WEB_PORT=$WEB_PORT
E2E_WEB_URL=http://localhost:$WEB_PORT
E2E_WORKER_PORT=$WORKER_PORT
E2E_WORKER_URL=http://localhost:$WORKER_PORT
E2E_INTERNAL_TOKEN=$MARULA_INTERNAL_TOKEN
MARULA_INTERNAL_TOKEN=$MARULA_INTERNAL_TOKEN
E2E_DATABASE_URL=$E2E_DIRECT_DATABASE_URL
DATABASE_URL=$E2E_DIRECT_DATABASE_URL
E2E_APP_DATABASE_URL=$E2E_APP_DATABASE_URL
MARULA_TEST_DATABASE_URL=$TEST_DATABASE_URL
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
python3 - "$ENV_FILE" "$API_PID" "$WORKER_PID" "$REGISTRY_DIR" "$RUN_ID" "$FINGERPRINT" "$LOG_DIR" \
  "$OWNER_SESSION" "$OWNER_PID" "$OWNER_CMD" "$REPO_ROOT" "$CHECKOUT_BRANCH" "$EVIDENCE_DIR" <<'PY' || true
import json, os, sys, time
(envfile, api, worker, regdir, run_id, fingerprint, log_dir,
 owner_session, owner_pid, owner_cmd, checkout, branch, evidence_dir) = sys.argv[1:14]
env = {}
for line in open(envfile):
    line = line.strip()
    if "=" in line:
        k, v = line.split("=", 1)
        env[k] = v
reg = {
    "run_id": run_id,
    "started_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
    "started_epoch": int(time.time()),
    "env_fingerprint": fingerprint,
    "log_dir": log_dir,
    # Who owns this run, in the terms a sweep can check: the session that
    # booted it and the process that dies with that session.
    "owner": {
        "session": owner_session or None,
        "pid": int(owner_pid) if owner_pid.isdigit() else None,
        "cmd": owner_cmd or None,
    },
    "checkout": checkout or None,
    "branch": branch or None,
    "evidence_dir": evidence_dir or None,
    "env": env,
    "services": [
        {"name": "core-api", "pid": int(api), "source": "local", "port": env.get("E2E_API_PORT", ""),
         "log": os.path.join(log_dir, "core-api.log")},
        {"name": "worker", "pid": int(worker), "source": "local", "port": env.get("E2E_WORKER_PORT", ""),
         "log": os.path.join(log_dir, "worker.log")},
    ],
}
os.makedirs(regdir, exist_ok=True)
with open(os.path.join(regdir, f"run-{run_id}-pids.json"), "w") as f:
    json.dump(reg, f, indent=2)
PY
echo "[e2e-stack] owner: session ${OWNER_SESSION:-none recorded} pid ${OWNER_PID:-?} (${OWNER_CMD:-unknown command})"
echo "[e2e-stack] registered run in $REGISTRY_DIR — check scripts/dev-status.sh"
# A stalled or orphaned lane is otherwise found by whoever happens to look, so
# every boot announces the ones already on the machine. `--brief` prints decision
# lines only — no database URL, no env fingerprint — and never touches a run.
ATTENTION="$(DEV_STATUS_BRIEF=1 bash "$REPO_ROOT/scripts/dev-status.sh" --brief 2>/dev/null \
  | grep -E '^  (ORPHAN|OWNER UNKNOWN|WARN STALLED)' || true)"
if [ -n "$ATTENTION" ]; then
  echo "[e2e-stack] other registered runs need attention (scripts/dev-status.sh):"
  printf '%s\n' "$ATTENTION" | sed 's/^/    /'
fi
# Past this line the run is discoverable, so a later failure is a lane's own
# teardown (`down $RUN_ID`) and not a leak.
UP_COMPLETE=1

########## console summary — a boot is never silent ##########
echo "[e2e-stack] run $RUN_ID ready — api http://${API_HOST}/  worker :${WORKER_PORT}  web :${WEB_PORT}"
for _svc in core-api worker; do
  echo "[e2e-stack] --- $_svc log (last 5 lines) ---"
  tail -n 5 "$LOG_DIR/$_svc.log" 2>/dev/null | sed 's/^/    /' || true
done

if [ "${E2E_FOLLOW:-0}" = "1" ]; then
  # Live view: stay attached and stream every log. Ctrl-C stops following ONLY —
  # the stack keeps running; teardown stays `e2e-stack.sh down $RUN_ID`.
  echo "[e2e-stack] following service logs — Ctrl-C stops following (stack keeps running)"
  tail -n 0 -f "$LOG_DIR/core-api.log" "$LOG_DIR/worker.log" || true
  exit 0
fi

if [ "${E2E_FOREGROUND:-0}" = "1" ]; then
  # attach mode: EXIT trap tears down ONLY this run id.
  trap 'down_run "$RUN_ID"' EXIT
  wait "$API_PID"
fi
