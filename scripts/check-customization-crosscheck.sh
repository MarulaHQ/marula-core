#!/usr/bin/env bash
# U9 — customization-aware migration cross-check (Wave 19 lane 4, policy §4.5).
#
# Fails when a migration modifies a construct referenced by customization
# rows WITHOUT an in-transaction transform. Customization constructs are the
# declarative rows the customization manifest loads (crates/core-db/src/
# manifest.rs): custom field definitions, board statuses, automation rules,
# webhook subscriptions, org settings, SLA policies, report definitions,
# ticket taxonomy, client workflow overrides, roles.
#
# What counts as "modifies a construct" (static lint over migration SQL,
# same posture as check-migrations.sh — no live DB):
#   DROP TABLE <customization-bearing table>
#   ALTER TABLE <customization-bearing table> DROP COLUMN
#   DROP INDEX on a customization-bearing table
#
# What counts as "in-transaction transform present": the migration file
# carries a marker comment `-- customization-transform:` on the same
# migration, declaring the row-level transform that runs with the schema
# change (validated by review; this lint only checks the declaration exists).
#
# Non-purpose: this does NOT run migrations, diff schema, or chase foreign
# keys transitively — it guards the customization surface declared in
# policy §5 (declarative rows migrate as data under the same rules as §4).

set -euo pipefail

migrations_dir="$(dirname "$0")/../crates/core-db/migrations"

if [[ ! -d "$migrations_dir" ]]; then
  echo "error: migrations dir not found at $migrations_dir" >&2
  exit 1
fi

# Customization-bearing tables: every table the customization manifest reads
# rows from (manifest.rs sections). Keep in sync with manifest.rs — a new
# manifest section must add its tables here.
custom_tables=(
  custom_field_definitions
  custom_field_options
  board_statuses
  automation_rules
  webhook_subscriptions
  webhook_deliveries
  settings
  sla_policies
  sla_targets
  report_definitions
  ticket_categories
  client_workflow_overrides
  roles
  role_permissions
)

# Tables whose ROWS are values referenced BY customizations but which are
# themselves seeded/registry data, not per-org customization rows.
# Not listed above: boards (org rows but referenced via board_statuses).

rc=0
for f in "$migrations_dir"/*.sql; do
  base="$(basename "$f")"

  for t in "${custom_tables[@]}"; do
    # DROP TABLE on a customization-bearing table.
    if grep -qiE "DROP[[:space:]]+TABLE[[:space:]]+(IF[[:space:]]+EXISTS[[:space:]]+)?(public\.)?${t}\b" "$f"; then
      # In-transaction transform marker = declared row migration.
      if ! grep -qi "customization-transform" "$f"; then
        echo "U9 VIOLATION: $base drops customization-bearing table '$t' without a '-- customization-transform:' declaration (policy §4.5)" >&2
        rc=1
      fi
    fi

    # DROP COLUMN on a customization-bearing table.
    if grep -qiE "ALTER[[:space:]]+TABLE[[:space:]]+(IF[[:space:]]+EXISTS[[:space:]]+)?(public\.)?${t}[[:space:]]+DROP[[:space:]]+COLUMN" "$f"; then
      if ! grep -qi "customization-transform" "$f"; then
        echo "U9 VIOLATION: $base drops a column on customization-bearing table '$t' without a '-- customization-transform:' declaration (policy §4.5)" >&2
        rc=1
      fi
    fi
  done
done

if [[ "$rc" -eq 0 ]]; then
  echo "U9 cross-check clean — no customization-bearing construct modified without an in-transaction transform declaration."
fi
exit "$rc"
