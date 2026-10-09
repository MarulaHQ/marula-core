-- The plugin outbound posture, the two executor surface kinds, and the emit seam
-- (TASKLIST Wave 43 lane 4, `p43-plugin-webhook-surface`; head was 0125).
--
-- THREE changes, each additive in effect:
--
-- (A) `webhooks` gains an OWNER. `connector_profile.webhook_endpoints` was
--     consent decoration: it created no subscription rows and recorded no
--     owner, so nothing could tear it down. A consented connector's declared
--     endpoints now seed REAL outbound subscription rows attributed to the
--     install, which makes them tear-down-able: an uninstall fires
--     `ON DELETE CASCADE`, and a revoke deletes them explicitly beside the
--     `plugin_installs` status flip in `core_db::plugins::revoke_catalog_entry`.
--
-- (B) `plugin_surface_entries.surface` admits the two EXECUTOR kinds a
--     connector may contribute — `automation_trigger` and `read`. The two UI
--     kinds (`widget`, `report_render`) stay wasm-only, per ADR 0017 §3: a
--     connector has no in-product UI surface to govern. The Rust parse arm
--     (`core-db/src/plugin_surface.rs::parse_surface`) and this CHECK land
--     in the SAME migration on purpose — the parse arm runs on READ and
--     would 409 on a row the CHECK had admitted.
--
-- (C) The emit seam needs NO schema of its own. `plugin_installs.
--     granted_capabilities` is JSONB carrying only a type CHECK
--     (0076:110), so the closed capability vocabulary is enforced in Rust
--     (`PluginCapability::parse` / `ALL`) rather than here. A plugin's
--     declared emitted event types validate against lane 2's registry
--     (`event_types`, migration 0124) at grant time, never here.
--
-- No new table, so no new RLS policy: `webhooks` and `plugin_surface_entries`
-- both keep their existing org-isolating policies (0020 / 0109), and
-- `assert_rls_coverage` (core-db/src/lib.rs) is unaffected by a new
-- `org_id`-keyed column on a table that already has one.

-- ============================================================================
-- (A) the install attribution on an outbound subscription
-- ============================================================================

-- NULL for a human-created subscription; set for one a connector's declared
-- endpoints seeded. `ON DELETE CASCADE` covers the uninstall path — the same
-- posture `event_types.plugin_install_id` already uses (0124).
ALTER TABLE webhooks
    ADD COLUMN owner_install_id uuid
    REFERENCES plugin_installs(id) ON DELETE CASCADE;

-- Backs the idempotent re-seed (`INSERT ... ON CONFLICT DO NOTHING`): a
-- re-consent that still declares the same endpoint must not fork a second
-- row. UNIQUE on (org, install, url) is the narrowest guarantee that
-- permits an admin to keep several distinct endpoints per connector.
--
-- PARTIAL, so human-created rows are unconstrained. Postgres treats NULLs
-- as distinct in a unique index, so a second NULL-bearing row never
-- conflicts — existing subscriptions behave exactly as before.
CREATE UNIQUE INDEX webhooks_owner_endpoint_uk
    ON webhooks (org_id, owner_install_id, url)
    WHERE owner_install_id IS NOT NULL;

-- The install-detail and provenance walks: every row one install owns.
CREATE INDEX webhooks_owner_install_idx
    ON webhooks (org_id, owner_install_id)
    WHERE owner_install_id IS NOT NULL;

-- ============================================================================
-- (B) the executor surface kinds
-- ============================================================================

-- A CHECK cannot be widened in place in Postgres, so the named constraint is
-- dropped and re-added over the widened value list. Rows are untouched: every
-- value already stored ('widget' | 'automation_action' | 'report_render' |
-- 'field_validator') remains admitted. The repo fixes the same shape of
-- relaxation the same way — see 0116 (calendar providers) and 0120
-- (time-entry actions), both allowlisted in scripts/check-migrations.sh.
ALTER TABLE plugin_surface_entries
    DROP CONSTRAINT IF EXISTS plugin_surface_entries_surface_check;

ALTER TABLE plugin_surface_entries
    ADD CONSTRAINT plugin_surface_entries_surface_check
    CHECK (surface IN
        ('widget', 'automation_action', 'report_render', 'field_validator',
         'automation_trigger', 'read'));

COMMENT ON COLUMN plugin_surface_entries.surface IS
    'widget/report_render are wasm-only (ADR 0017 §3, UI coherence); '
    'automation_action/automation_trigger/read are executor kinds a connector '
    'may contribute under the matching install grant';
