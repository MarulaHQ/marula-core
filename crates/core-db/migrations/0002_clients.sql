-- Client hierarchy: company → site → contact → device/asset.

CREATE TABLE clients (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  restricted_to jsonb NOT NULL DEFAULT '[]',
  notes text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_clients_org ON clients (org_id, name);

CREATE TABLE sites (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  client_id uuid NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
  name text NOT NULL,
  address text,
  city text,
  region text,
  postal_code text,
  country text,
  latitude double precision,
  longitude double precision,
  timezone text,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_sites_client ON sites (client_id);

CREATE TABLE contacts (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  client_id uuid NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
  site_id uuid REFERENCES sites(id) ON DELETE SET NULL,
  name text NOT NULL,
  email text,
  phone text,
  title text,
  is_primary boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_contacts_client ON contacts (client_id);

CREATE TABLE device_assets (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  client_id uuid NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
  site_id uuid REFERENCES sites(id) ON DELETE SET NULL,
  tag text NOT NULL,
  name text NOT NULL,
  kind text NOT NULL DEFAULT 'device',
  status text NOT NULL DEFAULT 'active',
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_assets_client ON device_assets (client_id);

ALTER TABLE clients ENABLE ROW LEVEL SECURITY;
CREATE POLICY clients_org ON clients USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE sites ENABLE ROW LEVEL SECURITY;
CREATE POLICY sites_org ON sites USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE contacts ENABLE ROW LEVEL SECURITY;
CREATE POLICY contacts_org ON contacts USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE device_assets ENABLE ROW LEVEL SECURITY;
CREATE POLICY device_assets_org ON device_assets USING (org_id = current_setting('app.org_id', true)::uuid);
