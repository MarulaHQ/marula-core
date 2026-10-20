-- Wave 32 lane 7: Dispatch v3 — routing provider seam + routing-aware
-- scheduling (the private build tracker Wave 32 lane 7; the RoutingProvider lane that
-- migration 0022 explicitly deferred travel math to).
--
-- Purpose:
-- (1) `appointments` gains travel PROVENANCE — which provider produced
--     `travel_minutes`, at what quality (heuristic vs measured), the
--     distance behind it, and an immutable snapshot of the resolved
--     journey start point the estimate ran from. `travel_minutes` itself
--     stays the scheduling-facing number (occupied capacity, see
--     core-domain::scheduling::occupied_minutes).
-- (2) `journey_start_points` — a per-technician, per-day start point a
--     dispatcher can set (home / office / a site / an ad-hoc address) so
--     a journey models reality (home -> site -> office -> site). One row
--     per (user, day); absence falls back to the auto-suggested start
--     (the previous day's last booked site) at read time.
-- (3) `travel_entry_drafts` gains the planned-appointment link (actual
--     travel reconciles against the schedule) and the created-expense
--     link (mileage reimbursement is an existing ticket_expense, never a
--     forked ledger row).
--
-- Non-purpose: no routing state tables (providers are operator config,
-- registry lives in-process), no per-leg cache table (estimates are
-- recomputed at save/read time from the provider), no new money entity
-- (expenses stay ticket_expenses), no entitlement machinery (ADR 0013:
-- premium routing packs gate themselves, never the core).
--
-- RLS: journey_start_points carries the standard org-scoped policy; the
-- other statements only widen existing org-scoped tables.

-- --- appointments: travel provenance ---------------------------------------

ALTER TABLE appointments
  -- Snapshot of the resolved start point: {kind, label, site_id,
  -- latitude, longitude, source}. JSONB so the snapshot can evolve
  -- without a migration; readers treat unknown keys as absent.
  ADD COLUMN travel_from jsonb,
  -- Provider-reported distance for the leg, metres (canonical unit).
  ADD COLUMN travel_distance_meters integer
    CHECK (travel_distance_meters IS NULL OR travel_distance_meters >= 0),
  -- The provider id that produced travel_minutes (`none` for the
  -- built-in heuristic, `osrm`, `mapbox`, `contract:<pack>` …); NULL when
  -- travel_minutes was set by hand and is not provider provenance.
  ADD COLUMN travel_provider text,
  -- `heuristic` | `measured`; NULL whenever travel_provider is NULL.
  ADD COLUMN travel_quality text
    CHECK (travel_quality IS NULL OR travel_quality IN ('heuristic', 'measured'));

COMMENT ON COLUMN appointments.travel_minutes IS
  'Travel minutes immediately BEFORE starts_at; OCCUPIED capacity for '
  'scheduling math (core-domain::scheduling::occupied_minutes) and never '
  'double-counted against the appointment span itself.';
COMMENT ON COLUMN appointments.travel_from IS
  'Resolved journey start point snapshot the estimate ran from '
  '(Wave 32 lane 7).';

-- --- journey start points ---------------------------------------------------

CREATE TABLE journey_start_points (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- The technician whose day this journey belongs to.
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  -- Local-to-the-technician calendar day (the dispatch board's day).
  day date NOT NULL,
  kind text NOT NULL CHECK (kind IN ('home', 'office', 'site', 'address')),
  -- Required when kind = 'site' (and the coordinates resolve through it).
  site_id uuid REFERENCES sites(id) ON DELETE SET NULL,
  -- Free-form address for home/office/address kinds. Coordinates may be
  -- supplied directly (already geocoded) or resolved by a provider with
  -- the geocode capability at read time — never guessed.
  address text CHECK (address IS NULL OR char_length(address) <= 300),
  label text CHECK (label IS NULL OR char_length(label) <= 120),
  latitude double precision CHECK (latitude IS NULL OR latitude BETWEEN -90 AND 90),
  longitude double precision CHECK (longitude IS NULL OR longitude BETWEEN -180 AND 180),
  created_by uuid NOT NULL REFERENCES users(id),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  -- One start point per technician per day: the dispatcher's override is
  -- a SET operation, not an ever-growing log.
  UNIQUE (org_id, user_id, day),
  -- A site start point must name its site; a site plus an address is a
  -- contradiction (resolve one source of truth).
  CHECK (kind <> 'site' OR site_id IS NOT NULL),
  CHECK (kind <> 'address' OR address IS NOT NULL OR (latitude IS NOT NULL AND longitude IS NOT NULL))
);
CREATE INDEX idx_journey_start_points_org_day
  ON journey_start_points (org_id, day, user_id);

ALTER TABLE journey_start_points ENABLE ROW LEVEL SECURITY;
CREATE POLICY journey_start_points_org ON journey_start_points
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- --- travel drafts: reconciliation links ------------------------------------

ALTER TABLE travel_entry_drafts
  -- The planned appointment this trip reconciles (optional — ad-hoc
  -- travel against a ticket stays legal).
  ADD COLUMN appointment_id uuid REFERENCES appointments(id) ON DELETE SET NULL,
  -- The ticket_expense this draft produced when its mileage was
  -- reimbursed; a submitted draft with an expense row is settled.
  ADD COLUMN expense_id uuid;

CREATE INDEX idx_travel_entry_drafts_appointment
  ON travel_entry_drafts (org_id, appointment_id)
  WHERE appointment_id IS NOT NULL;
