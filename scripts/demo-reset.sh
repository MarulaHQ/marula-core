#!/usr/bin/env bash
# demo-reset.sh — the guarded reset for a demo instance (Wave 47 lane V6
# `p47-demo-reset-cron`).
#
# A demo instance has to look pristine for every prospect. This script reseeds
# the demo org, refreshes its SLA clocks, and keeps the two things the demo
# cannot lose: the at-rest key and the BYOK configuration.
#
# Usage:
#   scripts/demo-reset.sh                      # dry run: show the plan and counts
#   scripts/demo-reset.sh --apply --yes        # reset now, no prompt
#   scripts/demo-reset.sh --apply --hours 24   # shift SLA clocks back 24h
#   scripts/demo-reset.sh --snapshot-only      # back up BYOK, change nothing
#
# It assumes the demo overlay is up:
#   docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.demo.yml \
#     --profile demo up -d
#
# Hard rules, enforced here:
#   1. The API must be bound to 127.0.0.1. The dev surface has no bearer auth;
#      a reset over a public interface is refused.
#   2. MARULA_SECRET_KEY must be exported and is never rotated. The script
#      records a SHA-256 fingerprint before and after and refuses to continue
#      if the environment changed.
#   3. BYOK configuration is copied out before the reset and copied back after
#      it: byok_secrets, ai_model_credentials, ai_surface_grants, ai_caps and
#      ai_model_picks, scoped to the demo org. The copy is sealed ciphertext
#      and never leaves the host.
#   4. A reset is explicit: --apply is required, and a TTY asks for confirmation
#      unless --yes is passed. Cron passes both.
#
# Non-purpose: not a production data operation. It targets the fixed demo org
# (`marula-demo`) and refuses any other slug.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

API_URL="${DEMO_API_URL:-http://127.0.0.1:${DEMO_API_PORT:-8080}}"
DEMO_SLUG="marula-demo"
HOURS=24
APPLY=0
YES=0
SNAPSHOT_ONLY=0
SNAP_ROOT="${DEMO_RESET_DIR:-$REPO_ROOT/.demo-reset}"
TABLES=(byok_secrets ai_model_credentials ai_surface_grants ai_caps ai_model_picks)

die() { echo "[demo-reset] FAIL: $*" >&2; exit 1; }
note() { echo "[demo-reset] $*"; }

while [ $# -gt 0 ]; do
  case "$1" in
    --apply) APPLY=1; shift ;;
    --yes|-y) YES=1; shift ;;
    --hours) HOURS="${2:-}"; shift 2 ;;
    --snapshot-only) SNAPSHOT_ONLY=1; shift ;;
    help|-h|--help) sed -n '1,40p' "$0"; exit 0 ;;
    *) die "unknown argument $1" ;;
  esac
done
case "$HOURS" in *[!0-9]*|"") die "--hours must be a whole number of hours (1-720)" ;; esac
[ "$HOURS" -ge 1 ] && [ "$HOURS" -le 720 ] || die "--hours must be between 1 and 720"

for tool in docker curl python3 openssl; do
  command -v "$tool" >/dev/null 2>&1 || die "$tool is not available"
done

compose() {
  docker compose -f "$REPO_ROOT/deploy/docker-compose.yml" \
                 -f "$REPO_ROOT/deploy/docker-compose.demo.yml" \
                 --profile demo "$@"
}

# The demo profile is the only supported target: the reset needs /api/dev and
# the one-shot seed path. Refuse a plain base stack.
compose ps --status running --services 2>/dev/null | grep -qx core-api \
  || die "the demo overlay is not up — start it with: docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.demo.yml --profile demo up -d"

# Rule 1: loopback only. `docker compose port` prints the host binding.
binding="$(compose port core-api 8080 2>/dev/null | head -1 || true)"
case "$binding" in
  127.0.0.1:*) ;;
  "") die "core-api is not running or publishes no 8080 — start the demo overlay" ;;
  *) die "core-api is published on $binding, not 127.0.0.1 — the dev surface must stay on loopback. Fix deploy/docker-compose.demo.yml before resetting" ;;
esac

# Rule 2: the at-rest key is explicit and unchanged.
[ -n "${MARULA_SECRET_KEY:-}" ] || die "MARULA_SECRET_KEY is not exported. Source the same env file the stack uses (set -a; . deploy/demo.env; set +a) and retry"
key_fp="$(printf '%s' "$MARULA_SECRET_KEY" | openssl dgst -sha256 | awk '{print $NF}')"
note "at-rest key fingerprint ${key_fp:0:12}… (the key is never printed or rotated)"

status_json="$(curl -fsS "$API_URL/api/dev/status" 2>/dev/null)" \
  || die "cannot reach $API_URL/api/dev/status — is the demo API up and on loopback?"
python3 - <<'PY' "$status_json" "$DEMO_SLUG"
import json, sys
d = json.loads(sys.argv[1])
slug = sys.argv[2]
if not d.get("enabled"):
    print("[demo-reset] FAIL: /api/dev is not mounted — start the demo overlay", file=sys.stderr)
    raise SystemExit(1)
if d.get("demo_slug") != slug:
    print(f"[demo-reset] FAIL: API serves demo slug {d.get('demo_slug')!r}, expected {slug!r}", file=sys.stderr)
    raise SystemExit(1)
PY

org_id="$(compose exec -T postgres psql -U marula -d marula -tAc \
  "select id from orgs where slug='$DEMO_SLUG'" 2>/dev/null | tr -d '[:space:]')"
[ -n "$org_id" ] || die "no org with slug '$DEMO_SLUG' — seed the demo first (docker compose ... --profile demo run --rm demo-seed)"
note "demo org $org_id"

mkdir -p "$SNAP_ROOT"
stamp="$(date +%Y%m%dT%H%M%S)"
snap="$SNAP_ROOT/$stamp"
mkdir -p "$snap"
chmod 700 "$SNAP_ROOT" "$snap"
printf '%s\n' "$key_fp" > "$snap/secret-key.sha256"
chmod 600 "$snap/secret-key.sha256"

# Snapshot only the demo org. Each table carries org_id; the predicate is
# repeated rather than trusting a search_path.
for table in "${TABLES[@]}"; do
  file="$snap/$table.copy"
  compose exec -T postgres psql -U marula -d marula -v ON_ERROR_STOP=1 \
    -c "\\copy (SELECT * FROM $table WHERE org_id = '$org_id') TO STDOUT WITH (FORMAT csv)" \
    > "$file" || die "could not snapshot $table"
  chmod 600 "$file"
  rows="$(compose exec -T postgres psql -U marula -d marula -tAc \
    "select count(*) from $table where org_id='$org_id'" 2>/dev/null | tr -d '[:space:]')"
  printf '%s\n' "${rows:-0}" > "$snap/$table.count"
  chmod 600 "$snap/$table.count"
  note "snapshot $table: ${rows:-0} rows"
done

if [ "$SNAPSHOT_ONLY" = 1 ]; then
  note "snapshot only — nothing changed. Snapshot: $snap"
  exit 0
fi

note "plan: POST /api/dev/reset-org, restore ${#TABLES[@]} BYOK tables, time-travel ${HOURS}h"
note "snapshot: $snap"
if [ "$APPLY" != 1 ]; then
  note "dry run — pass --apply (and --yes for cron) to reset"
  exit 0
fi
if [ "$YES" != 1 ] && [ -t 0 ]; then
  printf '[demo-reset] reset the demo org and restore BYOK? [y/N] '
  read -r answer
  case "$answer" in y|Y|yes|YES) ;; *) note "cancelled"; exit 1 ;; esac
fi

note "resetting the demo org …"
curl -fsS -X POST "$API_URL/api/dev/reset-org" >/dev/null \
  || die "reset-org failed — the org may be gone and the snapshot is intact at $snap. Re-run once the API answers; do not start a new snapshot"
note "reset complete — restoring BYOK …"

for table in "${TABLES[@]}"; do
  file="$snap/$table.copy"
  if [ ! -s "$file" ]; then
    note "restore $table: nothing to restore"
    continue
  fi
  compose exec -T postgres psql -U marula -d marula -v ON_ERROR_STOP=1 \
    -c "\\copy $table FROM STDIN WITH (FORMAT csv)" < "$file" \
    || die "restore of $table failed — snapshot is intact at $snap; restore it before any other reset"
  note "restore $table: ok"
done

note "shifting SLA clocks back ${HOURS}h …"
curl -fsS -X POST "$API_URL/api/dev/time-travel" \
  -H 'content-type: application/json' -d "{\"hours\":$HOURS}" >/dev/null \
  || die "time-travel failed — the reset and BYOK restore succeeded; re-run only the time-travel"

# Verify the two hard rules after the fact.
after_fp="$(printf '%s' "$MARULA_SECRET_KEY" | openssl dgst -sha256 | awk '{print $NF}')"
[ "$after_fp" = "$key_fp" ] || die "MARULA_SECRET_KEY changed during the reset — the restored BYOK ciphertext cannot be opened. Stop and restore the environment"
for table in "${TABLES[@]}"; do
  expected="$(cat "$snap/$table.count")"
  actual="$(compose exec -T postgres psql -U marula -d marula -tAc \
    "select count(*) from $table where org_id='$org_id'" 2>/dev/null | tr -d '[:space:]')"
  [ "$actual" = "$expected" ] || die "$table has $actual rows after restore, expected $expected — inspect $snap"
done
note "verified: at-rest key unchanged, BYOK row counts restored"

# Keep the five most recent snapshots; they are sealed ciphertext, not keys.
ls -1dt "$SNAP_ROOT"/*/ 2>/dev/null | tail -n +6 | while IFS= read -r old; do rm -rf "$old"; done

cat <<EOF
[demo-reset] demo is pristine.
  logins: owner@demo.local / dispatcher@demo.local / tech@demo.local / bookkeeper@demo.local
  password: demo-password-123
  SLA clocks: ${HOURS}h back
  BYOK: restored (${TABLES[*]})
  snapshot: $snap
EOF
