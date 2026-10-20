-- Two-way calendar sync (Wave 14 lane 2, the private build tracker Phase 4 opening tranche).
--
-- Purpose: the data model for the `CalendarSyncProvider` adapter seam and
-- the first-party iCal provider:
-- (1) `calendar_sync_connections` — one connection per (user, provider
--     feed). `direction` selects what the connection does: `outbound`
--     serves the org's appointments to the external calendar (a private
--     token-scoped iCal feed), `inbound` polls an external iCal URL for
--     busy time, `two_way` does both. `sync_token`/`last_synced_at`/
--     `last_error` are the poll state the provider seam maintains.
-- (2) `calendar_sync_events` — the inbound mirror: every external event
--     the provider last pulled for a connection, keyed by the external
--     UID (the duplicate rule: one mirror row per (connection, UID);
--     a re-poll with the same UID upserts — last SEQUENCE wins).
--
-- Busy-time flow (the 2.12 availability foundation contract): every
-- non-cancelled, non-transparent inbound event maps 1:1 onto a
-- `calendar_exclusions` row (0022) via `exclusion_id`, so the SAME
-- book-time double-check portal appointment requests and scheduling-link
-- bookings run (windows − on-call − time-off − bookings, re-read inside
-- the booking transaction) automatically honors external busy time. No
-- second conflict algorithm: external busy time IS time-off to the
-- availability math. On re-poll, moved/renamed events update their
-- exclusion span in place; events that disappear or arrive
-- STATUS:CANCELLED (or TRANSP:TRANSPARENT) have their exclusion row
-- removed/never created, freeing the slot again. Deleting the connection
-- cascades to its mirror rows and, through them, to the exclusion rows
-- the sync owns.
--
-- Conflict rule (documented choice): inbound busy time never blocks or
-- rewrites existing appointments. It feeds availability; an overlap with
-- a live appointment surfaces as an ordinary booking conflict at the
-- next conflict check, and the sync run reports it via
-- `calendar.sync_completed.overlaps` rather than failing the poll.
--
-- Duplicate rule: external UIDs are unique per connection (the UNIQUE
-- constraint below) — the same UID across DIFFERENT connections coexists
-- (two feeds may legitimately publish the same meeting). Within one
-- connection the highest SEQUENCE wins.
--
-- RLS: every new user-data table carries an org-scoped policy — the CI
-- gate (`assert_rls_coverage`) fails otherwise.

CREATE TYPE calendar_sync_direction AS ENUM ('outbound', 'inbound', 'two_way');
CREATE TYPE calendar_sync_status AS ENUM ('active', 'error', 'disabled');

CREATE TABLE calendar_sync_connections (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- The user whose calendar this connection syncs.
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name text NOT NULL,
  -- The provider seam discriminator; 'ical' is the first-party provider.
  provider text NOT NULL DEFAULT 'ical' CHECK (provider IN ('ical')),
  direction calendar_sync_direction NOT NULL DEFAULT 'two_way',
  status calendar_sync_status NOT NULL DEFAULT 'active',
  -- Inbound: the external iCal URL the provider polls.
  feed_url text,
  -- Outbound: SHA-256 hash of the private feed token; the raw token is
  -- returned exactly once at creation/regeneration and never stored.
  feed_token_hash text,
  -- Provider poll state (opaque to the API; the provider seam owns it).
  sync_token text,
  last_synced_at timestamptz,
  last_error text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX calendar_sync_connections_org_id_user_id_name_key
  ON calendar_sync_connections (org_id, user_id, name);
CREATE UNIQUE INDEX idx_calendar_sync_feed_token
  ON calendar_sync_connections (feed_token_hash)
  WHERE feed_token_hash IS NOT NULL;

ALTER TABLE calendar_sync_connections ENABLE ROW LEVEL SECURITY;
CREATE POLICY calendar_sync_connections_org ON calendar_sync_connections
  USING (org_id = current_setting('app.org_id', true)::uuid);

CREATE TABLE calendar_sync_events (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  connection_id uuid NOT NULL REFERENCES calendar_sync_connections(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  -- The external event's UID (stable across re-polls) — the duplicate key.
  external_uid text NOT NULL,
  -- External sequence: highest wins on re-poll.
  sequence integer NOT NULL DEFAULT 0 CHECK (sequence >= 0),
  title text,
  starts_at timestamptz NOT NULL,
  ends_at timestamptz NOT NULL,
  -- The exclusion row this event currently feeds into the 2.12
  -- availability foundation. NULL while the event is cancelled or
  -- transparent (not busy). Deleting the exclusion cascades here and the
  -- next poll re-imports the event.
  exclusion_id uuid REFERENCES calendar_exclusions(id) ON DELETE CASCADE,
  first_seen_at timestamptz NOT NULL DEFAULT now(),
  last_seen_at timestamptz NOT NULL DEFAULT now(),
  CHECK (ends_at > starts_at)
);

-- The duplicate rule: one mirror row per (connection, external UID).
CREATE UNIQUE INDEX calendar_sync_events_org_id_connection_id_external_uid_key
  ON calendar_sync_events (org_id, connection_id, external_uid);
CREATE INDEX idx_calendar_sync_events_user_time
  ON calendar_sync_events (org_id, user_id, starts_at);

ALTER TABLE calendar_sync_events ENABLE ROW LEVEL SECURITY;
CREATE POLICY calendar_sync_events_org ON calendar_sync_events
  USING (org_id = current_setting('app.org_id', true)::uuid);
