-- Rack records and their unit placements (surface-pack lane, migration 0213).
--
-- Purpose: Hudu-parity rack documentation — a 42U elevation per physical rack,
-- a device per unit on the front or the rear face, and the power and network
-- notes that make an elevation useful during an incident.
--
-- `knowledge_racks` is the rack; `knowledge_rack_devices` is one placement.
-- A placement may point at a `device_assets` row (the configuration record)
-- or stand alone as passive gear (patch panels, cable management), which is
-- why `asset_id` is nullable and the free-text `name` is required.
--
-- Overlap: the UNIQUE (rack_id, face, unit_start) gate refuses two devices
-- starting on the same unit. A truthful full-range exclusion would need a
-- GiST constraint on `int4range`, which the route layer would then have to
-- translate into a sentence; the app checks the range before insert and the
-- unique index is the backstop for the common case.
--
-- RLS on both tables, and every list index leads with (org_id, client_id) as
-- the knowledge brief requires.

CREATE TABLE knowledge_racks (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  client_id uuid NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
  site_id uuid REFERENCES sites(id) ON DELETE SET NULL,
  name text NOT NULL,
  code text,
  units smallint NOT NULL DEFAULT 42 CHECK (units BETWEEN 4 AND 58),
  width_mm integer CHECK (width_mm IS NULL OR width_mm > 0),
  depth_mm integer CHECK (depth_mm IS NULL OR depth_mm > 0),
  power_capacity_kw numeric(6, 2) CHECK (power_capacity_kw IS NULL OR power_capacity_kw >= 0),
  power_used_kw numeric(6, 2) CHECK (power_used_kw IS NULL OR power_used_kw >= 0),
  cooling text,
  location_note text,
  notes text,
  fields jsonb NOT NULL DEFAULT '{}',
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  updated_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, client_id, name)
);

CREATE INDEX idx_knowledge_racks_scope
  ON knowledge_racks (org_id, client_id, name);
CREATE INDEX idx_knowledge_racks_site
  ON knowledge_racks (org_id, site_id);

CREATE TABLE knowledge_rack_devices (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  rack_id uuid NOT NULL REFERENCES knowledge_racks(id) ON DELETE CASCADE,
  asset_id uuid REFERENCES device_assets(id) ON DELETE SET NULL,
  name text NOT NULL,
  device_type text NOT NULL DEFAULT 'compute'
    CHECK (device_type IN ('network', 'compute', 'storage', 'power', 'patch', 'other')),
  face text NOT NULL DEFAULT 'front' CHECK (face IN ('front', 'rear')),
  unit_start smallint NOT NULL CHECK (unit_start >= 1),
  unit_height smallint NOT NULL DEFAULT 1 CHECK (unit_height BETWEEN 1 AND 12),
  model text,
  serial text,
  status text NOT NULL DEFAULT 'online'
    CHECK (status IN ('online', 'attention', 'offline', 'passive')),
  power_notes text,
  network_notes text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (rack_id, face, unit_start)
);

CREATE INDEX idx_knowledge_rack_devices_rack
  ON knowledge_rack_devices (org_id, rack_id, face, unit_start);

ALTER TABLE knowledge_racks ENABLE ROW LEVEL SECURITY;
CREATE POLICY knowledge_racks_org ON knowledge_racks
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE knowledge_rack_devices ENABLE ROW LEVEL SECURITY;
CREATE POLICY knowledge_rack_devices_org ON knowledge_rack_devices
  USING (org_id = current_setting('app.org_id', true)::uuid);
