# E2E: Event-Type Registry (Wave 43, Lane 2)

## Goal

Replace the hardcoded `KNOWN_EVENT_TYPES` allowlist (14 entries in
`webhook_admin.rs`) with a DB-backed registry that the platform,
first-party features, and plugins contribute to. Keep an
explicit-validation posture: an unknown event type on subscription
create/update is rejected (422) unless the org holds a grant for it.

## Implementation

### Migration: `0124_event_type_registry.sql`

Two tables:

**`event_types`** — global catalogue of known event types. No `org_id`
column, no RLS (it defines WHAT types exist, not WHO may use them).
- `event_type` (text PK) — dot-path, e.g. `ticket.created`.
- `source` — `'platform'` | `'plugin'` | `'external'`.
- `label`, `description` — human-facing.
- `plugin_install_id` — FK to `plugin_installs` (nullable; NULL for
  platform/seed types).

**`event_type_grants`** — deny-by-default org-level permit.
- `(org_id, event_type)` composite UNIQUE key.
- `granted_by` — FK to `users(id)`, `ON DELETE SET NULL` (nullable so
  bootstrap/dev-reset/plugin-sync grants without a human actor persist).
- RLS policy: `USING (org_id = current_setting('app.org_id', true)::uuid)`
  (mirrors `mcp_capability_grants` from migration 0040).

**Seed data:** the original 14 platform types from `KNOWN_EVENT_TYPES`,
plus `alert.ingested` and `webhook.received` (the lane-1 emitted types
that were missing from the old const).

### Rust module: `crates/core-db/src/event_types.rs`

Runtime `sqlx::query` / `sqlx::query_scalar` (no `!` compile-time macros).
- `check_event_types(pool, org_id, &[String]) -> DbResult<Vec<String>>`
  — returns the types that fail validation (not platform, not granted).
  Opens a short transaction, sets `app.org_id`, runs a single `UNNEST`
  query, commits.
- `insert_event_type_grant(tx, org_id, &str, Option<Uuid>)`
  — used by plugin install sync and tests.
- `list_event_type_grants(tx, org_id) -> Vec<String>`
- `delete_event_type_grant(tx, org_id, &str)`
- `is_event_type_known(pool, &str) -> bool`
- `find_unknown_types(pool, &[String]) -> Vec<String>`

### Route refactor: `webhook_admin.rs`

- `KNOWN_EVENT_TYPES` const **removed**.
- `validate_event_types` is now `async`, signature:
  `(pool, org_id, event_types) -> ApiResult<()>`.
  Calls `core_db::event_types::check_event_types`, maps the ungranted
  list to `ApiError::Validation` (422).
- Callers updated at `create` (L222) and `update` (L284) to pass
  `&state.pool`, `org_id`, and `.await`.

### Validation rule (enforced in `check_event_types`)

1. Empty `event_types` → wildcard → OK (no validation).
2. For each type in the filter list:
   - If it exists in `event_types` with `source = 'platform'` → **allowed**
     (no grant needed).
   - If it exists in `event_types` (any source) AND has a grant for this
     org in `event_type_grants` → **allowed**.
   - Otherwise → **rejected** (returned in the ungranted list).

### What does NOT change

- Delivery-side matching: `find_enabled_for_event` in `webhooks.rs:154`
  already matches the stream against `event_types`. An empty array is a
  wildcard (all types). No change needed there.
- `alert.ingested` Rust constant at `events.rs:298` — already exists in
  the domain module, already included in the seed.
- `webhook.received` was added **only** to the `event_types` table seed,
  NOT as a Rust constant in `events.rs::types` (which would break
  `core-events/tests/event_conformance.rs`).

## Test Coverage

`crates/core-api/tests/event_type_registry_flow.rs`:

| Test | Scenario | Expected |
|------|----------|----------|
| `platform_types_accepted_without_grant` | Subscribe to `ticket.created`, `ticket.closed`, `webhook.test` | 201 |
| `lane1_types_accepted_without_grant` | Subscribe to `alert.ingested`, `webhook.received` | 201 |
| `unknown_type_rejected_without_grant` | Subscribe to `my.custom.event` (not seeded, no grant) | 422 |
| `granted_custom_type_accepted` | Insert grant row for `my.custom.event`, then subscribe | 201 |
| `update_rejects_ungranted_type` | PATCH subscription to `not.registered` | 422 |
| `mixed_platform_and_custom_types_partial_failure` | `ticket.created` + granted `my.custom.event` in one request | 201 |

The existing test in `integrations_flow.rs:160` (`"not.an.event"` → 422)
continues to pass — that type has no seed row and no grant.

## Files

- `crates/core-db/migrations/0124_event_type_registry.sql` — migration
- `crates/core-db/src/event_types.rs` — Rust query module (NEW)
- `crates/core-db/src/lib.rs` — module registration
- `crates/core-api/src/routes/webhook_admin.rs` — refactored validation
- `crates/core-api/tests/event_type_registry_flow.rs` — integration tests
