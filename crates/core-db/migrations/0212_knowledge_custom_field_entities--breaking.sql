-- Knowledge record types join the custom-field vocabulary (surface-pack lane,
-- migration 0212).
--
-- BREAKING-FLAG DISCLOSURE (upgrade-compatibility-policy.md §4.3): this file
-- carries `--breaking` solely because widening a CHECK requires DROP
-- CONSTRAINT, which the migration lint gates. DATA IMPACT: none. The new
-- constraint only ACCEPTS MORE values; no row is rewritten, dropped or
-- retyped, and every existing read is unchanged. It is re-added in the same
-- transaction.
--
-- Purpose: the documentation build-out adds four first-class record types —
-- racks, networks, domains and procedures. Each carries a `fields jsonb`
-- column so a team can extend a record without a migration, and each needs a
-- row in `custom_field_defs` to describe those fields the same way servers,
-- contacts and tickets already do.
--
-- The original CHECK (0009) closed the entity list at four names because only
-- four record types existed. It is widened here, once, to the full shape the
-- store can hold. A CHECK cannot be altered in place, so it is dropped and
-- re-created; no data changes.
--
-- Non-purpose: no new field kinds. The kind vocabulary stays closed, so an
-- existing custom-field form keeps working against a new entity unchanged.

ALTER TABLE custom_field_defs
  DROP CONSTRAINT IF EXISTS custom_field_defs_entity_check;

ALTER TABLE custom_field_defs
  ADD CONSTRAINT custom_field_defs_entity_check
  CHECK (entity IN (
    'ticket', 'client', 'contact', 'asset',
    'rack', 'network', 'domain', 'procedure'
  ));

-- The hub reads a record's custom fields by (org, entity, entity_id); the
-- existing primary key leads on field_id, so add the lookup order the hub
-- actually asks for.
CREATE INDEX IF NOT EXISTS idx_custom_field_values_entity
  ON custom_field_values (org_id, entity_id);
