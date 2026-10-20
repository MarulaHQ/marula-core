-- Wave 8b KB-import lane (the private build tracker D5).
--
-- Purpose: the import framework's batch / external-id / saved-mapping
-- CHECKs widen ADDITIVELY to the KB families 'kb_category' and
-- 'kb_article' (widening only — 0033's precedent; every previously valid
-- row stays valid). Parent-before-child run order extends to:
-- … → time entries → kb categories → kb articles, so an article's
-- `category_slug` always resolves to a category imported earlier in the
-- same run script.
--
-- Non-purpose: no new tables. KB storage itself shipped in 0031; this only
-- lets the D2 machinery address those rows as import families.

ALTER TABLE import_batches DROP CONSTRAINT import_batches_entity_kind_check;
ALTER TABLE import_batches ADD CONSTRAINT import_batches_entity_kind_check
  CHECK (entity_kind IN ('client','site','contact','board','ticket','time_entry','kb_category','kb_article'));

ALTER TABLE external_ids DROP CONSTRAINT external_ids_entity_kind_check;
ALTER TABLE external_ids ADD CONSTRAINT external_ids_entity_kind_check
  CHECK (entity_kind IN ('client','site','contact','board','ticket','time_entry','kb_category','kb_article'));

ALTER TABLE import_mapping_templates DROP CONSTRAINT import_mapping_templates_entity_kind_check;
ALTER TABLE import_mapping_templates ADD CONSTRAINT import_mapping_templates_entity_kind_check
  CHECK (entity_kind IN ('client','site','contact','board','ticket','time_entry','kb_category','kb_article'));
