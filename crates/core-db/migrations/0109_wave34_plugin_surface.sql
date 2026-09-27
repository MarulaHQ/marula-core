-- Plugin surface registry (Wave 34 lane 4, `p34-plugin-surface-closeout`;
-- migration 0109 is this lane's exclusive window; head was 0108).
--
-- Purpose: the §4.6 extension half — PERSISTED registrations of what an
-- installed plugin contributes to first-party surfaces:
--
--   * `widget`            — dashboard / ticket-pane widgets (declarative:
--                           closed render kind + JSON-Schema-validated
--                           props; the core renders, never plugin code —
--                           agent-safety §4.8)
--   * `automation_action` — plugin-provided automation ACTION KINDS
--                           (typed params + a consented built-in effect)
--   * `report_render`     — render kinds contributed to the report builder
--   * `field_validator`   — JSON-Schema validators for board custom fields
--
-- Rows are DERIVED from the pinned manifest's `surface_profile` at
-- approve/enable/grant time (plugin-host syncs them); they are data, not
-- code. An entry only exists while its capability is granted and the
-- install is not revoked — the sync deletes stale rows on downgrade,
-- revoke, or uninstall. Definitions carry the full validated declaration
-- JSON so the consent screen and the consuming surfaces render from ONE
-- source of truth.
--
-- Additive only: one new table + one scoped constraint relaxation.
-- Nothing existing is rewritten or dropped.
--
-- Cross-lane fix (user authorization 2026-09-27, same lane window): the
-- reply-snippet library (Wave 34 lane 3, no migration of its own) stores
-- snippets as JSON OBJECTS in `settings.value`, but the 0018-era
-- `settings_value_check` admits only scalars — every snippet write 409s
-- on a migrations-fresh database (`reply_snippets_flow` proves it). The
-- typed-registry doctrine validates values in core-domain; the DB check
-- stays deny-by-default for every OTHER namespace by admitting objects
-- only for `reply_snippet:*` keys.

ALTER TABLE settings DROP CONSTRAINT settings_value_check;
ALTER TABLE settings ADD CONSTRAINT settings_value_check
  CHECK (jsonb_typeof(value) IN ('string', 'number', 'boolean')
         OR (jsonb_typeof(value) = 'object' AND key LIKE 'reply\_snippet:%'));

CREATE TABLE plugin_surface_entries (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  install_id uuid NOT NULL REFERENCES plugin_installs(id) ON DELETE CASCADE,
  -- Closed surface vocabulary (typed reason-code registry consumes these).
  surface text NOT NULL CHECK (surface IN
    ('widget', 'automation_action', 'report_render', 'field_validator')),
  -- `plugin.<install-slug>.<local-id>` — collision-free across installs.
  entry_key text NOT NULL CHECK (entry_key ~ '^plugin\.[a-z0-9_-]+\.[a-z][a-z0-9_]{0,63}$'),
  label text NOT NULL CHECK (char_length(label) BETWEEN 1 AND 120),
  -- The validated declaration (widget/action/render-kind/validator shape
  -- per surface). Rendered verbatim on consent and consumed by the
  -- surfaces; never executable content.
  definition jsonb NOT NULL,
  enabled boolean NOT NULL DEFAULT true,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, install_id, surface, entry_key)
);

CREATE INDEX idx_plugin_surface_entries_org_surface
  ON plugin_surface_entries (org_id, surface) WHERE enabled;
CREATE INDEX idx_plugin_surface_entries_install
  ON plugin_surface_entries (org_id, install_id);

ALTER TABLE plugin_surface_entries ENABLE ROW LEVEL SECURITY;
CREATE POLICY plugin_surface_entries_org ON plugin_surface_entries
  USING (org_id = current_setting('app.org_id', true)::uuid);
