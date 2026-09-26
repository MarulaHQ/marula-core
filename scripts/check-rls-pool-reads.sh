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
# pre-auth/dev surfaces below.
#
# Matching (tightened Wave 34 lane 6): an allowlist entry is
# `<file>:<anchor>` and matches only when the anchor appears in the
# STATEMENT CONTEXT of the `&state.pool` hit — the hit line plus the
# continued lines of the same statement, walked back to its opening `let`.
# Anchors name the SECURITY DEFINER bridge (or dev-tools call) the read
# rides, never just the `.fetch_*` shape, so a NEW pool-read elsewhere in
# the same file — even one written `.fetch_optional(&state.pool)` — fails
# immediately. Adding a surface to the allowlist requires the same
# justification as the entries below: no session exists yet, so identity
# can only resolve via a narrow SECURITY DEFINER bridge (migration-seam
# function) — never a tenant-table read.
#
# Non-purpose: this is a static grep lint — it does not prove the tx binding
# at runtime; RLS integration tests (core-db) own that.

set -euo pipefail

routes_dir="$(dirname "$0")/../crates/core-api/src/routes"

if [[ ! -d "$routes_dir" ]]; then
  echo "error: routes dir not found at $routes_dir" >&2
  exit 1
fi

# Statement context for a hit line: the hit line plus the continued lines
# of the same statement, walked backwards up to the statement opener (a
# line containing `let `), a `;` terminator, a blank line, or 6 lines.
# Returns the joined text so an anchor can match any line of the statement
# that raises the pool call.
statement_context() {
  local file="$1" hit_line="$2"
  local start="$hit_line"
  local back=1
  while [ "$back" -le 6 ] && [ $((hit_line - back)) -ge 1 ]; do
    local prev
    prev="$(sed -n "$((hit_line - back))p" "$file")"
    # Blank line = statement boundary.
    if [ -z "$(printf '%s' "$prev" | tr -d '[:space:]')" ]; then
      break
    fi
    # Terminated statement above = boundary.
    case "$prev" in
      *';') break ;;
    esac
    start=$((hit_line - back))
    # Statement opener (the `let …` the query is assigned to) = include and stop.
    case "$prev" in
      *'let '*) break ;;
    esac
    back=$((back + 1))
  done
  sed -n "${start},${hit_line}p" "$file" | tr '\n' ' '
}

# Deliberate raw-pool surfaces. Everything NOT listed here fails the gate.
# Each entry: <file>:<anchor> — anchored on the SECURITY DEFINER bridge /
# dev-tools call the read rides (matched against the hit's statement
# context), so a new pool-read in the same file still fails.
typeset -a allowlist
allowlist=(
  # Pre-auth alert ingest (webhook HMAC): the source_ref is public routing
  # info; identity resolves via the 0014 SECURITY DEFINER seam before any
  # session exists. All subsequent reads ride an org-bound tx.
  'alerts.rs:auth_lookup_source(&state.pool'
  # Pre-auth SSO (Wave 18 lane 2b + Wave 34 lane 6 re-anchor): provider /
  # code / public-list resolution via migration-seam SECURITY DEFINER
  # functions (no session yet); the callback then re-reads the provider
  # secret under an org-bound tx.
  'oidc.rs:app_list_enabled_oidc_providers()'
  'oidc.rs:resolve_provider_by_name(&state.pool'
  'oidc.rs:app_resolve_sso_code('
  # Pre-auth invite redemption: the 0047 SECURITY DEFINER bridge resolves
  # (org_id, user_id) from the token hash; the claim itself is org-bound.
  'org_users.rs:resolve_invite_by_token(&state.pool'
  # Pre-auth portal login/refresh: portal_resolve_login / portal_resolve_
  # session bridges (0021) return identity only; credential verification
  # and every read after ride set_org/set_org_and_user txs.
  'portal.rs:portal_resolve_login('
  'portal.rs:portal_resolve_session('
  # Self-host health probe (p36 selfhost-visibility): SELECT 1 liveness
  # only — no tenant column is read, nothing to scope.
  'self_host.rs:query_scalar::<_, i32>("SELECT 1")'
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
  # Pre-auth Cloudflare Access edge exchange (0072): the source
  # (org_id, issuer, audience, jwks_url) resolves through the
  # app_resolve_edge_source SECURITY DEFINER bridge before any session
  # exists; every read after the exchange rides an org-bound tx.
  'edge.rs:resolve_enabled_source(&state.pool'
  # Presence heartbeat cross-replica relay: publish_heartbeat_notify is
  # `SELECT pg_notify($1, $2)` — a channel notification, NOT a tenant
  # read; nothing here touches an RLS-gated table. Fire-and-forget after
  # the in-process upsert; failure only delays cross-replica convergence.
  'presence.rs:publish_heartbeat_notify(&state.pool'
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
  unscoped=0
  while IFS= read -r hit; do
    [[ -z "$hit" ]] && continue
    ln="${hit%%:*}"
    ctx="$(statement_context "$f" "$ln")"
    allowed=false
    for entry in "${allowlist[@]}"; do
      ef="${entry%%:*}"
      es="${entry#*:}"
      [[ "$base" == "$ef" ]] || continue
      case "$ctx" in
        *"$es"*)
          allowed=true
          break
          ;;
      esac
    done
    if [[ "$allowed" == false ]]; then
      if [[ "$unscoped" -eq 0 ]]; then
        echo "::error file=$base::unscoped pool-read(s) — every tenant read must ride an org-bound tx (set_org_and_user); pre-auth identity may only resolve via a SECURITY DEFINER bridge with an allowlist entry in check-rls-pool-reads.sh:"
      fi
      printf '%s\n' "$hit"
      unscoped=$((unscoped + 1))
    fi
  done <<< "$file_hits"
  if [[ "$unscoped" -gt 0 ]]; then
    rc=1
    residual_total=$((residual_total + unscoped))
  fi
done

if [[ "$rc" -eq 0 ]]; then
  echo "RLS pool-read gate: clean ($total allowlisted pre-auth/dev pool reads across routes/)."
else
  echo "RLS pool-read gate: FAILED ($residual_total unscoped pool-read(s))." >&2
fi
exit "$rc"
