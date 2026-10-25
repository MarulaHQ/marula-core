#!/usr/bin/env sh
# Assert that backup retention actually culls.
#
# Why a script and not a reading of the line: `wal-g delete` is a DRY RUN
# unless `--confirm` is on the command line. The runner called it bare for a
# long time, which looks like retention in every log it prints and deletes
# nothing — so the evidence has to be the argv wal-g receives, not the prose
# in the script. This drives the real backupd.sh with a stub wal-g on PATH,
# records what it was called with, and asserts the flag is there.
#
# It also asserts the opposite direction: the rehearsal path must NOT pass
# --confirm, so an operator who asks for a dry run gets one. A check that only
# looked for --confirm would pass while dry-run mode silently deleted backups.
#
# Usage:   deploy/backup/check-retention.sh
# Requires: sh only. No daemon, no image build, no network, no database.
# Does NOT: touch a bucket, a container, or the repository's working tree.

set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
BACKUP_SH="$ROOT/deploy/backup/backupd.sh"
ENV_EXAMPLE="$ROOT/deploy/backup/wal-g.env.example"
FAIL=0
PASS=0

ok()   { PASS=$((PASS + 1)); printf '  ok   %s\n' "$*"; }
bad()  { FAIL=$((FAIL + 1)); printf '  FAIL %s\n' "$*"; }
head() { printf '\n== %s\n' "$*"; }

[ -f "$BACKUP_SH" ] || { echo "missing $BACKUP_SH" >&2; exit 2; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin" "$tmp/dumps"

# The stub records one line per delete invocation and answers the two other
# wal-g calls the pass makes. Anything else exits 0: this check is about
# retention argv, not about replaying wal-g.
cat >"$tmp/bin/wal-g" <<'STUB'
#!/bin/sh
case "$1 $2" in
  "backup-push --verify") exit 0 ;;
  "wal-verify integrity") printf '%s\n' '[wal-verify] integrity check status: OK'; exit 0 ;;
  "delete retain") printf '%s\n' "$*" >>"$WALG_CALLS"; exit 0 ;;
esac
exit 0
STUB
cat >"$tmp/bin/psql" <<'STUB'
#!/bin/sh
if [ "${PSQL_FAIL:-0}" = 1 ]; then
  printf '%s\n' 'psql: error: connection to server at "postgres", port 5432 failed: password authentication failed' >&2
  exit 2
fi
case "$*" in
  *data_checksums*) printf '%s\n' on ;;
  *) printf '%s\n' 1 ;;
esac
exit 0
STUB
chmod 0755 "$tmp/bin/wal-g" "$tmp/bin/psql"

# One pass of the real runner against the stubs. $1 = label, rest = extra env.
run_pass() {
	_label=$1
	shift
	: >"$tmp/calls"
	# shellcheck disable=SC2086
	env PATH="$tmp/bin:$PATH" \
		BACKUP_RUN_ONCE=1 \
		BACKUP_STAMP_FILE="$tmp/stamp.$_label" \
		PGDUMPS_DIR="$tmp/dumps" \
		DRILL_EVERY_DAYS=0 \
		WALG_S3_PREFIX=s3://check/pg \
		WALG_CALLS="$tmp/calls" \
		AWS_ACCESS_KEY_ID=check-access \
		AWS_SECRET_ACCESS_KEY=check-secret \
		"$@" sh "$BACKUP_SH" >"$tmp/$_label.log" 2>&1
	rc=$?
	# set -e is not used around this call on purpose: a non-zero here is data.
	return $rc
}

head "the pass culls by default"
if run_pass cull WALG_RETAIN_FULL=4; then
	ok "the pass exited 0"
else
	bad "the pass exited non-zero (log: $tmp/cull.log)"
	sed 's/^/       /' "$tmp/cull.log"
fi
if grep -q 'delete retain FULL 4' "$tmp/calls" 2>/dev/null; then
	ok "wal-g delete retain FULL 4 was invoked"
else
	bad "wal-g delete retain FULL 4 was never invoked"
fi
if grep -q -- '--confirm' "$tmp/calls" 2>/dev/null; then
	ok "--confirm was passed: wal-g culls instead of listing"
else
	bad "no --confirm: retention is a silent dry run again"
fi
if grep -q -- '--use-sentinel-time' "$tmp/calls" 2>/dev/null; then
	ok "--use-sentinel-time orders by backup start time, not by timeline name"
else
	bad "--use-sentinel-time missing: retain can delete the newest backups after a major upgrade"
fi

head "the rehearsal is explicit and deletes nothing"
if run_pass dryrun WALG_RETAIN_FULL=4 WALG_RETENTION_DRY_RUN=1; then
	ok "the dry-run pass exited 0"
else
	bad "the dry-run pass exited non-zero (log: $tmp/dryrun.log)"
fi
if grep -q 'delete retain FULL 4' "$tmp/calls" 2>/dev/null; then
	ok "wal-g delete retain FULL 4 was invoked (to list what it would delete)"
else
	bad "the dry run never invoked wal-g"
fi
if grep -q -- '--confirm' "$tmp/calls" 2>/dev/null; then
	bad "the dry run passed --confirm: an operator asking to rehearse just deleted backups"
else
	ok "no --confirm: the dry run deletes nothing"
fi
if grep -q 'retention DRY RUN' "$tmp/dryrun.log"; then
	ok "the pass says DRY RUN in its own log"
else
	bad "the dry run is not announced in the log"
fi

head "no retention configured means no delete"
if run_pass off WALG_RETAIN_FULL=; then
	ok "the pass exited 0 with WALG_RETAIN_FULL empty"
else
	bad "the pass exited non-zero with WALG_RETAIN_FULL empty"
fi
if [ -s "$tmp/calls" ]; then
	bad "wal-g delete ran with WALG_RETAIN_FULL empty: $(cat "$tmp/calls")"
else
	ok "wal-g delete did not run (empty = keep everything, deliberately)"
fi

head "a malformed count fails the pass instead of being passed to wal-g"
run_pass nonnumeric WALG_RETAIN_FULL=lots || true
if [ -s "$tmp/calls" ]; then
	bad "wal-g delete ran with WALG_RETAIN_FULL=lots"
else
	ok "wal-g delete did not run"
fi
if grep -q 'WALG_RETAIN_FULL=lots is not a number' "$tmp/nonnumeric.log"; then
	ok "the pass names the malformed value"
else
	bad "the malformed value is not reported"
fi

head "a refused connection fails the pass by name"
if run_pass noconn PSQL_FAIL=1 WALG_RETAIN_FULL=4; then
	bad "the pass exited 0 with the database unreachable"
else
	ok "the pass exited non-zero with the database unreachable"
fi
if grep -q 'cannot connect to marula@postgres:5432/marula' "$tmp/noconn.log"; then
	ok "the log names the connection it tried"
else
	bad "the log does not name the connection"
fi
if grep -q 'Set PGPASSWORD or PGPASSFILE' "$tmp/noconn.log"; then
	ok "the log names the variable that fixes it"
else
	bad "the log does not name PGPASSWORD/PGPASSFILE"
fi
if [ -s "$tmp/calls" ]; then
	bad "wal-g ran against a database that refused the connection: $(cat "$tmp/calls")"
else
	ok "wal-g did not run"
fi
if [ -e "$tmp/stamp.noconn" ]; then
	bad "the success stamp was advanced by a pass that could not connect"
else
	ok "the success stamp was not advanced"
fi

head "one variable spelling across the toolchain"
# The original defect: this runner read PG_USER while wal-g, restore.sh and
# the env template read PGUSER. Grep is the cheap assertion that the private
# spelling is gone rather than merely shadowed.
if grep -Eq 'PG_USER([:}-]|=)' "$BACKUP_SH" "$ROOT/deploy/backup/drill.sh"; then
	bad "PG_USER is still read in the backup scripts: $(grep -El 'PG_USER([:}-]|=)' "$BACKUP_SH" "$ROOT/deploy/backup/drill.sh" | tr '\n' ' ')"
else
	ok "backupd.sh and drill.sh read PGUSER only"
fi
if grep -q '^PGUSER=' "$ENV_EXAMPLE"; then
	ok "wal-g.env.example names PGUSER"
else
	bad "wal-g.env.example does not name PGUSER"
fi
if grep -q 'REPLICATION' "$ENV_EXAMPLE"; then
	ok "the template documents the REPLICATION grant a streaming backup-push needs"
else
	bad "the template does not mention REPLICATION"
fi

printf '\n%d passed, %d failed\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ]
