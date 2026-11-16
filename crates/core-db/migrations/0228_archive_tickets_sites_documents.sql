-- Migration 0228: archive state for tickets, sites and documents.
--
-- Links and panels can name a ticket, site or document, but none of the three
-- could be retired without deleting it, so there was no state a link-hiding
-- rule could test. `archived_at` is that state: the row stays readable by id
-- and keeps explaining the links, history and invoices that name it, while
-- lists, boards, pickers and search hide it by default.
--
--   * archived_at NULL — live record, the default for every existing row.
--     There is deliberately no backfill.
--   * archived_at set — archived. Written once with COALESCE(archived_at, now())
--     so a second archive keeps the first instant, and cleared to NULL on
--     restore, the same shape project_templates uses.
--
-- Compat: additive (policy §4.1) — nullable columns, no destructive
-- statements, no backfill required.

ALTER TABLE tickets
  ADD COLUMN archived_at timestamptz;

ALTER TABLE sites
  ADD COLUMN archived_at timestamptz;

ALTER TABLE documents
  ADD COLUMN archived_at timestamptz;
