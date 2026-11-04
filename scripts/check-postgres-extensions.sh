#!/usr/bin/env bash
# Assert the extension image actually carries — and USES — pg_cron, hypopg and
# vchord.
#
# Why: `deploy/postgres/Dockerfile` is the only image with the three, and the
# product degrades cleanly without them (migration 0145 reports notices, 0153
# returns `skipped: …`, retrieval falls back). That degradation is correct and
# it is also why the overlay was never proven by a gate: a stack booted on the
# stock `pgvector/pgvector:pg18` looks identical from the outside.
#
# So this runs against a database the stack has actually migrated — the
# extensions are asserted PRESENT (pg_extension) and USED (a scheduled pg_cron
# job, a hypothetical index the planner accepts, the vchord ANN index the boot's
# retrieval probe refuses to start without).
#
# Usage:
#   scripts/check-postgres-extensions.sh --container <name> --db <database>
#
# The gate path is a stack boot on the overlay image, which calls this itself:
#   E2E_DB_IMAGE=extensions scripts/e2e-stack.sh up
#
# Exits non-zero on the first missing or unused extension, naming which.
set -euo pipefail

CONTAINER=""
DB=""
DOCKER="${DOCKER:-docker}"

while [ $# -gt 0 ]; do
	case "$1" in
		--container) CONTAINER="${2:-}"; shift 2 ;;
		--db)        DB="${2:-}"; shift 2 ;;
		-h|--help)   sed -n '2,20p' "$0"; exit 0 ;;
		*) echo "unknown argument: $1" >&2; exit 2 ;;
	esac
done

[ -n "$CONTAINER" ] && [ -n "$DB" ] || {
	echo "usage: $0 --container <name> --db <database>" >&2
	exit 2
}

FAIL=0
ok()  { printf '  ok   %s\n' "$*"; }
bad() { FAIL=$((FAIL + 1)); printf '  FAIL %s\n' "$*"; }

# One value, one row. `-X` keeps a stray ~/.psqlrc out of the answer and `-q`
# keeps the notices migration 0145 emits from being read as data.
q() {
	"$DOCKER" exec -e PGOPTIONS='-c client_min_messages=warning' "$CONTAINER" \
		psql -X -q -t -A -U postgres -d "$DB" -c "$1" 2>/dev/null || true
}

echo "== extension image on $CONTAINER/$DB"

missing=""
for ext in pg_cron hypopg vchord; do
	version="$(q "SELECT extversion FROM pg_extension WHERE extname = '$ext'")"
	if [ -n "$version" ]; then
		ok "$ext installed ($version)"
	else
		bad "$ext is not in pg_extension — image, preload or migration did not take it"
		missing="$missing $ext"
	fi
done

# A missing extension makes the usage probes below meaningless; say so once and
# stop rather than emitting a cascade of confusing failures.
if [ -n "$missing" ]; then
	echo "  FAIL the image did not provide:$missing" >&2
	exit 1
fi

# --- used, not merely installed ------------------------------------------------

signals="$(q "SELECT string_agg(signal || '=' || installed || '/' || active, ' ' ORDER BY signal) FROM ops.db_signals WHERE signal IN ('pg_cron','hypopg')")"
case "$signals" in
	*"pg_cron=true/true"*) ok "ops.db_signals reports pg_cron installed and active" ;;
	*) bad "ops.db_signals reports pg_cron as '$signals' (want installed/active)" ;;
esac
case "$signals" in
	*"hypopg=true/true"*) ok "ops.db_signals reports hypopg installed and active" ;;
	*) bad "ops.db_signals reports hypopg as '$signals' (want installed/active)" ;;
esac

jobs="$(q "SELECT count(*) FROM cron.job WHERE jobname LIKE 'marula-%'")"
if [ "${jobs:-0}" -gt 0 ] 2>/dev/null; then
	ok "pg_cron carries $jobs scheduled marula maintenance job(s)"
else
	bad "cron.job holds no marula-* job: migration 0145 ran without a working scheduler"
fi

# A hypothetical index is the extension's whole purpose: the planner has to
# accept it and list it for the session. Created and dropped in one statement
# group so nothing is persisted.
hypo="$(q "SELECT hypopg_create_index('CREATE INDEX ON pg_class (relname)'); SELECT count(*) FROM hypopg_list_indexes; SELECT hypopg_reset();" | sed -n '2p')"
if [ "${hypo:-0}" -gt 0 ] 2>/dev/null; then
	ok "hypopg accepts and lists a hypothetical index, then resets"
else
	bad "hypopg did not list the hypothetical index it was asked for"
fi

# vchord's index is what `MARULA_RETRIEVAL_MODE=index` refuses to boot without.
# The e2e stack exports that mode when it boots this image, so its absence here
# is a real failure, not a configuration choice: the overlay exists to make that
# mode satisfiable, and the boot's own probe is the proof it is used.
preload="$(q "SELECT setting FROM pg_settings WHERE name = 'shared_preload_libraries'")"
case "$preload" in
	*vchord*) ok "vchord is in shared_preload_libraries" ;;
	*) bad "vchord is installed but not preloaded ($preload) — its index cannot be maintained" ;;
esac
am="$(q "SELECT am.amname FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid JOIN pg_am am ON am.oid = c.relam WHERE c.relname = 'search_embeddings_vchord'")"
if [ "$am" = "vchordrq" ]; then
	ok "the retrieval index is built with vchordrq"
else
	bad "search_embeddings_vchord is missing or uses '${am:-nothing}' — MARULA_RETRIEVAL_MODE=index was not the migrate mode"
fi

if [ "$FAIL" -eq 0 ]; then
	echo "  ok   the overlay image is present and used"
	exit 0
fi
echo "  $FAIL check(s) failed" >&2
exit 1
