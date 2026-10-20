-- Event-type registry (the private build tracker Wave 43 lane 2, `p43-event-type-registry`).
--
-- Replaces the hardcoded `KNOWN_EVENT_TYPES` const in webhook_admin.rs with
-- a DB-backed registry. Two tables:
--
-- `event_types` — the global catalogue of known event types. Sources:
--   'platform'  — emitted by Marula itself (seeded below); always valid, no grant needed.
--   'plugin'    — emitted by a plugin install; valid for the org that installed it.
--   'external'  — explicitly user-registered ad-hoc types (escape hatch for
--                 niche webhook sources); require an org-level grant to use.
-- This table is global (no org_id) — it defines WHAT event types exist, not
-- WHO may subscribe to them. It carries NO RLS policy by design.
--
-- `event_type_grants` — deny-by-default org-level permit table. A (org, event_type)
-- row grants that org permission to reference the event type in a subscription.
-- Platform types are exempt (checked in Rust against source='platform'). All
-- other types — plugin-contributed, external, or merely unknown — require a
-- grant row or the subscription create/update is rejected.
-- Mirrors the deny-by-default posture of `mcp_capability_grants` (migration
-- 0040).

CREATE TABLE event_types (
    event_type text PRIMARY KEY,
    -- 'platform' | 'plugin' | 'external'
    source text NOT NULL CHECK (source IN ('platform', 'plugin', 'external')),
    label text NOT NULL,
    description text,
    -- FK to the plugin install that contributes this type (NULL for
    -- platform/seed types). The plugins table is created later in the
    -- migration sequence so this reference is validated by the FK, not
    -- by position.
    plugin_install_id uuid REFERENCES plugin_installs(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX idx_event_types_source ON event_types (source);
CREATE INDEX idx_event_types_plugin ON event_types (plugin_install_id);

CREATE TABLE event_type_grants (
    org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    -- NOT a FK to event_types — the grant IS the escape hatch for types
    -- not in the registry. A grant row for a bare string is sufficient
    -- to make that type subscribable for the org, matching the spec:
    -- "an unknown type is rejected unless the org holds a grant for it."
    event_type text NOT NULL,
    -- Nullable so that grants created during bootstrap, dev reset, or
    -- plugin sync (no human actor) still persist; mirrors plugin_installs'
    -- installed_by NULLability (migration 0076).
    granted_by uuid REFERENCES users(id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (org_id, event_type)
);

CREATE INDEX idx_event_type_grants_org ON event_type_grants (org_id);

-- RLS: every user-data table with org_id must isolate by org. Grant
-- lookups run inside a set_org transaction so the policy filters to the
-- current org automatically.
ALTER TABLE event_type_grants ENABLE ROW LEVEL SECURITY;
CREATE POLICY event_type_grants_org ON event_type_grants
    USING (org_id = current_setting('app.org_id', true)::uuid);

-- Backfill the registry: the 14 originally-hardcoded platform types
-- (KNOWN_EVENT_TYPES) plus `alert.ingested` and the lane-1 emitted
-- `webhook.received`. These are always accepted without a grant.
INSERT INTO event_types (event_type, source, label, description) VALUES
    ('ticket.created',         'platform', 'Ticket created',  'A new ticket was created'),
    ('ticket.updated',         'platform', 'Ticket updated',  'A ticket''s details were modified'),
    ('ticket.status_changed',  'platform', 'Status changed',  'A ticket moved to a new status'),
    ('ticket.assigned',         'platform', 'Ticket assigned', 'A ticket was assigned to a user'),
    ('ticket.comment_added',   'platform', 'Comment added',   'A comment was added to a ticket'),
    ('ticket.closed',          'platform', 'Ticket closed',   'A ticket was closed'),
    ('client.created',         'platform', 'Client created',  'A new client was created'),
    ('client.updated',         'platform', 'Client updated',  'A client''s details were modified'),
    ('timer.started',          'platform', 'Timer started',   'A time-entry timer was started'),
    ('timer.stopped',          'platform', 'Timer stopped',   'A time-entry timer was stopped'),
    ('time_entry.created',     'platform', 'Time entry created', 'A time entry was recorded'),
    ('notification.created',   'platform', 'Notification created', 'A notification was sent'),
    ('sla.escalation_fired',   'platform', 'SLA escalation',  'An SLA breach triggered an escalation'),
    ('webhook.test',           'platform', 'Webhook test',    'A manual webhook test-fire event'),
    ('alert.ingested',         'platform', 'Alert ingested',  'An external alert source delivered a payload (lane 1)'),
    ('webhook.ingested',       'platform', 'Webhook ingested', 'An inbound webhook payload was ingested (lane 1)'),
    ('webhook.received',       'platform', 'Webhook received', 'An inbound webhook payload was received (lane 1)');
