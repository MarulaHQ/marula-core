-- Wave 34 claims close-out (`p34-verdicts-closeout`) — the four
-- schema-bearing rows of the verdict sweep's "Wave 34 verdict-sweep claims"
-- section. This is the lane's exclusive migration window; head was
-- 0115_wave41_marula_theme.
--
-- Four independent, additive changes, grouped by the row that needs them.
-- All follow policy §4.1: additive only, nullable until a caller writes,
-- no rewrite of an existing row, no default that invents data.
--
--   (1) project_phase_allocations          — claim 1, resource planning
--   (2) dispatch_work_next_queue           — claim 2, curated per-tech queues
--   (3) calendar_sync_connections          — claim 5, Google + M365 kinds
--   (4) project_phases dates               — claim 6, dated Gantt
--
-- ---------------------------------------------------------------------------
-- (1) project_phase_allocations — claim 1
--
-- What the shipped `/dispatch/planning` horizon does NOT have: it allocates
-- against DISPATCH APPOINTMENTS only. A technician's real load is the sum of
-- booked appointments and committed project phases, and a planner cannot
-- see the second half of that sum anywhere in the product today. This table
-- is the missing half: a named allocation of a technician to a project
-- phase, for a date range, at a stated share of their time.
--
-- Why a table and not a computed join over time_entries:
--
--   - A phase has a PLANNED window that is not the same thing as the time
--     actually booked against it. "Alice is 50% on the Acme migration
--     between the 3rd and the 21st" is a planning fact an org states; it is
--     not derivable after the fact from entries, and an org that is behind
--     schedule has under-booked entries precisely when the allocation is
--     most interesting.
--   - Capacity forecasting needs a denominator that is stable enough to
--     compare against a forecast. Logged hours are a lagging indicator and
--     are exactly wrong for the current week.
--
-- `share_percent` is an INTEGER 1..=100, NOT a float: a float invites
-- "33.3333%" allocations that no planner can act on, and the CHECK keeps
-- both 0 and >100 out. It is a share of ONE technician's available minutes,
-- not of the phase — 100 means that technician is fully on that phase for
-- the window, which is the case a planner needs to see (an overallocated
-- technician).
--
-- ON DELETE CASCADE from phase and from user: an allocation has no meaning
-- without its phase, and a departed technician's allocations must not linger
-- as phantom load in the forecast. The forecast is a read model over live
-- allocations, so orphan rows would be load nobody owns.
--
-- The UNIQUE (phase_id, user_id) pair is the "one allocation per technician
-- per phase" rule, and it is what lets the UI upsert rather than append: a
-- planner correcting a 60% to 40% updates the row instead of creating a
-- second, contradictory one whose two shares would SUM in the forecast.
CREATE TABLE project_phase_allocations (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- CASCADE: no phase, no allocation.
  phase_id uuid NOT NULL REFERENCES project_phases(id) ON DELETE CASCADE,
  -- CASCADE: a departed technician's allocations must vanish from the
  -- forecast rather than linger as load nobody owns.
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  -- The PLANNED window. Nullable because a planner allocating someone to an
  -- undated phase is a real and common state ("Alice owns the migration,
  -- we will date it later") and a NOT NULL here would force fake dates.
  start_date date,
  end_date date,
  -- 1..=100 share of THIS technician's available minutes. CHECK-enforced
  -- rather than validated in the app layer: a 0 or a 150 silently entering
  -- the forecast would corrupt every capacity number on the page.
  share_percent integer NOT NULL DEFAULT 100
    CHECK (share_percent >= 1 AND share_percent <= 100),
  -- Free-text rationale, surfaced in the planning UI so a dispatcher's
  -- reasoning is visible to the next dispatcher. Nullable, never required:
  -- a required note is a note nobody writes.
  note text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  -- The end date cannot precede the start. NULLS NOT DISTINCT semantics
  -- handled by writing the predicate so a half-dated allocation (only one
  -- end set) is allowed, which is the "we know when it starts" case.
  CONSTRAINT project_phase_allocations_dates_ordered
    CHECK (end_date IS NULL OR start_date IS NULL OR end_date >= start_date),
  -- One allocation per technician per phase. This is the anti-double-count
  -- rule, and it is what makes the forecast a sum rather than a mess.
  UNIQUE (phase_id, user_id)
);

-- The forecast reads allocations for a date window per technician, so the
-- leading column is user_id (the access pattern is "everyone's load in this
-- window"), then the window predicate.
CREATE INDEX idx_phase_allocations_user_window
  ON project_phase_allocations (org_id, user_id, start_date, end_date);
-- And the reverse read, "who is on this phase", for the phase row itself.
CREATE INDEX idx_phase_allocations_phase
  ON project_phase_allocations (phase_id);

ALTER TABLE project_phase_allocations ENABLE ROW LEVEL SECURITY;
CREATE POLICY project_phase_allocations_org ON project_phase_allocations
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- ---------------------------------------------------------------------------
-- (2) dispatch_work_next_queue — claim 2
--
-- The dispatch board ships a P1-P4-filterable UNASSIGNED queue plus skill
-- matching and routing suggestions. What it does not ship is the thing a
-- technician actually asks for: "what should I pick up next, in what
-- order?" That ordering is a per-technician, persisted, human-curated list
-- — not a filter, not a sort, and emphatically not something recomputed from
-- ticket fields, because the whole value is that a dispatcher pushed a
-- ticket up deliberately.
--
-- `position` is an INTEGER, not a float and not a timestamp: reordering a
-- queue is a drag, and a float gap leaves holes that accumulate. The API
-- compacts on every reorder (the handler rewrites the affected window), so
-- the integers stay dense and the endpoint's ordering is a plain ORDER BY.
--
-- Deliberately NOT a priority override. A curated order is advisory: it
-- changes what a technician sees first, never what the board's
-- conflict detection or the P1-P4 filter believes. Folding the curation
-- into priority would make a dispatcher's drag silently rewrite a ticket's
-- business meaning.
--
-- ON DELETE CASCADE from ticket: removing the ticket removes its queue
-- entries everywhere. A queue entry pointing at a deleted ticket is a row
-- the board renders as a ghost.
CREATE TABLE dispatch_work_next_queue (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- The technician whose queue this is. CASCADE with the user for the same
  -- reason as the allocation above.
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  ticket_id uuid NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
  position integer NOT NULL CHECK (position >= 0),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  -- A ticket appears at most once in a given technician's queue. Two rows
  -- for one ticket would render it twice and count it twice in the badge.
  UNIQUE (user_id, ticket_id)
);

-- The endpoint's access pattern is "this technician's queue in order".
CREATE INDEX idx_work_next_user_position
  ON dispatch_work_next_queue (org_id, user_id, position);
-- And the "which queues mention this ticket" read, so deleting or closing a
-- ticket can clean every technician's queue in one statement.
CREATE INDEX idx_work_next_ticket
  ON dispatch_work_next_queue (ticket_id);

ALTER TABLE dispatch_work_next_queue ENABLE ROW LEVEL SECURITY;
CREATE POLICY dispatch_work_next_queue_org ON dispatch_work_next_queue
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- ---------------------------------------------------------------------------
-- (3) calendar_sync_connections — claim 5, Google Workspace + M365 kinds
--
-- The `CalendarSyncProvider` seam shipped with exactly one implementation
-- ('ical'), and the DB CHECK was the belt to that braces: only 'ical' could
-- exist. This widens the CHECK to admit the two delegated OAuth providers
-- the seam header always named, and adds the columns an authorization-code
-- flow needs.
--
-- The OAuth credentials themselves are NOT here. The refresh token is a
-- long-lived bearer credential that can read and write a user's entire
-- calendar, and the repo's established answer to "where does a long-lived
-- third-party credential live" is the SEALED store (`byok_secrets`, the same
-- `mail.*` reserved-provider pattern the transport credentials use) — so
-- the token pair is sealed under `calendar.google` / `calendar.microsoft`
-- and this row carries only the non-secret bookkeeping. That is a
-- deliberate split, and the reason `access_token` below is a CACHE and not
-- the credential of record.
--
-- `access_token` is cached here anyway, with its expiry, because an hourly
-- round trip to the provider on every poll would make the poll endpoint a
-- proxy for Google's token service. It is short-lived (Google: 3600s) and
-- the sealed refresh token remains the thing that can regenerate it. If
-- this table is ever backed up to somewhere unencrypted, the exposure is one
-- hour of read-only calendar access rather than a durable credential.
--
-- `oauth_state` is the CSRF token for the authorization-code redirect, and
-- `oauth_state_expires_at` bounds it. An unbounded state row is a standing
-- invitation to finish someone else's authorization flow.
--
-- provider values are the two SEAM KINDS, spelled to match the
-- implementation names in `provider_for` ('ical', 'google', 'microsoft').
-- 'microsoft' rather than 'm365' or 'o365': those are marketing names for
-- the same thing, and one spelling is one fewer thing to normalize later.
ALTER TABLE calendar_sync_connections
  DROP CONSTRAINT IF EXISTS calendar_sync_connections_provider_check;

ALTER TABLE calendar_sync_connections
  ADD CONSTRAINT calendar_sync_connections_provider_check
  CHECK (provider IN ('ical', 'google', 'microsoft'));

-- The CSRF nonce for the authorization-code redirect, plus its expiry.
-- Both NULL for 'ical', which has no consent step.
ALTER TABLE calendar_sync_connections
  ADD COLUMN oauth_state text,
  ADD COLUMN oauth_state_expires_at timestamptz,
  -- Cached short-lived access token (see the note above). NULL when the
  -- sealed refresh token has not been redeemed yet.
  ADD COLUMN access_token text,
  ADD COLUMN token_expires_at timestamptz,
  -- The scopes actually granted, recorded so an operator can see what the
  -- consent screen said and what we stored agree. Read-only by construction:
  -- Marula never widens its own grant after consent.
  ADD COLUMN scopes text,
  -- The provider's account identifier (Google `sub`, Microsoft `oid` /
  -- `preferred_username`). NOT a credential and NOT an identity we use for
  -- login — it is recorded so an operator can tell WHICH account the
  -- connection is bound to when auditing a sync.
  ADD COLUMN external_account text;

-- An authorization state is only meaningful before it is used and before it
-- expires. Enforced here so a half-finished flow cannot linger.
ALTER TABLE calendar_sync_connections
  ADD CONSTRAINT calendar_sync_connections_oauth_state_pair
  CHECK (
    (oauth_state IS NULL AND oauth_state_expires_at IS NULL)
    OR (oauth_state IS NOT NULL AND oauth_state_expires_at IS NOT NULL)
  );

-- A cached token is only usable with its expiry. Same reasoning: a token row
-- with no expiry would be treated as valid forever.
ALTER TABLE calendar_sync_connections
  ADD CONSTRAINT calendar_sync_connections_token_pair
  CHECK (
    (access_token IS NULL AND token_expires_at IS NULL)
    OR (access_token IS NOT NULL AND token_expires_at IS NOT NULL)
  );

-- Connections still mid-consent are found by their pending state, not by a
-- full scan.
CREATE INDEX idx_calendar_sync_pending_oauth
  ON calendar_sync_connections (org_id, oauth_state_expires_at)
  WHERE oauth_state IS NOT NULL;

-- ---------------------------------------------------------------------------
-- (4) project_phases dates — claim 6, dated Gantt
--
-- Phases carry `sort_order` and no dates, so the project page can only
-- render an ordered list and a typed not-built for a Gantt. These two
-- columns are the entire gap: a start and an end per phase, from which the
-- bar chart derives, the phase-budget burn view anchors, and the critical
-- path is computable.
--
-- Both NULLABLE, and that is the honest state rather than a compromise:
-- most real projects have phases nobody has dated yet, and a NOT NULL pair
-- with a fake default (say, "today") would draw a Gantt that lies about the
-- plan. The UI renders undated phases in the ordered list and dated ones on
-- the bar chart, and says which is which.
--
-- end_date >= start_date is CHECK-enforced because the alternative is a bar
-- drawn backwards, which reads as a rendering bug and is actually a data
-- problem the operator needs told about.
ALTER TABLE project_phases
  ADD COLUMN start_date date,
  ADD COLUMN end_date date;

ALTER TABLE project_phases
  ADD CONSTRAINT project_phases_dates_ordered
  CHECK (end_date IS NULL OR start_date IS NULL OR end_date >= start_date);

-- The Gantt reads a project's phases ordered by start, and the portfolio
-- rollup reads every dated phase in a window across projects.
CREATE INDEX idx_project_phases_dates
  ON project_phases (org_id, start_date, end_date);
