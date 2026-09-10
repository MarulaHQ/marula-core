-- Migration 0054: asset-type taxonomy on the docs-platform (Wave 11 lane 5).
--
-- Assets reconcile with the docs-platform (0053) instead of introducing a
-- parallel registry:
--   * Asset TYPES ride `document_layouts` (the seeded system set is already
--     asset-shaped: server, workstation, firewall, network-device, ...). No
--     second type/field-schema registry is introduced.
--   * Asset INSTANCES stay in `device_assets` (0002), client-scoped and
--     already RLS'd, gaining typed classification: `layout_id` +
--     schema-validated `fields`, mirroring documents' shape. The free-text
--     `kind`/`status` columns are KEPT for connector compatibility; behavior
--     binds to semantic classes per CI6.
--   * Asset instances link via `document_links`: the kind CHECK constraints
--     gain the `asset` kind. Lane-7 semantics hold — resolved labels,
--     broken targets surfaced + removable, never cascade-deleted.

ALTER TABLE device_assets
  ADD COLUMN layout_id uuid REFERENCES document_layouts(id) ON DELETE SET NULL;
ALTER TABLE device_assets
  ADD COLUMN fields jsonb NOT NULL DEFAULT '{}';

CREATE INDEX idx_device_assets_org_layout ON device_assets (org_id, layout_id);

ALTER TABLE document_links DROP CONSTRAINT document_links_from_kind_check;
ALTER TABLE document_links DROP CONSTRAINT document_links_to_kind_check;
ALTER TABLE document_links
  ADD CONSTRAINT document_links_from_kind_check
  CHECK (from_kind IN ('document', 'password', 'asset'));
ALTER TABLE document_links
  ADD CONSTRAINT document_links_to_kind_check
  CHECK (to_kind IN ('document', 'password', 'client', 'ticket', 'kb_article', 'asset'));