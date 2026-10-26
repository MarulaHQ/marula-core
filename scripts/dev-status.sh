#!/usr/bin/env bash
# Process registry for agents (report §3.5): list the running Marula stack
# — pids, ports, env fingerprint, owner and activity — so a lane stops guessing
# which process it may kill. Sources of truth:
#   1. run-*-pids.json entries written by scripts/e2e-stack.sh, read from the
#      SHARED root's .kilo/dev-registry/ (the main checkout) PLUS any linked
#      worktree's own registry — so a stack booted inside a lane worktree is
#      still visible from here.
#   2. live docker containers matching marula-e2e-* (image pgvector/valkey)
#   3. listening ports held by the registered processes (re-checked live)
#
# `--reap` reclaims runs whose OWNER IS GONE and refuses every run whose owner
# is alive (Wave 47 lane A23). The registry carries the owning session id and
# the pid of the process that dies with that session, so "gone" and "idle" are
# different answers; without that split, a sweep would either kill a lane that
# is thinking or leave a cancelled lane's stack holding ports for hours (A21
# held three for five). `--reap` is honest in A17's sense: it prints what it
# removed, verifies the run is gone afterwards, and exits non-zero when a
# target it claimed could not be reclaimed.
#
# `--brief` prints only the decision line per run, with no database URL and no
# env fingerprint, so another long-running command can announce a stalled or
# orphaned lane without spraying state: `e2e-stack.sh up` calls it and names
# what needs attention before it finishes booting.
#
# A live-but-stalled owner is also surfaced, not reaped: every entry states its
# last commit (and its evidence dir's newest file, when the lane recorded one),
# and one that has produced nothing for MARULA_STALL_HOURS (default 4) prints
# a STALLED line instead of quietly holding ports.
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

MODE=status
STALL_HOURS="${MARULA_STALL_HOURS:-4}"
for arg in "$@"; do
  case "$arg" in
    --reap) MODE=reap ;;
    --status) MODE=status ;;
    --self-test) MODE=selftest ;;
    --brief) MODE=brief ;;
    --stall-hours=*) STALL_HOURS="${arg#*=}" ;;
    -h|--help)
      sed -n '2,30p' "$0"; exit 0 ;;
    *)
      echo "dev-status: unknown option '$arg' (try --reap, --stall-hours=N, or no option)" >&2
      exit 2 ;;
  esac
done

STACK_SCRIPT="$(cd "$(dirname "$0")" && pwd)/e2e-stack.sh"
REGISTRY_DIR="${REGISTRY_DIR:-$SHARED_ROOT/.kilo/dev-registry}"
DOCKER_BIN="${DOCKER:-docker}"

# ---------- the registry files, wherever they live ----------
registry_files() {
  local pat f
  for pat in "$REGISTRY_DIR/run-"*-pids.json \
             "$SHARED_ROOT/.kilo/worktrees/"*"/.kilo/dev-registry/run-"*-pids.json; do
    for f in $pat; do
      [ -f "$f" ] && printf '%s\n' "$f"
    done
  done
  return 0
}

# ---------- --reap: reclaim runs whose owner is gone, never a live one ----------
reap() {
  local files
  files="$(registry_files)"
  if [ -z "$files" ]; then
    echo "== reap =="
    echo "  no registered runs — nothing to reap."
    echo "dev-status --reap: 0 reclaimed, 0 kept, 0 refused (no registry entries)."
    return 0
  fi
  # shellcheck disable=SC2086 # the file list is newline-split on purpose
  python3 - "$STACK_SCRIPT" "$DOCKER_BIN" $files <<'PY'
import json, os, shutil, subprocess, sys, time

stack_script, docker_bin = sys.argv[1], sys.argv[2]
files = sys.argv[3:]

def pid_alive(pid):
    try:
        os.kill(int(pid), 0)
        return True
    except (ProcessLookupError, TypeError, ValueError):
        return False
    except PermissionError:
        return True  # someone else's process: alive, and not ours to kill

def container_names(run):
    return [f"marula-e2e-pg-{run}", f"marula-e2e-redis-{run}"]

def containers_up(run):
    if shutil.which(docker_bin) is None:
        return None  # unknown: no docker to ask
    up = []
    for name in container_names(run):
        if subprocess.run([docker_bin, "inspect", name],
                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0:
            up.append(name)
    return up

reclaimed, kept, refused, failed = [], [], [], []
print("== reap ==")
for path in files:
    try:
        d = json.load(open(path))
    except Exception as exc:  # a half-written entry is not a licence to guess
        refused.append((path, f"unreadable registry entry ({exc})"))
        continue
    run = d.get("run_id") or os.path.basename(path).removeprefix("run-").removesuffix("-pids.json")
    owner = d.get("owner") or {}
    pid, session = owner.get("pid"), owner.get("session") or "none recorded"

    if not pid:
        refused.append((run, "no owner recorded — cannot tell gone from idle"))
        continue
    if pid_alive(pid):
        kept.append(run)
        print(f"  KEEP    {run}: owner alive (pid {pid}, session {session}) — not touched")
        continue

    services = [s for s in d.get("services", []) if pid_alive(s.get("pid"))]
    boxes = containers_up(run)
    print(f"  ORPHAN  {run}: owner pid {pid} is gone (session {session}); "
          f"{len(services)} live service(s), "
          f"{'docker unavailable' if boxes is None else str(len(boxes)) + ' container(s)'}")

    proc = subprocess.run(["bash", stack_script, "down", run],
                          capture_output=True, text=True)
    out = (proc.stdout + proc.stderr).strip()
    if out:
        print("\n".join("          " + line for line in out.splitlines()))

    # Verify the effect, not the report: a teardown can exit 0 having removed
    # nothing, which is exactly the failure A17 fixed for `down`.
    deadline = time.time() + 5
    still = [s for s in d.get("services", []) if pid_alive(s.get("pid"))]
    while still and time.time() < deadline:
        time.sleep(0.2)
        still = [s for s in d.get("services", []) if pid_alive(s.get("pid"))]
    boxes_after = containers_up(run)
    entry_gone = not os.path.exists(path)

    if still or (boxes_after not in (None, [])) or not entry_gone:
        failed.append(run)
        print(f"  FAIL    {run}: still present after down — "
              f"{len(still)} live service(s), "
              f"{'docker unavailable' if boxes_after is None else str(len(boxes_after)) + ' container(s)'}, "
              f"registry entry {'kept' if not entry_gone else 'removed'}")
        continue

    removed_work = bool(services) or bool(boxes)
    containers_line = ("docker unavailable — container removal not verified"
                       if boxes is None else f"{len(boxes)} container(s) removed")
    if removed_work:
        reclaimed.append(run)
        print(f"  REAPED  {run}: {len(services)} process(es) stopped, {containers_line}")
    else:
        reclaimed.append(run)
        print(f"  REAPED  {run}: nothing was running — reclaimed the stale registry entry only")

for run, why in refused:
    print(f"  REFUSED {run}: {why}")
    print(f"          reclaim by hand once the owner is known: bash scripts/e2e-stack.sh down {run}")

print()
print(f"dev-status --reap: {len(reclaimed)} reclaimed, {len(kept)} kept (owner alive), "
      f"{len(refused)} refused, {len(failed)} failed.")
if failed:
    print("A run reported failed above is still holding resources; the sweep did NOT report success.",
          file=sys.stderr)
    sys.exit(1)
sys.exit(0)
PY
}

# ---------- self-test: the reaper's three answers, without a stack ----------
# A fake docker (the same shape scripts/e2e-stack.sh's self-test uses) plus a
# real `sleep` standing in for a service pid lets the sweep's decisions be
# asserted here: an orphan is reclaimed, a live owner is refused, an entry with
# no recorded owner is refused, and a run that survives its teardown is a
# FAILURE with a non-zero exit — never a success line.
selftest() {
  local tmp failures=0 checks=0 out rc svc
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/dev-status-selftest.XXXXXX")" || return 1
  trap "rm -rf '$tmp'" EXIT
  mkdir -p "$tmp/reg" "$tmp/logs"
  cat > "$tmp/docker" <<'FAKE'
#!/usr/bin/env bash
state="${FAKE_DOCKER_STATE:?}"
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

  check() { # check <label> <got> <want>
    checks=$((checks + 1))
    if [ "$2" = "$3" ]; then
      echo "  PASS  $1"
    else
      echo "  FAIL  $1: got '$2', want '$3'"
      failures=$((failures + 1))
    fi
  }

  run_reap() {
    rc=0
    out="$(DOCKER="$tmp/docker" FAKE_DOCKER_STATE="$tmp/state" FAKE_DOCKER_LOG="$tmp/argv.log" \
           REGISTRY_DIR="$tmp/reg" LOG_ROOT="$tmp/logs" E2E_ENV_FILE="$tmp/no-such.env" \
           bash "$SELF" --reap 2>&1)" || rc=$?
  }

  write_entry() { # write_entry <run> <owner-pid-or-empty> <service-pid>
    python3 - "$tmp/reg/run-$1-pids.json" "$1" "$2" "$3" <<'PY'
import json, sys, time
path, run, owner_pid, service_pid = sys.argv[1:5]
entry = {
    "run_id": run,
    "started_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
    "started_epoch": int(time.time()),
    "env_fingerprint": "selftest",
    "log_dir": "/nonexistent",
    "owner": {"session": "ses_selftest", "pid": int(owner_pid) if owner_pid else None, "cmd": "selftest"},
    "checkout": None,
    "branch": None,
    "evidence_dir": None,
    "env": {},
    "services": [{"name": "core-api", "pid": int(service_pid), "source": "local", "port": "1"}],
}
json.dump(entry, open(path, "w"), indent=2)
PY
  }

  SELF="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"

  echo "dev-status self-test: the reaper (fake docker, no stack)"

  # 1. owner gone, work still running: reclaimed, and the effects are checked.
  sleep 300 >/dev/null 2>&1 & svc=$!; disown "$svc" 2>/dev/null || true
  : > "$tmp/state"
  printf '%s\n' marula-e2e-pg-e2e-9001 marula-e2e-redis-e2e-9001 > "$tmp/state"
  write_entry e2e-9001 999999 "$svc"
  run_reap
  check "an orphan with work running exits 0" "$rc" "0"
  check "the orphan is named ORPHAN" "$(printf '%s' "$out" | grep -c 'ORPHAN  e2e-9001')" "1"
  check "the orphan is named REAPED" "$(printf '%s' "$out" | grep -c 'REAPED  e2e-9001')" "1"
  check "the service process is actually gone" "$(kill -0 "$svc" 2>/dev/null && echo alive || echo gone)" "gone"
  check "the containers are actually gone" "$(wc -l < "$tmp/state" | tr -d ' ')" "0"
  check "the registry entry is actually gone" "$([ -f "$tmp/reg/run-e2e-9001-pids.json" ] && echo kept || echo removed)" "removed"

  # 2. owner alive: refused, and its work is untouched.
  sleep 300 >/dev/null 2>&1 & svc=$!; disown "$svc" 2>/dev/null || true
  : > "$tmp/state"
  write_entry e2e-9002 "$$" "$svc"
  run_reap
  check "a live owner exits 0" "$rc" "0"
  check "a live owner is kept, not reaped" "$(printf '%s' "$out" | grep -c 'KEEP    e2e-9002')" "1"
  check "a live owner's service survives the sweep" "$(kill -0 "$svc" 2>/dev/null && echo alive || echo gone)" "alive"
  kill "$svc" 2>/dev/null || true
  rm -f "$tmp/reg/run-e2e-9002-pids.json"

  # 3. no recorded owner: cannot tell gone from idle, so refuse and name the command.
  sleep 300 >/dev/null 2>&1 & svc=$!; disown "$svc" 2>/dev/null || true
  write_entry e2e-9003 "" "$svc"
  run_reap
  check "an entry with no owner is refused" "$(printf '%s' "$out" | grep -c 'REFUSED e2e-9003')" "1"
  check "the refusal names the manual command" "$(printf '%s' "$out" | grep -c 'e2e-stack.sh down e2e-9003')" "1"
  check "the unowned run is left alone" "$(kill -0 "$svc" 2>/dev/null && echo alive || echo gone)" "alive"
  # The listing must not call an unowned run an orphan: from outside, gone and
  # idle are the same picture, which is why the sweep refuses it (A23 item D).
  rc=0
  out="$(DOCKER="$tmp/docker" FAKE_DOCKER_STATE="$tmp/state" REGISTRY_DIR="$tmp/reg" \
         LOG_ROOT="$tmp/logs" E2E_ENV_FILE="$tmp/no-such.env" bash "$SELF" 2>&1)" || rc=$?
  check "the listing calls an unowned run OWNER UNKNOWN, not ORPHAN" \
    "$(printf '%s' "$out" | grep -c 'OWNER UNKNOWN')" "1"
  # `--brief` is what another long-running command reads, so it must carry the
  # decision without the run's credentials.
  rc=0
  out="$(DOCKER="$tmp/docker" FAKE_DOCKER_STATE="$tmp/state" REGISTRY_DIR="$tmp/reg" \
         LOG_ROOT="$tmp/logs" E2E_ENV_FILE="$tmp/no-such.env" bash "$SELF" --brief 2>&1)" || rc=$?
  check "brief mode still names the unowned run" \
    "$(printf '%s' "$out" | grep -c 'OWNER UNKNOWN')" "1"
  check "brief mode prints no database URL" \
    "$(printf '%s' "$out" | grep -c 'DATABASE_URL')" "0"
  check "brief mode prints no env fingerprint" \
    "$(printf '%s' "$out" | grep -c 'fingerprint')" "0"
  kill "$svc" 2>/dev/null || true
  rm -f "$tmp/reg/run-e2e-9003-pids.json"

  # 4. owner gone and nothing left running: honest wording, not "removed" work.
  : > "$tmp/state"
  write_entry e2e-9004 999999 999998
  run_reap
  check "an empty orphan reclaims the record only" \
    "$(printf '%s' "$out" | grep -c 'reclaimed the stale registry entry only')" "1"
  check "an empty orphan does not claim to have stopped a process" \
    "$(printf '%s' "$out" | grep -c 'process(es) stopped')" "0"

  # 5. an orphan that survives its teardown: a FAILED run, non-zero exit.
  bash -c 'trap "" TERM; sleep 300' >/dev/null 2>&1 & svc=$!; disown "$svc" 2>/dev/null || true
  sleep 0.2
  : > "$tmp/state"
  printf '%s\n' marula-e2e-pg-e2e-9005 > "$tmp/state"
  write_entry e2e-9005 999999 "$svc"
  run_reap
  check "a run that survives teardown exits non-zero" "$rc" "1"
  check "a run that survives teardown is named FAIL" "$(printf '%s' "$out" | grep -c 'FAIL    e2e-9005')" "1"
  check "the sweep does not claim success" "$(printf '%s' "$out" | grep -c 'dev-status --reap: .*0 failed')" "0"
  kill -9 "$svc" 2>/dev/null || true
  rm -f "$tmp/reg"/run-e2e-900*-pids.json

  if [ "$failures" -eq 0 ]; then
    echo "dev-status self-test: $checks checks pass"
    return 0
  fi
  echo "dev-status self-test: $failures of $checks failed"
  return 1
}

if [ "$MODE" = "selftest" ]; then
  selftest
  exit $?
fi

if [ "$MODE" = "reap" ]; then
  reap
  exit $?
fi

BRIEF=0
[ "$MODE" = "brief" ] && BRIEF=1
export DEV_STATUS_BRIEF="$BRIEF"

echo "== registered e2e-stack runs (run-*-pids.json) =="
files="$(registry_files)"
if [ -n "$files" ]; then
  # shellcheck disable=SC2086 # the file list is newline-split on purpose
  python3 - "$SHARED_ROOT" "$STALL_HOURS" $files <<'PY'
import json, os, subprocess, sys, time

shared_root, stall_hours = sys.argv[1], float(sys.argv[2])
files = sys.argv[3:]
BRIEF = os.environ.get("DEV_STATUS_BRIEF") == "1"

def pid_alive(pid):
    try:
        os.kill(int(pid), 0)
        return True
    except (ProcessLookupError, TypeError, ValueError):
        return False
    except PermissionError:
        return True

def state(pid):
    if not pid:
        return "unknown"
    return f"alive pid {pid}" if pid_alive(pid) else f"gone (pid {pid} recorded)"

def last_commit(checkout, branch):
    """Newest commit time on the owning checkout's branch, or None."""
    if not checkout or not os.path.isdir(checkout):
        return None
    try:
        out = subprocess.run(
            ["git", "-C", checkout, "log", "-1", "--format=%ct", branch or "HEAD"],
            capture_output=True, text=True, timeout=5)
        return float(out.stdout.strip()) if out.stdout.strip().isdigit() else None
    except Exception:
        return None

def newest_file(root):
    """Newest mtime under the lane's recorded evidence dir, or None."""
    if not root or not os.path.isdir(root):
        return None
    newest = None
    for base, _dirs, names in os.walk(root):
        for name in names:
            try:
                mtime = os.path.getmtime(os.path.join(base, name))
            except OSError:
                continue
            if newest is None or mtime > newest:
                newest = mtime
    return newest

stalled = orphaned = unknown = 0
now = time.time()
for path in files:
    origin = shared_root
    if "/worktrees/" in path:
        origin = "worktree " + path.split("/worktrees/", 1)[1].split("/.kilo/dev-registry/")[0]
    try:
        d = json.load(open(path))
    except Exception as exc:
        print(f"\nrun ?  [{origin}] unreadable registry entry: {exc}")
        continue
    env = d.get("env", {})
    owner = d.get("owner") or {}
    pid = owner.get("pid")
    session = owner.get("session") or "none recorded"
    if BRIEF:
        print(f"run {d.get('run_id', '?')}  [{origin}]")
    else:
        print(f"\nrun {d.get('run_id', '?')}  started {d.get('started_at', '?')}  "
              f"fingerprint {d.get('env_fingerprint', '?')}  [{origin}]")
        print(f"  DATABASE_URL : {env.get('E2E_DATABASE_URL', '?')}")
        print(f"  JWT_SECRET   : {env.get('JWT_SECRET', '?')[:12]}… (fingerprint field — keep secret)")
        print(f"  owner        : session {session}, "
              f"{state(pid) if pid else 'no pid recorded — gone and idle look the same'}"
              f"{' — ' + owner.get('cmd', '') if owner.get('cmd') else ''}")
        for s in d.get("services", []):
            print(f"  {s.get('name', '?'):12} port={s.get('port', '?'):6} {state(s.get('pid'))}")
        if d.get("log_dir"):
            print(f"  logs         : {d['log_dir']}")

    # Activity: the newest of (last commit on the owning branch, newest file in
    # the evidence dir the lane recorded). Silence alone is not proof of a
    # stall — an instrument can fail after measuring and before writing — so
    # this reports what it measured and never kills anything.
    commit_epoch = last_commit(d.get("checkout"), d.get("branch"))
    evidence_epoch = newest_file(d.get("evidence_dir"))
    newest = max([t for t in (commit_epoch, evidence_epoch) if t is not None], default=None)
    signals = []
    if commit_epoch is not None:
        signals.append(f"commit {(now - commit_epoch) / 3600:.1f}h ago")
    if evidence_epoch is not None:
        signals.append(f"evidence {(now - evidence_epoch) / 3600:.1f}h ago")
    if newest is None:
        started = d.get("started_epoch")
        newest = float(started) if started else None
        signals = ["no commit or evidence recorded; counting from the boot"]
    idle_hours = (now - newest) / 3600 if newest is not None else None

    if not pid:
        # Nothing recorded for the owner. Silence on its own is not proof the
        # owner died (an instrument can fail after measuring), so this is named
        # for a human instead of being called an orphan.
        unknown += 1
        print(f"  OWNER UNKNOWN no owner was recorded for this run, so gone cannot be told "
              f"from idle — tear down by hand: bash scripts/e2e-stack.sh down {d.get('run_id', '?')}")
    elif not pid_alive(pid):
        orphaned += 1
        print(f"  ORPHAN       owner gone — reclaim: bash scripts/dev-status.sh --reap")
    elif idle_hours is not None and idle_hours >= stall_hours:
        stalled += 1
        print(f"  WARN STALLED owner alive and nothing landed for {idle_hours:.1f}h "
              f"(limit {stall_hours:g}h, {'; '.join(signals)})")
        print(f"               checkpoint or tear down: bash scripts/e2e-stack.sh down "
              f"{d.get('run_id', '?')}")
    else:
        print(f"  activity     : {'; '.join(signals)} — active")

print()
print(f"{len(files)} registered run(s): {orphaned} orphaned, {stalled} stalled, "
      f"{unknown} with no recorded owner (stall limit {stall_hours:g}h, "
      f"MARULA_STALL_HOURS overrides).")
PY
else
  echo "  (none)"
fi

# Brief mode stops here: another command is calling this for the decision lines,
# and the container table plus the process scan are for a human reading it.
if [ "$BRIEF" = "1" ]; then
  exit 0
fi

echo
echo "== live marula-e2e-* containers =="
if command -v docker >/dev/null 2>&1; then
  docker ps --filter "name=marula-e2e" --format \
    "table {{.Names}}\t{{.Image}}\t{{.Ports}}\t{{.Status}}" 2>/dev/null \
    | { read -r header; echo "  $header"; while read -r l; do echo "  $l"; done; }
  echo
  # A live container whose run has no registry entry is either a boot that never
  # registered (it died before the health check) or a lane that deleted the
  # registry file. Either way nothing else names it, so it is listed here with
  # the exact command that reclaims it. `--reap` cannot take it (there is no
  # owner to check), which is why it is named rather than swept.
  unregistered=0
  for name in $(docker ps --filter "name=marula-e2e" --format '{{.Names}}' 2>/dev/null); do
    run="${name#marula-e2e-pg-}"
    [ "$run" = "$name" ] && run="${name#marula-e2e-redis-}"
    registered=no
    for f in "$SHARED_ROOT/.kilo/dev-registry/run-$run-pids.json" \
             "$SHARED_ROOT/.kilo/worktrees/"*"/.kilo/dev-registry/run-$run-pids.json"; do
      [ -f "$f" ] && registered=yes && break
    done
    if [ "$registered" = "no" ]; then
      if [ "$unregistered" -eq 0 ]; then
        echo "  UNREGISTERED (no registry entry — nothing else names these):"
      fi
      unregistered=$((unregistered + 1))
      echo "    $name"
      echo "      reclaim:  bash scripts/e2e-stack.sh down $run"
    fi
  done
  echo
  echo "To inspect:  docker logs marula-e2e-pg-<run-id>"
  echo "Service logs:  $SHARED_ROOT/.kilo/e2e-logs/<run-id>/{core-api,worker}.log"
else
  echo "  (docker unavailable)"
fi

echo
echo "== other common Marula processes (core-api / worker / next standalone) =="
ps -axo pid,command | grep -E "release/(core-api|worker)|\.next/standalone/server\.js|marula-core-cli" \
  | grep -v grep | sed 's/^/  /' || echo "  (none)"
echo
echo "Reap orphaned runs:  bash scripts/dev-status.sh --reap"
echo "Teardown: bash scripts/e2e-stack.sh down <RUN_ID>   # run ids are e2e-<digits> (bare digits work too)."
echo "          A teardown that removes nothing now exits non-zero and names what it looked for,"
echo "          so a run that is still up after a down is a failure, not a silent success."
