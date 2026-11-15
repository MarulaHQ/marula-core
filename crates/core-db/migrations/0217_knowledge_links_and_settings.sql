-- Relationships between knowledge records, and the per-client hub settings
-- (surface-pack lane, migration 0217).
--
-- Purpose: two things the client-first hub needs.
--
-- 1. `knowledge_links` is the generic edge. `document_links` and
--    `record_links` each carry a deliberately narrow pair vocabulary; a rack
--    linking to a subnet, a domain linking to a certificate secret, and a
--    procedure linking to an asset share no vocabulary with either. The kind
--    columns stay open text with a CHECK list so a new record type is one
--    migration, not a rewrite of an existing table's constraint.
--
-- 2. `knowledge_client_settings` holds what makes a hub a hub rather than a
--    count: who owns this client's documentation, which documents are pinned,
--    and the free-text review note the next technician reads first. One row
--    per (org, client); absence means "no owner set", never a null hub.
--
-- Every list index leads with (org_id, client_id). RLS on both tables.

CREATE TABLE knowledge_links (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  client_id uuid REFERENCES clients(id) ON DELETE CASCADE,
  from_kind text NOT NULL CHECK (from_kind IN (
    'rack', 'rack_device', 'network', 'subnet', 'domain', 'procedure',
    'document', 'kb_article', 'asset', 'secret', 'ticket', 'site'
  )),
  from_id uuid NOT NULL,
  to_kind text NOT NULL CHECK (to_kind IN (
    'rack', 'rack_device', 'network', 'subnet', 'domain', 'procedure',
    'document', 'kb_article', 'asset', 'secret', 'ticket', 'site'
  )),
  to_id uuid NOT NULL,
  label text,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  CHECK (from_kind <> to_kind OR from_id <> to_id),
  UNIQUE (org_id, from_kind, from_id, to_kind, to_id)
);

CREATE INDEX idx_knowledge_links_from
  ON knowledge_links (org_id, from_kind, from_id);
CREATE INDEX idx_knowledge_links_to
  ON knowledge_links (org_id, to_kind, to_id);
CREATE INDEX idx_knowledge_links_client
  ON knowledge_links (org_id, client_id);

CREATE TABLE knowledge_client_settings (
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  client_id uuid NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
  owner_user_id uuid REFERENCES users(id) ON DELETE SET NULL,
  pinned_document_ids uuid[] NOT NULL DEFAULT '{}',
  review_note text,
  updated_by uuid REFERENCES users(id) ON DELETE SET NULL,
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (org_id, client_id)
);

ALTER TABLE knowledge_links ENABLE ROW LEVEL SECURITY;
CREATE POLICY knowledge_links_org ON knowledge_links
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE knowledge_client_settings ENABLE ROW LEVEL SECURITY;
CREATE POLICY knowledge_client_settings_org ON knowledge_client_settings
  USING (org_id = current_setting('app.org_id', true)::uuid);
