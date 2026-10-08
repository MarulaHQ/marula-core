# E2E: Inbound Webhook Receiver (Phase 4)

## Goal
Accept signed webhook payloads from external systems into Marula, deduplicating within a rate window and emitting a `WEBHOOK_INGESTED` domain event onto the outbox.

## Implementation

### Migration
`0123_generic_webhook_source.sql` extends the alerting schema:
- `alert_sources.event_type` (nullable) — if set, the source is a "generic webhook" endpoint at `/api/v1/webhooks/ingest/{source_ref}`.
- `alert_sources.payload_schema_ref` (nullable) — optional schema pointer for validation.
- `alert_source_auth_lookup` gains `event_type` and `payload_schema_ref` columns so auth lookups are single-query.
- `webhook_ingest_dedup(org_id, source_id, dedup_key, occurred_at)` — dedup ledger with UNIQUE (org, source, dedup_key) fence and rate-limit index on occurred_at. RLS policy scopes to org.

### Handler
`webhook_receiver.rs` (`post(webhook_ingest)`):
1. Looks up source by `source_ref` via `auth_lookup_source` → 404 if missing.
2. Checks `r_enabled` → 403 if disabled.
3. Checks `r_event_type` → 404 if source isn't a generic webhook (no event_type set).
4. Verifies `X-Marula-Signature` (HMAC-SHA256 of raw body using source secret) → 401 if missing/bad.
5. Calls `alerting::count_recent_ingests` → 429 RateLimited if ≥ `RATE_MAX_PER_WINDOW` in `RATE_WINDOW_SECS` (300s).
6. Calls `alerting::insert_webhook_dedup` → if `!was_new`, returns 200 with `{"status":"duplicate"}`.
7. On new ingest, inserts into `outbox_events` with `event_type = source.event_type`, entity = `"webhook"`, entity_id = source_ref, payload = envelope JSON.
8. Inserts an `audit::insert` row with `WEBHOOK_INGESTED` action.
9. Returns 200 with `WebhookIngestResponse { status: "emitted", event_id, board_id }`.

### Routing
Mounted in `routes/mod.rs`:
```rust
pub mod webhook_receiver;
...
.merge(webhook_receiver::public_routes(gen))
```

### Alert Source Creation
- `CreateAlertSourceRequest` gains optional `event_type` and `payload_schema_ref`.
- `event_type` validation: non-empty, dot-path format (must contain `.`), max 100 chars.
- `PatchAlertSourceRequest` uses `Option<Option<String>>` for patch semantics (null = clear, Some = set).
- If `event_type` is set, the source is addressable at the generic ingest path. Classic alerting (no `event_type`) continues to use `/api/v1/alerts/ingest/{source_ref}` via the existing alerts handler.

## Key Decisions
- **Route paths**: Generic webhooks at `/api/v1/webhooks/ingest/{source_ref}` and `/api/v1/alerts/ingest/{source_ref}`; both in both `/api` and `/api/v1` generations. Mounted in `public_routes`, NOT behind JWT middleware (uses HMAC signature instead).
- **Signature header**: `X-Marula-Signature` (shared from `alerts` module via `crate::routes::alerts::SIGNATURE_HEADER`).
- **Dedup**: UNIQUE constraint on `(org_id, source_id, dedup_key)` — duplicates return 200 with `status: "duplicate"` (not 409).
- **Outbox entity_id**: Source ref, so downstream consumers can correlate.
- **Payload**: Envelope JSON `{ board_id, source_ref, source_id, entity_id, data }` stored in `outbox_events.payload`.
- **Org scoping**: `with_org_tx` passes `source.r_org_id` as both org_id and user_id (no JWT user on this endpoint; the org is resolved from the source lookup).

## Gate Results
- **`cargo check -p core-api -p core-db` (SQLX_OFFLINE=true)**: PASS — zero errors.
- **`cargo check -p core-api -p core-db` (SQLX_OFFLINE=false, online)**: PASS — zero errors. SQLx macros resolve against DB schema.
- **`sqlx prepare --workspace`**: FAILS — but only due to pre-existing `marula-desktop` crate error (`frontendDist` at `../apps/web/out` doesn't exist, requires `npm run build` first). Not related to this change.
- **`cargo test -p core-api --test webhook_receiver_flow`**: PASS — 4/4 tests green:
  - `ingest_auth_shell` — 404/401 paths verified
  - `generic_ingest_emits_envelope` — outbox event verified
  - `replay_returns_duplicate` — dedup fence verified
  - `dual_mount_parity` — `/api` + `/api/v1` parity verified
