-- Custom fields: JSONB values + field-definition tables (§5).

CREATE TABLE custom_field_defs (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  entity text NOT NULL CHECK (entity IN ('ticket','client','contact','asset')),
  name text NOT NULL,
  kind text NOT NULL CHECK (kind IN ('text','number','date','select','multiselect','bool','url','user')),
  config jsonb NOT NULL DEFAULT '{}',
  required boolean NOT NULL DEFAULT false,
  sort_order integer NOT NULL DEFAULT 0,
  archived_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, entity, name)
);

CREATE TABLE custom_field_values (
  field_id uuid NOT NULL REFERENCES custom_field_defs(id) ON DELETE CASCADE,
  entity_id uuid NOT NULL,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  value jsonb NOT NULL,
  PRIMARY KEY (field_id, entity_id)
);

ALTER TABLE custom_field_defs ENABLE ROW LEVEL SECURITY;
CREATE POLICY custom_field_defs_org ON custom_field_defs USING (org_id = current_setting('app.org_id', true)::uuid);
ALTER TABLE custom_field_values ENABLE ROW LEVEL SECURITY;
CREATE POLICY custom_field_values_org ON custom_field_values USING (org_id = current_setting('app.org_id', true)::uuid);
