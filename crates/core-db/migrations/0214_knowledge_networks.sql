-- Network records: sites, subnets, VLANs and IP reservations (surface-pack
-- lane, migration 0214).
--
-- Purpose: Hudu-parity IPAM at the depth a technician needs during an
-- incident — which network, which subnet, which VLAN, which gateway, and
-- which address is already spoken for. A network belongs to a client and
-- optionally to a site; a subnet belongs to a network; a reservation belongs
-- to a subnet and may name the asset that holds the address.
--
-- Non-purpose: no live discovery and no DHCP server. Radar-style scanning is
-- an add-on product, and this is the record it would write into. `cidr` and
-- `inet` are Postgres types on purpose: a subnet that is not a subnet should
-- be refused by the database, not by a regular expression in a form.
--
-- Every list index leads with (org_id, client_id); RLS on every table.

CREATE TABLE knowledge_networks (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  client_id uuid NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
  site_id uuid REFERENCES sites(id) ON DELETE SET NULL,
  name text NOT NULL,
  kind text NOT NULL DEFAULT 'lan'
    CHECK (kind IN ('lan', 'wan', 'wifi', 'vpn', 'voice', 'guest', 'other')),
  gateway text,
  dns text,
  vlan_zone text,
  notes text,
  fields jsonb NOT NULL DEFAULT '{}',
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  updated_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, client_id, name)
);

CREATE INDEX idx_knowledge_networks_scope
  ON knowledge_networks (org_id, client_id, name);

CREATE TABLE knowledge_subnets (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  client_id uuid NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
  network_id uuid NOT NULL REFERENCES knowledge_networks(id) ON DELETE CASCADE,
  cidr cidr NOT NULL,
  vlan_id integer CHECK (vlan_id IS NULL OR (vlan_id >= 1 AND vlan_id <= 4094)),
  name text,
  gateway text,
  used_for text,
  dhcp boolean NOT NULL DEFAULT true,
  notes text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (network_id, cidr)
);

CREATE INDEX idx_knowledge_subnets_scope
  ON knowledge_subnets (org_id, client_id);
CREATE INDEX idx_knowledge_subnets_network
  ON knowledge_subnets (org_id, network_id, cidr);

CREATE TABLE knowledge_ip_reservations (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  client_id uuid NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
  subnet_id uuid NOT NULL REFERENCES knowledge_subnets(id) ON DELETE CASCADE,
  address inet NOT NULL,
  hostname text,
  asset_id uuid REFERENCES device_assets(id) ON DELETE SET NULL,
  status text NOT NULL DEFAULT 'assigned'
    CHECK (status IN ('assigned', 'reserved', 'deprecated')),
  notes text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (subnet_id, address)
);

CREATE INDEX idx_knowledge_ip_reservations_scope
  ON knowledge_ip_reservations (org_id, client_id);
CREATE INDEX idx_knowledge_ip_reservations_subnet
  ON knowledge_ip_reservations (org_id, subnet_id, address);

ALTER TABLE knowledge_networks ENABLE ROW LEVEL SECURITY;
CREATE POLICY knowledge_networks_org ON knowledge_networks
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE knowledge_subnets ENABLE ROW LEVEL SECURITY;
CREATE POLICY knowledge_subnets_org ON knowledge_subnets
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE knowledge_ip_reservations ENABLE ROW LEVEL SECURITY;
CREATE POLICY knowledge_ip_reservations_org ON knowledge_ip_reservations
  USING (org_id = current_setting('app.org_id', true)::uuid);
