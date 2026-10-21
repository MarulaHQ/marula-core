-- Schema for the vector ANN benchmark.
--
-- Mirrors the two production tables the search vector leg touches, at the
-- columns that leg reads, so the measured query is the product's query with
-- no extra joins:
--   search_index      crates/core-db/migrations/0023_search_v1.sql:12-38
--   search_embeddings crates/core-db/migrations/0062_search_vector.sql:33-57
--
-- RLS is enabled with the same policy the migrations install, and the
-- benchmark role is neither the table owner nor a superuser. A measurement
-- taken as the owner would bypass the policy and test nothing, which is the
-- one way this fixture can lie.

DROP SCHEMA IF EXISTS bench CASCADE;

DROP TABLE IF EXISTS search_embeddings CASCADE;
DROP TABLE IF EXISTS search_index CASCADE;
DROP TABLE IF EXISTS orgs CASCADE;

DROP ROLE IF EXISTS marula_app;
CREATE ROLE marula_app NOLOGIN;

CREATE TABLE orgs (id uuid PRIMARY KEY);

CREATE TABLE search_index (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  entity_kind text NOT NULL CHECK (entity_kind IN ('ticket','comment','client','site','contact',
                                                   'kb_article','document')),
  entity_id uuid NOT NULL,
  parent_id uuid,
  title text NOT NULL DEFAULT '',
  body text NOT NULL DEFAULT '',
  client_scope uuid,
  indexed_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, entity_kind, entity_id)
);

CREATE INDEX idx_search_kind ON search_index (org_id, entity_kind);

ALTER TABLE search_index ENABLE ROW LEVEL SECURITY;
CREATE POLICY search_index_org ON search_index
  USING (org_id = current_setting('app.org_id', true)::uuid);

CREATE TABLE search_embeddings (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  entity_kind text NOT NULL CHECK (entity_kind IN ('ticket','comment','client','site','contact',
                                                   'kb_article','document')),
  entity_id uuid NOT NULL,
  model text NOT NULL,
  content_hash text NOT NULL,
  embedding vector(768) NOT NULL,
  embedded_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, entity_kind, entity_id)
);

CREATE INDEX idx_search_embeddings_org_kind ON search_embeddings (org_id, entity_kind);

ALTER TABLE search_embeddings ENABLE ROW LEVEL SECURITY;
CREATE POLICY search_embeddings_org ON search_embeddings
  USING (org_id = current_setting('app.org_id', true)::uuid);

GRANT USAGE ON SCHEMA public TO marula_app;
GRANT SELECT ON orgs, search_index, search_embeddings TO marula_app;

CREATE SCHEMA bench AUTHORIZATION marula_app;

-- Tenant registry. Filled by 01-fixture.sql, read by everything after it, so
-- each stage can run in its own psql session.
CREATE TABLE bench.tenant_ids (label text PRIMARY KEY, id uuid NOT NULL, n int NOT NULL);
GRANT SELECT ON bench.tenant_ids TO marula_app;

-- One row per timed iteration.
SET ROLE marula_app;

CREATE TABLE bench.timings (
  engine text NOT NULL,
  arm text NOT NULL,
  tenant text NOT NULL,
  iter int NOT NULL,
  ms double precision NOT NULL
);
CREATE TABLE bench.builds (
  engine text NOT NULL,
  arm text NOT NULL,
  build_ms double precision NOT NULL,
  size_bytes bigint NOT NULL DEFAULT 0
);
-- Tenant registry. Filled by 01-fixture.sql, read by everything after it.
CREATE TABLE bench.exact_ids (
  tenant text NOT NULL,
  q int NOT NULL,
  ids uuid[] NOT NULL,
  PRIMARY KEY (tenant, q)
);
-- Ground truth for the index's own quality: nearest neighbours with no tenant
-- filter, taken as the table owner. Comparing a global ANN arm against this
-- answers "can the index find the nearest rows at all", which is a different
-- question from "does it find this tenant's rows".
CREATE TABLE bench.exact_global_ids (
  tenant text NOT NULL,
  q int NOT NULL,
  ids uuid[] NOT NULL,
  PRIMARY KEY (tenant, q)
);
CREATE TABLE bench.arm_ids (
  arm text NOT NULL,
  tenant text NOT NULL,
  q int NOT NULL,
  ids uuid[] NOT NULL,
  PRIMARY KEY (arm, tenant, q)
);
CREATE TABLE bench.plans (
  engine text NOT NULL,
  arm text NOT NULL,
  tenant text NOT NULL,
  plan text NOT NULL
);
RESET ROLE;
