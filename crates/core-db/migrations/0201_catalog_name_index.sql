-- Migration 0201: the index catalogue matching reads through.
--
-- A describing draft matches its lines against `catalog_items` by exact name
-- first and then by a word-overlap search. The word-overlap leg is a
-- `lower(name)` scan under the org's RLS scope, and without an index that is a
-- sequential pass over the org's whole catalogue on every authoring call.
--
-- Partial on `active`, because matching a draft to an archived item would put
-- a retired price on a new quote.
--
-- Compat: additive (policy §4.1) — one index.

CREATE INDEX idx_catalog_items_name_lower_active
  ON catalog_items (org_id, lower(name))
  WHERE active;
