-- Domain and website records (surface-pack lane, migration 0215).
--
-- Purpose: Hudu's website tracker without its crawler. A record names a
-- domain or a website, its registrar and registrar expiry, its TLS expiry and
-- issuer, where DNS lives, where it is hosted, and whether this team manages
-- the DNS. Expiry rolls up into the client hub and the org-wide expirations
-- view, which is the one thing a technician actually opens this record for.
--
-- Non-purpose: no WHOIS polling and no certificate probe. Those are an
-- add-on's job; this table is what an integration or a person writes into,
-- and every date is annotated with where it came from.
--
-- One row per (client, domain) so a re-import cannot duplicate a domain.

CREATE TABLE knowledge_domains (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  client_id uuid NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
  domain text NOT NULL,
  kind text NOT NULL DEFAULT 'domain'
    CHECK (kind IN ('domain', 'website')),
  registrar text,
  registrar_expiry date,
  ssl_expiry date,
  ssl_issuer text,
  dns_provider text,
  hosting text,
  manages_dns boolean NOT NULL DEFAULT true,
  monitoring boolean NOT NULL DEFAULT false,
  status_url text,
  notes text,
  fields jsonb NOT NULL DEFAULT '{}',
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  updated_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, client_id, domain)
);

CREATE INDEX idx_knowledge_domains_scope
  ON knowledge_domains (org_id, client_id, domain);
-- The expiry sweep reads every date in the org; one index per date column
-- keeps that a range scan rather than a table scan.
CREATE INDEX idx_knowledge_domains_registrar_expiry
  ON knowledge_domains (org_id, registrar_expiry)
  WHERE registrar_expiry IS NOT NULL;
CREATE INDEX idx_knowledge_domains_ssl_expiry
  ON knowledge_domains (org_id, ssl_expiry)
  WHERE ssl_expiry IS NOT NULL;

ALTER TABLE knowledge_domains ENABLE ROW LEVEL SECURITY;
CREATE POLICY knowledge_domains_org ON knowledge_domains
  USING (org_id = current_setting('app.org_id', true)::uuid);
