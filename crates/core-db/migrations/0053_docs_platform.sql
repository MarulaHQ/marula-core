-- Docs-platform v1 (Wave 11 lane 7 `p11-money-docs-ui`, 2026-09-10): the
-- MSP documentation constructs on top of the D4 version-history foundation
-- (0032). Four adds, all org-scoped and RLS-gated:
--
--   * documents gain SCOPE + STRUCTURE: a nullable `client_id` (NULL =
--   org-global document, the pre-0053 semantics preserved) and a typed
--   layout (`layout_id` + `fields` jsonb validated against the layout's
--   field schema at the app layer).
--
--   * document_layouts — org-managed document/asset types with custom
--   field schemas. System layouts (`is_system`) are the seeded standard
--   MSP set (server, workstation, firewall, network device, application,
--   location, contact, SOP, cert/domain, backup job); they can be edited
--   but not deleted, so imports/integrations can rely on them.
--
--   * document_links — cross-entity relationships (IT Glue-style
--   linking): any document or password can link to a document, password,
--   client, ticket, or KB article. No cascade deletes on targets: links
--   to vanished targets are surfaced as broken and removable, never
--   silently dropped.
--
--   * passwords + password_history — the staff password vault (org or
--   client scoped). Secrets are stored ONLY as AES-256-GCM ciphertext
--   sealed by the app (core-api/src/crypto.rs; same posture as OIDC/TOTP
--   secrets) — the database never sees a plaintext credential. Every
--   change appends the previous ciphertext to password_history (audit
--   posture: history rows are never updated or deleted by app code).
--   Plaintext exists only in the audited `reveal` response.
--
-- KB client-facing flags already exist (0031) and need nothing here.

-- 1. scope + structure on documents --------------------------------------
ALTER TABLE documents
  ADD COLUMN client_id uuid REFERENCES clients(id) ON DELETE SET NULL,
  ADD COLUMN layout_id uuid,
  ADD COLUMN fields jsonb NOT NULL DEFAULT '{}';

-- 2. document layouts -----------------------------------------------------
CREATE TABLE document_layouts (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  slug text NOT NULL,
  name text NOT NULL,
  description text NOT NULL DEFAULT '',
  icon text NOT NULL DEFAULT 'file',
  -- array of {key, label, type, required, position, options?}; type is
  -- one of text|number|date|bool|select|url|email. App-validated.
  fields jsonb NOT NULL DEFAULT '[]',
  is_system boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, slug)
);

ALTER TABLE documents
  ADD CONSTRAINT documents_layout_fk
  FOREIGN KEY (layout_id) REFERENCES document_layouts(id) ON DELETE SET NULL;

CREATE INDEX idx_documents_org_client ON documents (org_id, client_id, updated_at DESC);
CREATE INDEX idx_documents_org_layout ON documents (org_id, layout_id);

-- 3. cross-entity relationship links --------------------------------------
CREATE TABLE document_links (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  from_kind text NOT NULL CHECK (from_kind IN ('document', 'password')),
  from_id uuid NOT NULL,
  to_kind text NOT NULL CHECK (to_kind IN ('document', 'password', 'client', 'ticket', 'kb_article')),
  to_id uuid NOT NULL,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (from_kind, from_id, to_kind, to_id)
);

CREATE INDEX idx_document_links_from ON document_links (org_id, from_kind, from_id);
CREATE INDEX idx_document_links_to ON document_links (org_id, to_kind, to_id);

-- 4. password vault -------------------------------------------------------
CREATE TABLE passwords (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  client_id uuid REFERENCES clients(id) ON DELETE CASCADE,
  title text NOT NULL,
  username text NOT NULL DEFAULT '',
  url text NOT NULL DEFAULT '',
  secret_ciphertext bytea NOT NULL,
  notes_ciphertext bytea,
  expires_at timestamptz,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  updated_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE password_history (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  password_id uuid NOT NULL REFERENCES passwords(id) ON DELETE CASCADE,
  secret_ciphertext bytea NOT NULL,
  changed_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX idx_passwords_org_client ON passwords (org_id, client_id, updated_at DESC);
CREATE INDEX idx_password_history_pw ON password_history (org_id, password_id, created_at DESC);

-- 5. RLS -------------------------------------------------------------------
ALTER TABLE document_layouts ENABLE ROW LEVEL SECURITY;
CREATE POLICY document_layouts_org ON document_layouts
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE document_links ENABLE ROW LEVEL SECURITY;
CREATE POLICY document_links_org ON document_links
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE passwords ENABLE ROW LEVEL SECURITY;
CREATE POLICY passwords_org ON passwords
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE password_history ENABLE ROW LEVEL SECURITY;
CREATE POLICY password_history_org ON password_history
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- 6. seed the standard system layouts for existing orgs --------------------
-- New orgs get the same set via seed_default_document_layouts (orgs.rs).
INSERT INTO document_layouts (id, org_id, slug, name, description, icon, fields, is_system)
SELECT gen_random_uuid(), o.id, d.slug, d.name, d.description, d.icon, d.fields::jsonb, true
FROM orgs o
CROSS JOIN (VALUES
  ('server', 'Server', 'Physical or virtual server', 'server',
   '[{"key":"hostname","label":"Hostname","type":"text","required":true,"position":0},
     {"key":"ip_address","label":"IP address","type":"text","position":1},
     {"key":"os","label":"Operating system","type":"text","position":2},
     {"key":"make_model","label":"Make / model","type":"text","position":3},
     {"key":"serial","label":"Serial number","type":"text","position":4},
     {"key":"warranty_end","label":"Warranty end","type":"date","position":5},
     {"key":"eol_date","label":"EOL date","type":"date","position":6},
     {"key":"virtualization_host","label":"Virtualization host","type":"bool","position":7}]'),
  ('workstation', 'Workstation', 'Desktop or laptop endpoint', 'laptop',
   '[{"key":"hostname","label":"Hostname","type":"text","required":true,"position":0},
     {"key":"assigned_user","label":"Assigned user","type":"text","position":1},
     {"key":"os","label":"Operating system","type":"text","position":2},
     {"key":"make_model","label":"Make / model","type":"text","position":3},
     {"key":"serial","label":"Serial number","type":"text","position":4},
     {"key":"warranty_end","label":"Warranty end","type":"date","position":5}]'),
  ('firewall', 'Firewall', 'Perimeter firewall / UTM', 'shield',
   '[{"key":"hostname","label":"Hostname","type":"text","required":true,"position":0},
     {"key":"wan_ip","label":"WAN address","type":"text","position":1},
     {"key":"lan_ip","label":"LAN address","type":"text","position":2},
     {"key":"firmware","label":"Firmware","type":"text","position":3},
     {"key":"support_end","label":"Support end","type":"date","position":4}]'),
  ('network-device', 'Network device', 'Switch, AP, or other LAN gear', 'network',
   '[{"key":"hostname","label":"Hostname","type":"text","required":true,"position":0},
     {"key":"ip_address","label":"IP address","type":"text","position":1},
     {"key":"location","label":"Location","type":"text","position":2},
     {"key":"make_model","label":"Make / model","type":"text","position":3},
     {"key":"ports","label":"Ports","type":"number","position":4}]'),
  ('application', 'Application', 'SaaS or line-of-business application', 'app-window',
   '[{"key":"vendor","label":"Vendor","type":"text","position":0},
     {"key":"url","label":"URL","type":"url","position":1},
     {"key":"license_count","label":"License count","type":"number","position":2},
     {"key":"renewal_date","label":"Renewal date","type":"date","position":3},
     {"key":"admin_contact","label":"Admin contact","type":"email","position":4}]'),
  ('location', 'Location', 'Client site or office', 'map-pin',
   '[{"key":"address","label":"Address","type":"text","required":true,"position":0},
     {"key":"primary_contact","label":"Primary contact","type":"text","position":1},
     {"key":"access_hours","label":"Access hours","type":"text","position":2},
     {"key":"gate_code_ref","label":"Access code (see vault)","type":"text","position":3}]'),
  ('contact', 'Contact record', 'Third-party contact (vendor, carrier, landlord)', 'user',
   '[{"key":"role","label":"Role","type":"text","required":true,"position":0},
     {"key":"phone","label":"Phone","type":"text","position":1},
     {"key":"email","label":"Email","type":"email","position":2},
     {"key":"company","label":"Company","type":"text","position":3}]'),
  ('sop', 'SOP', 'Standard operating procedure / runbook', 'book',
   '[{"key":"applies_to","label":"Applies to","type":"text","position":0},
     {"key":"owner","label":"Owner","type":"text","position":1},
     {"key":"last_reviewed","label":"Last reviewed","type":"date","position":2}]'),
  ('certificate-domain', 'Certificate / domain', 'TLS certificate or domain registration', 'certificate',
   '[{"key":"domain","label":"Domain","type":"text","required":true,"position":0},
     {"key":"registrar","label":"Registrar","type":"text","position":1},
     {"key":"expires","label":"Expires","type":"date","position":2},
     {"key":"auto_renew","label":"Auto-renew","type":"bool","position":3}]'),
  ('backup-job', 'Backup job', 'Backup configuration for a system or site', 'database',
   '[{"key":"target","label":"What is backed up","type":"text","required":true,"position":0},
     {"key":"schedule","label":"Schedule","type":"text","position":1},
     {"key":"retention","label":"Retention","type":"text","position":2},
     {"key":"destination","label":"Destination","type":"text","position":3},
     {"key":"last_tested","label":"Restore last tested","type":"date","position":4}]')
) AS d(slug, name, description, icon, fields)
ON CONFLICT DO NOTHING;
