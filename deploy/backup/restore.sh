#!/bin/sh
# Marula restore rehearsal — scratch-restore + verify. Run manually:
#   sh deploy/backup/restore.sh <path to .dump>
#
# Requires: psql/pg_restore reachable and a running PG. The connecting user
# (PGUSER, default `marula`) must be able to create roles and databases — it is
# the restore's admin, and the only identity that keeps authority past this run.
# Nothing in the script prompts: give the admin a PGPASSWORD or a PGPASSFILE.
# Creates the scratch DB `marula_restore_test`, restores into it, then asserts
# the restore is real:
#   1. schema landed (tables restored, RLS enforced on every public table);
#   2. round-trip row parity: when the source DB is reachable (PGDATABASE
#      set, e.g. `PGDATABASE=marula` against the live stack), row counts
#      for every public table must match the dump's origin — a dump that
#      loses rows in transit is not restorable cleanly. Override
#      PGSOURCE_HOST/PGSOURCE_PORT when the source is a different server.
#      Set RESTORE_PARITY=none to skip it: an automated drill restores a dump
#      taken minutes ago against a live database, where exact counts are
#      EXPECTED to differ and the comparison would fail every time.
#
# Roles: this script never gives the application role BYPASSRLS. The restore
# writes every org's rows, so it needs to look past row level security, and that
# authority belongs to an identity that exists for this run and no longer:
#
#   * `marula_restore_<pid>` — LOGIN, BYPASSRLS, a password generated here, and
#     the owner of the scratch database. It is not a member of the admin role
#     and cannot create roles or databases: everything it can do is create
#     objects in the one database this run made, and write rows without a policy
#     in the way. That database and this role are dropped in an EXIT trap, so
#     both go away on success, on failure and on a signal.
#   * `marula_app` / `meridian_app` — the roles the dump's ACLs name (0114 kept
#     the pre-rebrand role's grants, so both appear). When absent they are
#     created NOLOGIN, NOBYPASSRLS: the GRANTs resolve and nobody can log in as
#     them on this server. When one already exists holding SUPERUSER or
#     BYPASSRLS the rehearsal stops instead of altering a live role — an app role
#     that can bypass RLS is a tenancy defect, and a rehearsal is not where it
#     gets fixed silently.
#
# Two statement classes in the dump belong to the cluster administrator, not to
# the restore: extensions and their comments (the admin installed them, so it
# owns them — `vector` and friends are not trusted extensions, so only a
# superuser can create them) and `ALTER DEFAULT PRIVILEGES` (rules for a role the
# restore is not a member of). The admin creates the extensions before the
# restore so the schema can use their types, and replays the rest in a second
# pass afterwards. That keeps the restore role free of the admin's authority
# instead of handing it a membership it would only need for seven statements.
#
# Drops the scratch DB when done; RESTORE_KEEP=1 leaves it in place so a caller
# (deploy/backup/drill.sh) can assert against the restored rows before dropping
# it — its objects are handed back to the admin role so the restore role can
# still be dropped. A backup that has never restored is not a backup — rehearse
# this.
set -eu
DUMP="${1:?usage: restore.sh <path to .dump>}"
[ -f "$DUMP" ] || { echo "no such dump: $DUMP" >&2; exit 1; }
PGHOST="${PGHOST:-localhost}"
PGPORT="${PGPORT:-15432}"
ADMIN_ROLE="${PGUSER:-marula}"
SCRATCH_DB="marula_restore_test"
# The roles the dump's ACLs name. Overridable for a dump from an older schema.
APP_ROLES="${RESTORE_APP_ROLES:-marula_app meridian_app}"
# Per-run identity: the PID keeps two concurrent rehearsals apart on one host.
RESTORE_ROLE="marula_restore_$$"
RESTORE_PW="$(od -An -tx1 -N 16 /dev/urandom | tr -d ' \n')"
KEEP="${RESTORE_KEEP:-0}"

ROLE_CREATED=0
SCRATCH_CREATED=0
RESTORE_OK=0
RESTORE_TOC=""
ADMIN_TOC=""

log() { echo "[restore $(date -u +%FT%TZ)] $*"; }
fail() { log "FAIL: $*"; exit 1; }

# The admin connection: whatever credentials the caller's environment carries
# (PGPASSWORD, PGPASSFILE, a trust rule). Never re-exported, never logged.
psql_admin() {
  _db="$1"
  shift
  psql -h "$PGHOST" -p "$PGPORT" -U "$ADMIN_ROLE" -d "$_db" -tA \
    -v ON_ERROR_STOP=1 "$@"
}

# The restore identity: the only connection in this script that may write rows
# into every org's tables. Its password lives in this process and dies with it.
psql_restore() {
  PGPASSWORD="$RESTORE_PW" psql -h "$PGHOST" -p "$PGPORT" -U "$RESTORE_ROLE" \
    -d "$SCRATCH_DB" -tA -v ON_ERROR_STOP=1 "$@"
}

# Everything this script creates is torn down here: success, `fail`, a signal,
# or a shell that dies under `set -e`. That is the whole point of the dedicated
# role — there is no path on which a BYPASSRLS identity or a restored copy of
# the database is left behind.
cleanup() {
  trap - EXIT INT TERM HUP
  [ -z "$RESTORE_TOC" ] || rm -f "$RESTORE_TOC"
  [ -z "$ADMIN_TOC" ] || rm -f "$ADMIN_TOC"
  if [ "$SCRATCH_CREATED" = "1" ]; then
    if [ "$KEEP" = "1" ] && [ "$RESTORE_OK" = "1" ]; then
      # The caller asserts against these rows, so the database outlives the
      # script. Its objects must not outlive the role that made them, or DROP
      # ROLE cannot run: hand both the objects and the database to the admin.
      # A failed run hands back nothing and is dropped like any other: half a
      # restore is not an artifact worth keeping.
      if psql_admin "$SCRATCH_DB" \
        -c "REASSIGN OWNED BY \"$RESTORE_ROLE\" TO \"$ADMIN_ROLE\";" >/dev/null 2>&1; then
        # Only after the handback: DROP OWNED would drop anything still owned,
        # and a half-moved database is worth less than a loud warning.
        psql_admin "$SCRATCH_DB" \
          -c "DROP OWNED BY \"$RESTORE_ROLE\";" >/dev/null 2>&1 \
          || log "WARNING: could not revoke privileges held by $RESTORE_ROLE"
        psql_admin postgres \
          -c "ALTER DATABASE $SCRATCH_DB OWNER TO \"$ADMIN_ROLE\";" >/dev/null 2>&1 \
          || log "WARNING: could not hand $SCRATCH_DB back to $ADMIN_ROLE"
      else
        log "WARNING: could not reassign objects owned by $RESTORE_ROLE"
      fi
    else
      psql_admin postgres -c "DROP DATABASE IF EXISTS $SCRATCH_DB WITH (FORCE);" >/dev/null 2>&1 \
        || psql_admin postgres -c "DROP DATABASE IF EXISTS $SCRATCH_DB;" >/dev/null 2>&1 \
        || log "WARNING: could not drop $SCRATCH_DB"
      log "scratch dropped ($SCRATCH_DB)"
    fi
  fi
  if [ "$ROLE_CREATED" = "1" ]; then
    if psql_admin postgres -c "DROP ROLE IF EXISTS \"$RESTORE_ROLE\";" >/dev/null 2>&1; then
      log "restore role dropped ($RESTORE_ROLE)"
    else
      # Fail-safe, not best-effort: if the role cannot be dropped, it must not
      # keep the attribute or a usable login. The next run creates a fresh one
      # under its own PID.
      psql_admin postgres \
        -c "ALTER ROLE \"$RESTORE_ROLE\" NOBYPASSRLS NOLOGIN;" \
        >/dev/null 2>&1 || log "WARNING: could not strip $RESTORE_ROLE after a failed drop"
      log "WARNING: could not drop $RESTORE_ROLE — attribute revoked, login disabled"
    fi
  fi
}
trap cleanup EXIT
# A signal must leave through the same door: cleanup runs from the EXIT trap,
# and the shell exits with the conventional status instead of carrying on to a
# verification leg whose role no longer exists.
trap 'log "interrupted — tearing the run down"; exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

log "preflight: $DUMP"
command -v psql >/dev/null 2>&1 || fail "psql not on PATH"
command -v pg_restore >/dev/null 2>&1 || fail "pg_restore not on PATH"
# Read the archive's table of contents before creating anything. A truncated or
# overwritten dump should never get as far as a role and a database.
TOC=$(pg_restore --list "$DUMP" 2>/dev/null) || fail "pg_restore cannot read $DUMP — the archive is corrupt"
psql_admin postgres -c "SELECT 1;" >/dev/null 2>&1 \
  || fail "admin connection failed ($ADMIN_ROLE@$PGHOST:$PGPORT)"

# Extensions are created on the source by migrations and replayed by the dump,
# and the ones that matter here (vector, btree_gist, pg_stat_statements) are not
# trusted: a non-superuser cannot install them. Create them as the admin first;
# pg_restore's own `CREATE EXTENSION IF NOT EXISTS` then skips them.
EXTENSIONS=$(printf '%s\n' "$TOC" | awk '$4 == "EXTENSION" && $5 == "-" {print $6}')

log "roles: app roles named by the dump are kept powerless"
for _role in $APP_ROLES; do
  _attrs=$(psql_admin postgres -c \
    "SELECT rolsuper::int::text || rolbypassrls::int::text FROM pg_roles WHERE rolname = '$_role';")
  case "$_attrs" in
    "") log "  $_role absent — creating NOLOGIN, NOBYPASSRLS"
        psql_admin postgres -c \
          "CREATE ROLE \"$_role\" NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;" \
          >/dev/null || fail "could not create the app role $_role"
        ;;
    "00") log "  $_role present, no elevated attribute"
        ;;
    *) fail "app role $_role in $PGHOST:$PGPORT holds SUPERUSER or BYPASSRLS (super/bypass=$_attrs) — RLS is the tenancy boundary, so this rehearsal refuses to run against it. Revoke the attribute, then rehearse."
        ;;
  esac
done

log "roles: create $RESTORE_ROLE (BYPASSRLS, this run only)"
psql_admin postgres <<SQL >/dev/null || fail "could not create the restore role $RESTORE_ROLE"
CREATE ROLE "$RESTORE_ROLE" LOGIN PASSWORD '$RESTORE_PW' BYPASSRLS NOINHERIT
  NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION;
SQL
ROLE_CREATED=1

log "scratch DB create (owner $RESTORE_ROLE)"
psql_admin postgres -c "DROP DATABASE IF EXISTS $SCRATCH_DB WITH (FORCE);" >/dev/null \
  || fail "could not clear a previous $SCRATCH_DB"
psql_admin postgres -c "CREATE DATABASE $SCRATCH_DB OWNER \"$RESTORE_ROLE\";" >/dev/null \
  || fail "could not create $SCRATCH_DB"
SCRATCH_CREATED=1

for _ext in $EXTENSIONS; do
  psql_admin "$SCRATCH_DB" -c "CREATE EXTENSION IF NOT EXISTS \"$_ext\";" >/dev/null 2>&1 \
    || fail "could not create extension $_ext in $SCRATCH_DB"
done

# Split the archive's table of contents: everything the restore role may run,
# and the entries only the admin may run. `-L` makes pg_restore execute exactly
# the listed entries, so the restore role never sees a statement that needs
# authority it does not have. A future dump that grows a new admin-only
# statement fails this run loudly rather than quietly widening the role — the
# fix is to add its entry type to this one pattern.
ADMIN_ONLY_TOC_ENTRIES='(EXTENSION|COMMENT - EXTENSION|DEFAULT ACL)'
RESTORE_TOC=$(mktemp /tmp/.restore-role-toc.XXXXXX)
ADMIN_TOC=$(mktemp /tmp/.restore-admin-toc.XXXXXX)
printf '%s\n' "$TOC" | grep -v -E "^[0-9]+; [0-9]+ [0-9]+ $ADMIN_ONLY_TOC_ENTRIES " \
  >"$RESTORE_TOC" || :
printf '%s\n' "$TOC" | grep -E "^[0-9]+; [0-9]+ [0-9]+ $ADMIN_ONLY_TOC_ENTRIES " \
  >"$ADMIN_TOC" || :

log "pg_restore into scratch, as $RESTORE_ROLE"
PGPASSWORD="$RESTORE_PW" pg_restore -h "$PGHOST" -p "$PGPORT" -U "$RESTORE_ROLE" \
  -d "$SCRATCH_DB" --no-owner --clean --if-exists -L "$RESTORE_TOC" "$DUMP" \
  || fail "pg_restore reported errors on $DUMP — the rehearsal is not green"

if grep -qv '^;' "$ADMIN_TOC"; then
  _admin_entries=$(grep -c -v '^;' "$ADMIN_TOC")
  log "pg_restore the $_admin_entries administrator statements, as $ADMIN_ROLE"
  # No --clean here: these entries add comments and default-privilege rules to a
  # database that already holds the restore, and must not drop anything.
  pg_restore -h "$PGHOST" -p "$PGPORT" -U "$ADMIN_ROLE" \
    -d "$SCRATCH_DB" --no-owner -L "$ADMIN_TOC" "$DUMP" \
    || fail "pg_restore could not replay the administrator statements in $DUMP"
fi
rm -f "$RESTORE_TOC" "$ADMIN_TOC"

log "verify: schema landed (table count)"
TABLES=$(psql_restore -c "SELECT count(*) FROM pg_tables WHERE schemaname='public';")
[ "${TABLES:-0}" -gt 0 ] 2>/dev/null || fail "scratch DB has $TABLES tables in public schema"
log "  $TABLES tables in public schema"

log "verify: RLS enforced on every public table"
# Everything org-scoped carries RLS; four server-wide registry tables are
# deliberately sans-RLS (no org_id column by design, see
# 0076_plugin_v2.sql, 0095_wave19_supplychain_identity.sql), and
# `org_erasure_receipts` is a platform record whose tenant column is
# `purged_org_id` — the org it names no longer exists (0146, see
# docs/reference/data-lifecycle.md §3). This allowlist must match the migration
# suite exactly — count rows should not be edited.
#
# Four more are not tenant data and cannot carry a policy: `_sqlx_migrations` is
# the migrator's own ledger, `event_types` is the global catalogue of what event
# types exist rather than who may subscribe (0124: "carries NO RLS policy by
# design"; the per-org permit is `event_type_grants`), `embedding_space_state`
# is the instance's single space row (0152), and `workflow_node_catalogue` pins a
# release and carries no node rows (0128). Without them the check failed on every
# current dump — masked until now by the pg_restore leg failing first.
NO_RLS="plugin_catalog|plugin_versions|plugin_signing_keys|ai_decision_question_sets|org_erasure_receipts|_sqlx_migrations|event_types|embedding_space_state|workflow_node_catalogue"
RLS_OFF=$(psql_restore -c \
  "SELECT c.relname FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
   WHERE n.nspname = 'public' AND c.relkind = 'r' AND c.relrowsecurity = false
     AND c.relname !~ '^($NO_RLS)$';")
[ -z "${RLS_OFF:-}" ] || fail "public tables lack RLS: $RLS_OFF — restore is partial"
log "  RLS on for every public table outside the allowlist ($TABLES public tables)"

# The reads below cross every org, which is exactly what the restore role's
# BYPASSRLS buys. Running them as anyone else would prove less: a plain app role
# sees zero rows here and reports a mismatch on every table.
log "verify: identity $(psql_restore -c "SELECT current_user || ' bypassrls=' ||
  (SELECT rolbypassrls FROM pg_roles WHERE rolname = current_user) || ' superuser=' ||
  (SELECT rolsuper FROM pg_roles WHERE rolname = current_user);")"

if [ -n "${PGDATABASE:-}" ] && [ "${RESTORE_PARITY:-exact}" != "none" ]; then
  # Parity target: the DB the dump was taken from. Defaults to the same
  # server as the restore target; point PGSOURCE_HOST/PGSOURCE_PORT at the
  # live stack when rehearsing into a separate scratch server.
  SRC_HOST="${PGSOURCE_HOST:-$PGHOST}"
  SRC_PORT="${PGSOURCE_PORT:-$PGPORT}"
  log "verify: round-trip row parity vs source DB '$PGDATABASE' via $SRC_HOST:$SRC_PORT"
  MISMATCHES=$(mktemp /tmp/.restore-parity.XXXXXX)
  psql_restore -c \
    "SELECT tablename FROM pg_tables WHERE schemaname='public' ORDER BY tablename;" |
  while IFS= read -r t; do
    a=$(psql_restore -c "SELECT count(*) FROM \"$t\";" || echo '?')
    b=$(psql -h "$SRC_HOST" -p "$SRC_PORT" -U "$ADMIN_ROLE" -d "$PGDATABASE" \
      -tA -c "SELECT count(*) FROM \"$t\";" || echo '?')
    if [ "$a" != "$b" ]; then
      echo "$t: scratch=$a source=$b" >>"$MISMATCHES"
    fi
  done
  if [ -s "$MISMATCHES" ]; then
    cat "$MISMATCHES" >&2
    fail "row counts differ between scratch and source"
  fi
  rm -f "$MISMATCHES"
  log "  row parity OK for all $TABLES tables"
elif [ -n "${PGDATABASE:-}" ]; then
  log "verify: round-trip skipped (RESTORE_PARITY=none — the caller asserts on the restored rows)"
else
  log "verify: round-trip skipped (PGDATABASE not set — point it at the source stack to assert row parity)"
fi

if [ "$KEEP" = "1" ]; then
  log "scratch kept ($SCRATCH_DB) — the caller drops it after its own assertions"
fi
RESTORE_OK=1
log "scratch restore OK — schema complete, RLS enforced"
