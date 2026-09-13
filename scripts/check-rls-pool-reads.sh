#!/usr/bin/env zsh
# RLS pool-read gate (Wave 13 lane 3, TASKLIST RLS-scope audit item).
#
# The unscoped-GUC 500 class: a route handler that reads tenant rows straight
# off the shared PgPool (or off a tx that never ran `set_org_and_user`) fails
# under RLS with an internal error, because `app.org_id` is never set for the
# statement. Doctrine: EVERY tenant read must ride an org-bound tx —
# `state.pool.begin()` + `core_db::set_org_and_user(&mut tx, org_id, user_id)`
# (or the `org_tx::with_org_tx` helper) — never the bare pool.
#
# This gate greps `&state.pool` under crates/core-api/src/routes/ and fails
# review when a pool-read appears that is NOT one of the allowlisted
# pre-auth/dev surfaces below. Allowlist entries are content-anchored
# (file + call substring, rationale inline), not line-anchored, so edits
# around them don't wedge CI — but a NEW pool-read anywhere else fails
# immediately. Adding a surface to the allowlist requires the same
# justification: no session exists yet, so identity can only resolve via a
# narrow SECURITY DEFINER bridge (migration-seam function) — never a
# tenant-table read.
#
# Non-purpose: this is a static grep lint — it does not prove the tx binding
# at runtime; RLS integration tests (core-db) own that.

set -euo pipefail

routes_dir="$(dirname "$0")/../crates/core-api/src/routes"

if [[ ! -d "$routes_dir" ]]; then
  echo "error: routes dir not found at $routes_dir" >&2
  exit 1
fi

# Deliberate raw-pool surfaces. Everything NOT listed here fails the gate.
# Each entry: <file>:<call-substring> — anchored on the SECURITY DEFINER
# bridge / dev-tools call so a new pool-read in the same file still fails
# (only these exact surfaces are sanctioned).
typeset -a allowlist
allowlist=(
  # Pre-auth alert ingest (webhook HMAC): the source_ref is public routing
  # info; identity resolves via the 0014 SECURITY DEFINER seam before any
  # session exists. All subsequent reads ride an org-bound tx.
  'alerts.rs:auth_lookup_source(&state.pool'
  # Pre-auth SSO: provider/code resolution via migration-seam SECURITY
  # DEFINER functions (no session yet); the callback then re-reads the
  # provider secret under an org-bound tx.
  'oidc.rs:.fetch_all(&state.pool)'
  'oidc.rs:resolve_provider_by_name(&state.pool'
  'oidc.rs:.fetch_one(&state.pool)'
  # Pre-auth invite redemption: the 0047 SECURITY DEFINER bridge resolves
  # (org_id, user_id) from the token hash; the claim itself is org-bound.
  'org_users.rs:resolve_invite_by_token(&state.pool'
  # Pre-auth portal login/refresh: portal_resolve_login / portal_resolve_
  # session bridges (0021) return identity only; credential verification
  # and every read after ride set_org/set_org_and_user txs.
  'portal.rs:.fetch_optional(&state.pool)'
  # Dev-tools surface (DX2): mounted ONLY when MERIDIAN_DEV_TOOLS=1, demo
  # org only, and every core_db::demo call self-scopes via
  # set_org(DEMO_ORG_ID) inside its transaction. The best-effort system
  # audit row is a write-through (Actor::System, DEMO_ORG_ID), not a
  # tenant read.
  'dev.rs:core_db::audit::insert(&state.pool'
  'dev.rs:core_db::demo::is_seeded(&state.pool'
  'dev.rs:core_db::demo::reset(&state.pool'
  'dev.rs:core_db::demo::seed(&state.pool'
  'dev.rs:core_db::demo::time_travel(&state.pool'
  'dev.rs:core_db::demo::recent_events(&state.pool'
)

rc=0
total=0
residual_total=0
for f in "$routes_dir"/*.rs; do
  base="$(basename "$f")"
  file_hits=$(grep -nF '&state.pool' "$f" || true)
  [[ -z "$file_hits" ]] && continue
  count=$(printf '%s\n' "$file_hits" | wc -l | tr -d ' ')
  total=$((total + count))
  residual="$file_hits"
  for entry in $allowlist; do
    ef="${entry%%:*}"
    es="${entry#*:}"
    if [[ "$base" == "$ef" ]]; then
      residual="$(printf '%s\n' "$residual" | grep -vF -- "$es" || true)"
    fi
  done
  if [[ -n "${residual//$'\n'/}" ]]; then
    echo "::error file=$base::unscoped pool-read(s) — every tenant read must ride an org-bound tx (set_org_and_user); pre-auth identity may only resolve via a SECURITY DEFINER bridge with an allowlist entry in check-rls-pool-reads.sh:"
    printf '%s\n' "$residual"
    rc=1
    residual_total=$((residual_total + $(printf '%s\n' "$residual" | wc -l | tr -d ' ')))
  fi
done

if [[ "$rc" -eq 0 ]]; then
  echo "RLS pool-read gate: clean ($total allowlisted pre-auth/dev pool reads across routes/)."
else
  echo "RLS pool-read gate: FAILED ($residual_total unscoped pool-read(s))." >&2
fi
exit "$rc"
