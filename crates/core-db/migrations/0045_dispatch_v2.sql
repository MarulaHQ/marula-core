-- Dispatch v2 (the private build tracker Phase 3, Wave 9 lane 9): resource planning,
-- the dispatch connector's external-status ledger, and travel/mileage
-- entry drafts. Five org-scoped tables, all RLS-bound like every other
-- tenant table (0001 convention), all additive.
--
-- `dispatch_queue_entries` — dispatcher overrides for the unassigned
-- queue. The base queue stays computed (priority rank, then effective
-- SLA/due deadline — `unassigned_queue_annotated`); an override row pins
-- a ticket to an explicit rank that sorts ahead of the computed order.
-- UNIQUE (org_id, ticket_id): one override per ticket, upsert semantics.
--
-- `user_skills` — the technician skill/certification catalog. A row is
-- "current" while `expires_at` is NULL or in the future, so the same
-- table covers plain skills and expiring certifications without a second
-- entity. UNIQUE (user_id, skill): skill labels are org-vocabulary.
--
-- `ticket_required_skills` — the ticket side of matching: which skills a
-- job needs. Matching (GET /api/dispatch/match) intersects the two
-- tables; nothing here enforces that a required skill exists on anyone.
--
-- `travel_entry_drafts` — draft time-entry-like rows for travel/mileage:
-- a worked window (so the eventual time entry carries real minutes) plus
-- distance/vehicle fields the time-entry schema has no home for. Submit
-- converts through the EXISTING time-entry path (insert_time_entry + the
-- time_entry.created event) and stamps `submitted_time_entry_id` + the
-- write-trail row as the provenance query; the draft is never rewritten
-- by the submit — its status flips in the same transaction. Distance is
-- double precision (mileage entries need one-decimal precision, not
-- ledger arithmetic); `distance_unit` is pinned to km|mi.
--
-- `dispatch_status_updates` — append-only ledger of field-service status
-- pushes (en_route, arrived, in_progress, completed, blocked, delayed)
-- against an appointment, from the dispatch connector (actor = api key)
-- or staff. The appointment's own lifecycle status (scheduled/completed/
-- cancelled) is untouched; this ledger is the connector-visible ground
-- truth and the board's per-block status shading source.

CREATE TABLE dispatch_queue_entries (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  ticket_id uuid NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
  rank integer NOT NULL CHECK (rank >= 0),
  note text CHECK (char_length(note) <= 500),
  created_by uuid NOT NULL REFERENCES users(id),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, ticket_id)
);
CREATE INDEX idx_dispatch_queue_entries_rank ON dispatch_queue_entries (org_id, rank);

ALTER TABLE dispatch_queue_entries ENABLE ROW LEVEL SECURITY;
CREATE POLICY dispatch_queue_entries_org ON dispatch_queue_entries
  USING (org_id = current_setting('app.org_id', true)::uuid);

CREATE TABLE user_skills (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  skill text NOT NULL CHECK (char_length(skill) BETWEEN 1 AND 80),
  -- NULL = does not expire (a plain skill); set = certification expiry.
  expires_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (user_id, skill)
);
CREATE INDEX idx_user_skills_org ON user_skills (org_id, skill);

ALTER TABLE user_skills ENABLE ROW LEVEL SECURITY;
CREATE POLICY user_skills_org ON user_skills
  USING (org_id = current_setting('app.org_id', true)::uuid);

CREATE TABLE ticket_required_skills (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  ticket_id uuid NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
  skill text NOT NULL CHECK (char_length(skill) BETWEEN 1 AND 80),
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (ticket_id, skill)
);
CREATE INDEX idx_ticket_required_skills_org ON ticket_required_skills (org_id, skill);

ALTER TABLE ticket_required_skills ENABLE ROW LEVEL SECURITY;
CREATE POLICY ticket_required_skills_org ON ticket_required_skills
  USING (org_id = current_setting('app.org_id', true)::uuid);

CREATE TABLE travel_entry_drafts (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  ticket_id uuid NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
  -- The worked window the eventual time entry carries.
  started_at timestamptz NOT NULL,
  ended_at timestamptz NOT NULL,
  distance double precision NOT NULL CHECK (distance > 0),
  distance_unit text NOT NULL CHECK (distance_unit IN ('km','mi')),
  vehicle text NOT NULL CHECK (char_length(vehicle) BETWEEN 1 AND 40),
  billable boolean NOT NULL DEFAULT false,
  notes text CHECK (char_length(notes) <= 2000),
  status text NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','submitted')),
  -- Set at submit; the backlink that keeps draft ↔ entry provenance
  -- queryable ("who did this" resolves through entity_writes + audit).
  submitted_time_entry_id uuid,
  created_by uuid NOT NULL REFERENCES users(id),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CHECK (ended_at > started_at),
  -- A submitted draft is frozen: the entry exists, edits would fork it.
  CHECK (status = 'draft' OR submitted_time_entry_id IS NOT NULL)
);
CREATE INDEX idx_travel_entry_drafts_org ON travel_entry_drafts (org_id, status, user_id);

ALTER TABLE travel_entry_drafts ENABLE ROW LEVEL SECURITY;
CREATE POLICY travel_entry_drafts_org ON travel_entry_drafts
  USING (org_id = current_setting('app.org_id', true)::uuid);

CREATE TABLE dispatch_status_updates (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  appointment_id uuid NOT NULL REFERENCES appointments(id) ON DELETE CASCADE,
  status text NOT NULL CHECK (status IN
    ('en_route','arrived','in_progress','completed','blocked','delayed')),
  note text CHECK (char_length(note) <= 500),
  -- The pushing principal: an api-key id (connector) or a staff user.
  actor_id uuid NOT NULL,
  source text NOT NULL CHECK (source IN ('connector','staff')),
  occurred_at timestamptz NOT NULL DEFAULT now(),
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_dispatch_status_updates_appt
  ON dispatch_status_updates (org_id, appointment_id, occurred_at DESC);

ALTER TABLE dispatch_status_updates ENABLE ROW LEVEL SECURITY;
CREATE POLICY dispatch_status_updates_org ON dispatch_status_updates
  USING (org_id = current_setting('app.org_id', true)::uuid);
