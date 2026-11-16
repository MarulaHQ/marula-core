#!/usr/bin/env bash
# Migration safety gate (BUILD-PROMPT §CI): destructive schema changes are
# forward-only but must be flagged in the filename with `--breaking` and
# documented, so operators can't upgrade past a data-losing step unnoticed.
# (bash, not zsh: the CI runner guarantees bash, this file is CI-portable.)
#
# What counts as destructive:
#   DROP TABLE / DATABASE / SCHEMA / INDEX / CONSTRAINT / POLICY
#   ALTER TABLE ... DROP COLUMN
#   ALTER COLUMN ... TYPE (may rewrite/reject data)
#   TRUNCATE
#   UPDATE/DELETE touching a whole table without WHERE
#
# Non-purpose: this is a static lint over migration SQL — it does not run
# migrations or diff against the live database.

set -euo pipefail

migrations_dir="$(dirname "$0")/../crates/core-db/migrations"

if [[ ! -d "$migrations_dir" ]]; then
  echo "error: migrations dir not found at $migrations_dir" >&2
  exit 1
fi

rc=0
# Only scan files NOT already flagged as breaking.
for f in "$migrations_dir"/*.sql; do
  base="$(basename "$f")"
  if [[ "$base" == *--breaking* ]]; then
    continue
  fi

  hits=$(grep -inE \
    '^\s*(DROP\s+(TABLE|DATABASE|SCHEMA|INDEX|CONSTRAINT|POLICY)|TRUNCATE)|DROP\s+COLUMN|ALTER\s+COLUMN\s+\S+\s+TYPE' \
    "$f" | grep -vE '^[0-9]+:[[:space:]]*--' || true)

  # Comment lines are not statements: prose may name DROP safely.

  # Unfiltered UPDATE/DELETE (whole-table data loss): statement starts a line
  # and carries no WHERE on that line. Keyed off the statement keyword so
  # `ON DELETE CASCADE;` fragments inside ADD COLUMN lines never match.
  unfiltered=$(grep -inE \
    '^\s*(DELETE\s+FROM\s+\S+\s*;|UPDATE\s+\S+\s+SET\b[^;]*;)' \
    "$f" | grep -ivE 'where' | grep -vE '^[0-9]+:[[:space:]]*--' || true)
  hits="${hits}${unfiltered}"

  # Deliberate whole-table backfills in ALREADY-SHIPPED migrations. Shipped
  # migration files are checksummed by sqlx — editing them (to add --breaking
  # or annotations) would break `sqlx migrate run` on every environment that
  # already applied them — so each intentional unfiltered statement is
  # whitelisted here with its rationale. Everything not listed stays strict.
  typeset -a migration_backfill_allowlist
  migration_backfill_allowlist=(
    # 0013: seeds refresh-token rotation chains — each existing session
    # becomes its own chain root (see the comment above the statement);
    # no data is lost.
    '0013_auth_hardening.sql:UPDATE auth_sessions SET chain_id = id;'

    # 0112/0113: widening invoice_lines' kind vocabulary. PG CHECK constraints
    # have no ON-UPDATE alter, so the kind check and the two unique indexes
    # are dropped and recreated over the widened value list; rows untouched.
    '0112_wave39_contract_types.sql:DROP INDEX IF EXISTS uq_invoice_lines_period;'
    '0112_wave39_contract_types.sql:DROP INDEX IF EXISTS uq_invoice_lines_fixed_fee;'
    '0113_wave39_project_billing.sql:DROP INDEX IF EXISTS uq_invoice_lines_fixed_fee;'

    # 0116/0120: same drop-and-re-add of a named CHECK constraint when its
    # value list widens (calendar providers; time-entry actions). No rows.
    '0116_wave34_claims_fixes.sql:DROP CONSTRAINT IF EXISTS calendar_sync_connections_provider_check;'
    '0120_wave41_time_entry_fixed_action.sql:DROP CONSTRAINT IF EXISTS time_entry_approvals_action_check;'

    # 0126: same drop-and-re-add when the plugin surface vocabulary gains the
    # two executor kinds (automation_trigger, read) a connector may contribute.
    # Every existing value stays admitted; no rows touched.
    '0126_plugin_outbound_surface_kinds.sql:DROP CONSTRAINT IF EXISTS plugin_surface_entries_surface_check;'

    # 0224: narrowing contract_clauses_read from an implicit ALL policy to
    # SELECT. PG has no ALTER POLICY for the command, so the policy is dropped
    # and recreated in the same file; it discards a rule, never rows.
    '0224_contract_clauses_read_select.sql:DROP POLICY IF EXISTS contract_clauses_read ON contract_clauses;'
  )
  filtered="$hits"
  for entry in "${migration_backfill_allowlist[@]}"; do
    stmt="${entry#*:}"
    filtered="$(printf '%s\n' "$filtered" | grep -vF -- "$stmt" || true)"
  done
  hits="$filtered"

  if [[ -n "$hits" ]]; then
    echo "::error file=$base::destructive statement(s) without --breaking flag:"
    echo "$hits"
    rc=1
  fi
done

# Filename shape check: NNN_name.sql keeps history sortable and greppable.
for f in "$migrations_dir"/*.sql; do
  base="$(basename "$f")"
  if ! [[ "$base" =~ ^[0-9]{4}_[a-z0-9_]+(--breaking)?\.sql$ ]]; then
    echo "::error file=$base::migration filename must match NNN_name.sql"
    rc=1
  fi
done

# Shipped migrations are immutable. sqlx digests the whole file, comments
# included, so editing one that already shipped makes every existing
# installation's next `migrate` fail with "previously applied but has been
# modified". A prose sweep did exactly that on 2026-10-21 (53 files, comment
# lines only), and the upgrade matrix found it weeks later. The newest upgrade
# fixture records the digest each version was captured with, so it is the
# reference for every version it holds.
fixture="$(dirname "$0")/../fixtures/upgrades/v0.3.0.seed.sql.gz"
if [[ -f "$fixture" ]]; then
  # A heredoc inside `$( )` is parsed differently by bash 3.2 (the macOS
  # /bin/bash): the body's quotes are not literal there and the substitution
  # never closes. The script goes in a temp file instead, which every bash
  # parses the same way.
  certify_py="$(mktemp "${TMPDIR:-/tmp}/check-migrations.XXXXXX")"
  trap 'rm -f "$certify_py"' EXIT
  cat > "$certify_py" <<'PYEOF'
import gzip, hashlib, os, re, sys

fixture, migrations_dir = sys.argv[1], sys.argv[2]

# The crates' own exemption manifest: the same file `run_migrations` reads, so
# an allowed rewrite is named in one place and argued for there.
allowed = set()
manifest = os.path.join(migrations_dir, "APPLIED_REWRITES")
if os.path.exists(manifest):
    with open(manifest) as handle:
        for line in handle:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            allowed.add(int(line.split()[0]))

recorded = {}
started = False
with gzip.open(fixture, "rt") as handle:
    for line in handle:
        if line.startswith("COPY public._sqlx_migrations"):
            started = True
            continue
        if not started:
            continue
        if line.startswith("\\."):
            break
        parts = line.rstrip("\n").split("\t")
        if len(parts) > 4 and parts[0].isdigit():
            recorded[int(parts[0])] = parts[4].replace("\\x", "").lstrip("\\")

files = {}
for name in os.listdir(migrations_dir):
    match = re.match(r"^(\d+)_", name)
    if match and name.endswith(".sql"):
        files[int(match.group(1))] = os.path.join(migrations_dir, name)

for version, want in sorted(recorded.items()):
    if version in allowed:
        continue
    path = files.get(version)
    if path is None:
        print(f"{version}: recorded in the fixture, missing from this tree")
        continue
    with open(path, "rb") as handle:
        got = hashlib.sha384(handle.read()).hexdigest()
    if got != want:
        print(f"{version}: {os.path.basename(path)}")
PYEOF
  drift="$(python3 "$certify_py" "$fixture" "$migrations_dir")"
  rm -f "$certify_py"
  trap - EXIT
  if [[ -n "$drift" ]]; then
    echo "::error::shipped migration(s) changed since the v0.3.0 capture — sqlx refuses to migrate an existing install:" >&2
    printf '%s\n' "$drift" >&2
    rc=1
  fi
fi

if [[ "$rc" -eq 0 ]]; then
  echo "Migration safety: clean ($(ls "$migrations_dir"/*.sql | wc -l | tr -d ' ') migrations scanned)."
else
  echo "Migration safety: FAILED — read the lines above: an unflagged destructive statement, a filename that does not match NNN_name.sql, or a shipped migration edited after its release." >&2
fi
exit "$rc"
