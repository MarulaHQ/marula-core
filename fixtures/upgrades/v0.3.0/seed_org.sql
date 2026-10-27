-- U7 upgrade fixture seed — v0.3.0 (policy §8)
--
-- Purpose: seed ONE production-like org exercising every customization
-- surface the upgrade matrix must carry forward: custom fields (with an
-- archived definition), custom board statuses + explicit transitions (and a
-- board relying on the semantic-category floor), a pre-close checklist
-- template, automation rules, webhook subscriptions with event filters,
-- org- and user-scope settings-registry values, SLA calendars + policies
-- with escalation ladders, stored report definitions + a schedule, KB,
-- documents with version history, contracts/projects, portal configuration,
-- plugin install state (a catalog entry, an enabled install with its digest
-- pin, two surface entries, a health row, KV rows and their usage counter),
-- plus representative operational data (clients/sites/contacts/tickets/time
-- entries/invoicing/alerts/email/import/provenance/agent-platform) so the
-- migrated fixture looks like a real deployment, not a test stub.
--
-- Carried from v0.2.0 unchanged except for the plugin section and the
-- fixture-relative attachment key: the seed describes the shapes a release
-- had, and re-running it against a newer schema is what proves the
-- migrations kept it valid.
--
-- Contract: deterministic fixed UUIDs (single org id, below), fixed
-- timestamps, plain SQL — safe to re-run (idempotent: wipes the fixture org
-- first), applied by an admin connection AFTER migrations (RLS is bypassed
-- as table owner; row shape is identical to production).
--
-- Org id anchor: 00000000-0000-4000-8000-000000000001

BEGIN;

-- Re-run safety: remove any previous fixture org (children cascade).
DELETE FROM orgs WHERE id = '00000000-0000-4000-8000-000000000001';

-- ============================================================ identity ====
INSERT INTO orgs (id, name, slug, ticket_prefix, ticket_sequence, eod_nudges_enabled, eod_nudge_hour, time_gap_min_minutes, invoice_sequence) VALUES
  ('00000000-0000-4000-8000-000000000001', 'Acme IT Services', 'acme-it', 'ACM', 1009, true, 18, 60, 2);

INSERT INTO roles (id, org_id, name, builtin, permissions, is_system) VALUES
  ('00000000-0000-4000-8000-000000000011', '00000000-0000-4000-8000-000000000001', 'Administrator', 'admin', '["*"]', true),
  ('00000000-0000-4000-8000-000000000012', '00000000-0000-4000-8000-000000000001', 'Member', 'member', '["tickets:read","tickets:write","clients:read"]', true),
  ('00000000-0000-4000-8000-000000000013', '00000000-0000-4000-8000-000000000001', 'Dispatcher', NULL, '["tickets:read","tickets:write","dispatch:write"]', false);

INSERT INTO users (id, org_id, email, display_name, password_hash, timezone, is_active, mfa_totp_secret) VALUES
  ('00000000-0000-4000-8000-000000000021', '00000000-0000-4000-8000-000000000001', 'dana@acme-it.example',  'Dana Reyes',      '$argon2id$v=19$m=19456,t=2,p=1$fixtureseed$fixturehashdana',  'America/New_York', true, NULL),
  ('00000000-0000-4000-8000-000000000022', '00000000-0000-4000-8000-000000000001', 'omar@acme-it.example',  'Omar Haddad',     '$argon2id$v=19$m=19456,t=2,p=1$fixtureseed$fixturehashomar',  'America/New_York', true, 'a2VsYWRlZmlyc3R1ZmZpeHRvdHBzZWNyZXQ='),
  ('00000000-0000-4000-8000-000000000023', '00000000-0000-4000-8000-000000000001', 'sam@acme-it.example',   'Sam Okafor',      '$argon2id$v=19$m=19456,t=2,p=1$fixtureseed$fixturehashsam',   'America/Chicago',  true, NULL),
  ('00000000-0000-4000-8000-000000000024', '00000000-0000-4000-8000-000000000001', 'priya@acme-it.example', 'Priya Natarajan', '$argon2id$v=19$m=19456,t=2,p=1$fixtureseed$fixturehashpriya', 'UTC',              true, NULL);

INSERT INTO user_roles (user_id, role_id) VALUES
  ('00000000-0000-4000-8000-000000000021', '00000000-0000-4000-8000-000000000011'),
  ('00000000-0000-4000-8000-000000000022', '00000000-0000-4000-8000-000000000011'),
  ('00000000-0000-4000-8000-000000000023', '00000000-0000-4000-8000-000000000012'),
  ('00000000-0000-4000-8000-000000000024', '00000000-0000-4000-8000-000000000013');

-- SSO + MFA state (auth customization surface)
INSERT INTO oidc_providers (id, org_id, name, display_name, issuer, discovery_url, client_id, client_secret_enc, jit_provisioning, enabled, created_by) VALUES
  ('00000000-0000-4000-8000-000000000031', '00000000-0000-4000-8000-000000000001', 'jumpcloud', 'JumpCloud SSO', 'https://oauth.jumpcloud.com/', 'https://oauth.jumpcloud.com/.well-known/openid-configuration', 'acme-marula-sp', decode('9faf5d0c1b2e4a6780c3d5e7f1902a4b6c8d0e2f4a6b8c0d2e4f6a8b0c2d4e6f','hex'), true, true, '00000000-0000-4000-8000-000000000021');

INSERT INTO oidc_accounts (id, org_id, provider_id, user_id, subject, email, linked_by, linked_at, last_login_at) VALUES
  ('00000000-0000-4000-8000-000000000032', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000031', '00000000-0000-4000-8000-000000000024', 'priya.natarajan@jumpcloud', 'priya@acme-it.example', '00000000-0000-4000-8000-000000000021', '2026-06-15T14:00:00Z', '2026-09-01T09:12:00Z');

INSERT INTO totp_secrets (id, org_id, user_id, secret_enc, confirmed, last_used_step) VALUES
  ('00000000-0000-4000-8000-000000000033', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000022', decode('5d1f0a3c8e2b476d9a0c5e8f1b4a7d0c','hex'), true, 42100000);

INSERT INTO recovery_codes (id, org_id, user_id, code_hash, used_at) VALUES
  ('00000000-0000-4000-8000-000000000034', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000022', 'sha256$fixtureseed$recocodeseed01', NULL),
  ('00000000-0000-4000-8000-000000000035', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000022', 'sha256$fixtureseed$recocodeseed02', '2026-08-14T16:40:00Z');

INSERT INTO auth_sessions (id, user_id, refresh_token_hash, user_agent, expires_at, chain_id) VALUES
  ('00000000-0000-4000-8000-000000000036', '00000000-0000-4000-8000-000000000021', 'sha256$fixtureseed$authsession01', 'Mozilla/5.0 fixture', '2026-10-01T00:00:00Z', '00000000-0000-4000-8000-000000000037');

INSERT INTO api_keys (id, org_id, name, key_hash, prefix, scopes, created_by, last_used_at, revoked_at) VALUES
  ('00000000-0000-4000-8000-000000000041', '00000000-0000-4000-8000-000000000001', 'CI integration',  'argon2$fixtureseed$apikey01', 'mrl_ab12cd34', '["tickets:read","clients:read","time_entries:read"]', '00000000-0000-4000-8000-000000000021', '2026-09-02T02:00:00Z', NULL),
  ('00000000-0000-4000-8000-000000000042', '00000000-0000-4000-8000-000000000001', 'Legacy exporter', 'argon2$fixtureseed$apikey02', 'mrl_ef90ab12', '["clients:read"]', '00000000-0000-4000-8000-000000000021', '2026-07-01T00:00:00Z', '2026-08-01T00:00:00Z');

-- ====================================================== clients / CRM ====
INSERT INTO clients (id, org_id, name, restricted_to, notes) VALUES
  ('00000000-0000-4000-8000-000000000051', '00000000-0000-4000-8000-000000000001', 'Northwind Traders', '[]', 'Flagship MSP account'),
  ('00000000-0000-4000-8000-000000000052', '00000000-0000-4000-8000-000000000001', 'Contoso Manufacturing', '[]', NULL);

INSERT INTO sites (id, org_id, client_id, name, address, city, region, postal_code, country, timezone) VALUES
  ('00000000-0000-4000-8000-000000000054', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000051', 'Northwind HQ', '500 Harbor Blvd', 'Seattle', 'WA', '98104', 'US', 'America/Los_Angeles'),
  ('00000000-0000-4000-8000-000000000055', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000052', 'Contoso Plant 1', '12 Foundry Rd', 'Detroit', 'MI', '48201', 'US', 'America/Detroit');

INSERT INTO contacts (id, org_id, client_id, site_id, name, email, phone, title, is_primary) VALUES
  ('00000000-0000-4000-8000-000000000057', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000051', '00000000-0000-4000-8000-000000000054', 'Lena Fischer', 'lena.fischer@northwind.example', '+1-206-555-0141', 'IT Manager', true),
  ('00000000-0000-4000-8000-000000000058', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000052', '00000000-0000-4000-8000-000000000055', 'Marco Ruiz', 'marco.ruiz@contoso.example', '+1-313-555-0177', 'Plant Systems Lead', true);

INSERT INTO device_assets (id, org_id, client_id, site_id, tag, name, kind, status) VALUES
  ('00000000-0000-4000-8000-000000000059', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000051', '00000000-0000-4000-8000-000000000054', 'NW-FW-01', 'Perimeter firewall', 'device', 'active');

-- ================================= boards / statuses / transitions =======
INSERT INTO board_groups (id, org_id, owner_id, name, position) VALUES
  ('00000000-0000-4000-8000-000000000071', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000021', 'Service Delivery', 0);

INSERT INTO boards (id, org_id, name, description, is_default, inbound_address, require_checklist_close, board_group_id) VALUES
  ('00000000-0000-4000-8000-000000000081', '00000000-0000-4000-8000-000000000001', 'Service Desk', 'Default intake for client requests', true, 'support-8f2k@inbound.acme-it.example', true, '00000000-0000-4000-8000-000000000071'),
  ('00000000-0000-4000-8000-000000000082', '00000000-0000-4000-8000-000000000001', 'Projects Board', 'Non-ticket project work intake', false, NULL, false, '00000000-0000-4000-8000-000000000071');

INSERT INTO board_members (org_id, board_id, user_id, role) VALUES
  ('00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000081', '00000000-0000-4000-8000-000000000023', 'admin'),
  ('00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000082', '00000000-0000-4000-8000-000000000024', 'viewer');

INSERT INTO board_favorites (user_id, board_id, sort_order) VALUES
  ('00000000-0000-4000-8000-000000000021', '00000000-0000-4000-8000-000000000081', 0);

-- Board 1: custom named statuses (each still bound to its semantic
-- category) + explicit transition adjacency.
INSERT INTO board_statuses (id, board_id, org_id, name, category, sort_order, is_entry, color) VALUES
  ('00000000-0000-4000-8000-000000000091', '00000000-0000-4000-8000-000000000081', '00000000-0000-4000-8000-000000000001', 'New',                 'open',        0, true,  '#64748b'),
  ('00000000-0000-4000-8000-000000000092', '00000000-0000-4000-8000-000000000081', '00000000-0000-4000-8000-000000000001', 'Triage',              'in_progress', 1, false, '#0ea5e9'),
  ('00000000-0000-4000-8000-000000000093', '00000000-0000-4000-8000-000000000081', '00000000-0000-4000-8000-000000000001', 'Waiting on Customer', 'waiting',     2, false, '#f59e0b'),
  ('00000000-0000-4000-8000-000000000094', '00000000-0000-4000-8000-000000000081', '00000000-0000-4000-8000-000000000001', 'Resolved',            'closed',      3, false, '#10b981'),
  ('00000000-0000-4000-8000-000000000095', '00000000-0000-4000-8000-000000000081', '00000000-0000-4000-8000-000000000001', 'Scheduled',           'waiting',     4, false, '#8b5cf6');

INSERT INTO board_status_transitions (org_id, board_id, from_status_id, to_status_id) VALUES
  ('00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000081', '00000000-0000-4000-8000-000000000091', '00000000-0000-4000-8000-000000000092'),
  ('00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000081', '00000000-0000-4000-8000-000000000091', '00000000-0000-4000-8000-000000000095'),
  ('00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000081', '00000000-0000-4000-8000-000000000092', '00000000-0000-4000-8000-000000000093'),
  ('00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000081', '00000000-0000-4000-8000-000000000092', '00000000-0000-4000-8000-000000000094'),
  ('00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000081', '00000000-0000-4000-8000-000000000093', '00000000-0000-4000-8000-000000000092'),
  ('00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000081', '00000000-0000-4000-8000-000000000094', '00000000-0000-4000-8000-000000000092'),
  ('00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000081', '00000000-0000-4000-8000-000000000095', '00000000-0000-4000-8000-000000000092');

-- Board 2: custom statuses, NO explicit transitions (category floor).
INSERT INTO board_statuses (id, board_id, org_id, name, category, sort_order, is_entry, color) VALUES
  ('00000000-0000-4000-8000-000000000096', '00000000-0000-4000-8000-000000000082', '00000000-0000-4000-8000-000000000001', 'Intake',   'open',        0, true,  '#64748b'),
  ('00000000-0000-4000-8000-000000000097', '00000000-0000-4000-8000-000000000082', '00000000-0000-4000-8000-000000000001', 'Scoping',  'in_progress', 1, false, '#0ea5e9'),
  ('00000000-0000-4000-8000-000000000098', '00000000-0000-4000-8000-000000000082', '00000000-0000-4000-8000-000000000001', 'On Hold',  'waiting',     2, false, '#f59e0b'),
  ('00000000-0000-4000-8000-000000000099', '00000000-0000-4000-8000-000000000082', '00000000-0000-4000-8000-000000000001', 'Delivered','closed',      3, false, '#10b981');

-- ================================================ custom fields ==========
INSERT INTO custom_field_defs (id, org_id, entity, name, kind, config, required, sort_order, archived_at, board_id) VALUES
  ('00000000-0000-4000-8000-000000000101', '00000000-0000-4000-8000-000000000001', 'ticket', 'Environment',        'select', '{"options":["Production","Staging","Development"]}', true,  0, NULL, '00000000-0000-4000-8000-000000000081'),
  ('00000000-0000-4000-8000-000000000102', '00000000-0000-4000-8000-000000000001', 'ticket', 'Business Impact',    'text',   '{"max_length":280}', true, 1, NULL, NULL),
  ('00000000-0000-4000-8000-000000000103', '00000000-0000-4000-8000-000000000001', 'ticket', 'Estimated Hours',    'number', '{"min":0,"max":500,"step":0.5}', false, 2, NULL, NULL),
  ('00000000-0000-4000-8000-000000000104', '00000000-0000-4000-8000-000000000001', 'client', 'Contract Tier',      'select', '{"options":["Bronze","Silver","Gold"]}', false, 0, NULL, NULL),
  ('00000000-0000-4000-8000-000000000105', '00000000-0000-4000-8000-000000000001', 'contact','Preferred Language', 'select', '{"options":["English","Spanish","German"]}', false, 0, NULL, NULL),
  ('00000000-0000-4000-8000-000000000106', '00000000-0000-4000-8000-000000000001', 'asset',  'Warranty Expiry',    'date',   '{}', false, 0, NULL, NULL),
  ('00000000-0000-4000-8000-000000000107', '00000000-0000-4000-8000-000000000001', 'ticket', 'Legacy Source ID',   'text',   '{}', false, 3, '2026-05-01T00:00:00Z', NULL);

-- ================================================= checklist template ====
INSERT INTO board_checklist_templates (id, org_id, board_id, name) VALUES
  ('00000000-0000-4000-8000-000000000111', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000081', 'Resolution checklist');

INSERT INTO checklist_template_items (id, org_id, template_id, text, sort_order) VALUES
  ('00000000-0000-4000-8000-000000000112', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000111', 'Root cause documented',        0),
  ('00000000-0000-4000-8000-000000000113', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000111', 'KB article linked or created', 1),
  ('00000000-0000-4000-8000-000000000114', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000111', 'Client notified',              2);

-- ===================================================== automations =======
INSERT INTO automation_rules (id, org_id, board_id, name, trigger_event, conditions, actions, enabled) VALUES
  ('00000000-0000-4000-8000-000000000121', '00000000-0000-4000-8000-000000000001', NULL, 'P1 pages on-call', 'ticket.created',
   '[{"field":"priority","op":"equals","value":"p1"}]',
   '[{"type":"assign","user_id":"00000000-0000-4000-8000-000000000023"},{"type":"notify","kind":"sla_risk","user_ids":["00000000-0000-4000-8000-000000000022"]}]', true),
  ('00000000-0000-4000-8000-000000000122', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000081', 'Waiting-state watchdog', 'ticket.status_changed',
   '[{"field":"to_category","op":"equals","value":"waiting"}]',
   '[{"type":"add_comment","body":"Flagged as waiting — remember to follow up."}]', true),
  ('00000000-0000-4000-8000-000000000123', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000082', 'Stale intake reminder', 'ticket.commented',
   '[{"field":"category","op":"equals","value":"open"}]',
   '[{"type":"notify","kind":"ticket_update","user_ids":["00000000-0000-4000-8000-000000000024"]}]', false);

-- ====================================================== webhooks =========
INSERT INTO webhooks (id, org_id, url, secret_encrypted, event_types, enabled, created_by) VALUES
  ('00000000-0000-4000-8000-000000000131', '00000000-0000-4000-8000-000000000001', 'https://hooks.northwind.example/marula/tickets', decode('3f8a1c2d5e7b90a1c3d5e7f90a1b3c5d7e9f0a2b4c6d8e0f2a4b6c8d0e2f4a6b','hex'), '{ticket.created,ticket.status_changed}', true, '00000000-0000-4000-8000-000000000021'),
  ('00000000-0000-4000-8000-000000000132', '00000000-0000-4000-8000-000000000001', 'https://ops.acme-it.example/webhooks/wildcard',    decode('7b2d9f4e1a3c5b7d9f0e2a4c6b8d0f2a4c6e8b0d2f4a6c8e0b2d4f6a8c0e2b4d','hex'), '{}', false, '00000000-0000-4000-8000-000000000021');

-- ====================================================== settings =========
INSERT INTO settings (id, org_id, scope, key, owner_id, value, updated_by) VALUES
  ('00000000-0000-4000-8000-000000000141', '00000000-0000-4000-8000-000000000001', 'org',  'org.display_name',          '00000000-0000-4000-8000-000000000001', '"Acme IT Services"', '00000000-0000-4000-8000-000000000021'),
  ('00000000-0000-4000-8000-000000000142', '00000000-0000-4000-8000-000000000001', 'org',  'org.default_timezone',      '00000000-0000-4000-8000-000000000001', '"America/New_York"', '00000000-0000-4000-8000-000000000021'),
  ('00000000-0000-4000-8000-000000000143', '00000000-0000-4000-8000-000000000001', 'org',  'portal.enabled',            '00000000-0000-4000-8000-000000000001', 'true', '00000000-0000-4000-8000-000000000021'),
  ('00000000-0000-4000-8000-000000000144', '00000000-0000-4000-8000-000000000001', 'org',  'sla.default_calendar_hours','00000000-0000-4000-8000-000000000001', '8', '00000000-0000-4000-8000-000000000022'),
  ('00000000-0000-4000-8000-000000000145', '00000000-0000-4000-8000-000000000001', 'user', 'ui.theme',                  '00000000-0000-4000-8000-000000000021', '"dark"', '00000000-0000-4000-8000-000000000021'),
  ('00000000-0000-4000-8000-000000000146', '00000000-0000-4000-8000-000000000001', 'user', 'ui.density',                '00000000-0000-4000-8000-000000000023', '"comfortable"', '00000000-0000-4000-8000-000000000023');

-- ================================================= SLA calendars+policies
-- windows is the BusinessCalendar 7-slot [Mon..Sun] array of Option pairs.
INSERT INTO sla_calendars (id, org_id, name, timezone, windows, holidays, maintenance) VALUES
  ('00000000-0000-4000-8000-000000000151', '00000000-0000-4000-8000-000000000001', 'Business Hours ET', 'America/New_York',
   '[["09:00","17:00"],["09:00","17:00"],["09:00","17:00"],["09:00","17:00"],["09:00","17:00"],null,null]',
   '["2026-11-26","2026-11-27","2026-12-25"]',
   '[{"name":"ISP maintenance window","starts_at":"2026-09-12T06:00:00Z","ends_at":"2026-09-12T10:00:00Z"}]'),
  ('00000000-0000-4000-8000-000000000152', '00000000-0000-4000-8000-000000000001', 'Follow-the-Sun 24x7', 'UTC',
   '[["00:00","23:59"],["00:00","23:59"],["00:00","23:59"],["00:00","23:59"],["00:00","23:59"],["00:00","23:59"],["00:00","23:59"]]', '[]', '[]');

-- `calendar` is the embedded fallback BusinessCalendar document (name,
-- timezone, 7-slot windows, holidays, maintenance).
INSERT INTO sla_policies (id, org_id, name, is_default, targets, calendar, calendar_id) VALUES
  ('00000000-0000-4000-8000-000000000161', '00000000-0000-4000-8000-000000000001', 'Standard Response', true,
   '{"p1":{"first_response_minutes":15,"resolve_minutes":480},"p2":{"first_response_minutes":60,"resolve_minutes":1440},"p3":{"first_response_minutes":240,"resolve_minutes":4320},"p4":{"first_response_minutes":480,"resolve_minutes":10080}}',
   '{"name":"Standard Response fallback","timezone":"America/New_York","windows":[["09:00","17:00"],["09:00","17:00"],["09:00","17:00"],["09:00","17:00"],["09:00","17:00"],null,null],"holidays":[],"maintenance":[]}',
   '00000000-0000-4000-8000-000000000151'),
  ('00000000-0000-4000-8000-000000000162', '00000000-0000-4000-8000-000000000001', 'Critical Systems 24x7', false,
   '{"p1":{"first_response_minutes":10,"resolve_minutes":240},"p2":{"first_response_minutes":30,"resolve_minutes":480}}',
   '{"name":"Critical Systems fallback","timezone":"UTC","windows":[["00:00","23:59"],["00:00","23:59"],["00:00","23:59"],["00:00","23:59"],["00:00","23:59"],["00:00","23:59"],["00:00","23:59"]],"holidays":[],"maintenance":[]}',
   '00000000-0000-4000-8000-000000000152');

INSERT INTO sla_escalation_steps (id, org_id, policy_id, step_index, trigger_kind, at_pct, notify_user_ids, enabled) VALUES
  ('00000000-0000-4000-8000-000000000163', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000161', 0, 'risk',     75,  '["00000000-0000-4000-8000-000000000022"]', true),
  ('00000000-0000-4000-8000-000000000164', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000161', 1, 'breach',   100, '["00000000-0000-4000-8000-000000000021","00000000-0000-4000-8000-000000000022"]', true),
  ('00000000-0000-4000-8000-000000000165', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000161', 2, 'escalate', 150, '["00000000-0000-4000-8000-000000000021"]', true),
  ('00000000-0000-4000-8000-000000000166', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000162', 0, 'breach',   100, '["00000000-0000-4000-8000-000000000022"]', true);

-- ====================================================== reports (R3) =====
INSERT INTO report_definitions (id, org_id, name, description, kind, spec, viz, scope, created_by) VALUES
  ('00000000-0000-4000-8000-000000000171', '00000000-0000-4000-8000-000000000001', 'Tickets by priority', 'Open + closed ticket volume per priority', 'tickets',
   '{"filters":{"board_id":"00000000-0000-4000-8000-000000000081"},"group_by":"priority","measures":["count"]}', 'bar', 'org', '00000000-0000-4000-8000-000000000021'),
  ('00000000-0000-4000-8000-000000000172', '00000000-0000-4000-8000-000000000001', 'Billable hours per week', 'Billable minutes trend', 'time_entries',
   '{"filters":{"billable":true},"group_by":"week","measures":["minutes"]}', 'line', 'org', '00000000-0000-4000-8000-000000000021'),
  ('00000000-0000-4000-8000-000000000173', '00000000-0000-4000-8000-000000000001', 'SLA outcomes detail', 'Per-ticket SLA outcome table', 'sla_outcomes',
   '{"filters":{"breached":true},"group_by":"ticket","measures":["count"]}', 'table', 'self', '00000000-0000-4000-8000-000000000022');

INSERT INTO report_schedules (id, org_id, report_definition_id, name, recipients, cadence, time_of_day, day_of_week, timezone, window_days, enabled, next_run_at, created_by) VALUES
  ('00000000-0000-4000-8000-000000000174', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000171', 'Weekly ticket digest', '{00000000-0000-4000-8000-000000000021,00000000-0000-4000-8000-000000000022}', 'weekly', '08:00', 1, 'America/New_York', 7, true, '2026-09-07T12:00:00Z', '00000000-0000-4000-8000-000000000021');

INSERT INTO report_deliveries (id, org_id, schedule_id, slot_at, status, detail) VALUES
  ('00000000-0000-4000-8000-000000000175', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000174', '2026-08-31T12:00:00Z', 'delivered', '{"recipients":2,"format":"csv"}');

-- ========================================================== KB ===========
INSERT INTO kb_categories (id, org_id, name, slug, description, position, client_visible) VALUES
  ('00000000-0000-4000-8000-000000000181', '00000000-0000-4000-8000-000000000001', 'Onboarding', 'onboarding', 'New client setup guides', 0, true),
  ('00000000-0000-4000-8000-000000000182', '00000000-0000-4000-8000-000000000001', 'Runbooks',   'runbooks',   'Internal operational runbooks', 1, false);

INSERT INTO kb_articles (id, org_id, category_id, title, slug, body, status, client_visible, created_by, updated_by) VALUES
  ('00000000-0000-4000-8000-000000000183', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000181', 'Welcome to the portal', 'welcome-to-the-portal', '# Welcome\n\nHow to raise your first request.', 'published', true, '00000000-0000-4000-8000-000000000021', '00000000-0000-4000-8000-000000000021'),
  ('00000000-0000-4000-8000-000000000184', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000181', 'VPN setup guide', 'vpn-setup', '# VPN\n\nStep-by-step VPN onboarding.', 'published', true, '00000000-0000-4000-8000-000000000023', '00000000-0000-4000-8000-000000000023'),
  ('00000000-0000-4000-8000-000000000185', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000182', 'Firewall change runbook', 'firewall-change-runbook', '# Runbook\n\nChange control steps for NW-FW-01.', 'draft', false, '00000000-0000-4000-8000-000000000022', '00000000-0000-4000-8000-000000000022'),
  ('00000000-0000-4000-8000-000000000186', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000182', 'Old backup restore procedure', 'backup-restore-legacy', '# Legacy\n\nSuperseded by the new restore runbook.', 'archived', false, '00000000-0000-4000-8000-000000000021', '00000000-0000-4000-8000-000000000021');

-- ====================================================== documents ========
INSERT INTO documents (id, org_id, title, current_version, created_by, updated_by) VALUES
  ('00000000-0000-4000-8000-000000000191', '00000000-0000-4000-8000-000000000001', 'Network topology — Northwind HQ', 2, '00000000-0000-4000-8000-000000000022', '00000000-0000-4000-8000-000000000022'),
  ('00000000-0000-4000-8000-000000000192', '00000000-0000-4000-8000-000000000001', 'Escalation policy', 1, '00000000-0000-4000-8000-000000000021', '00000000-0000-4000-8000-000000000021');

INSERT INTO document_versions (id, org_id, document_id, version, title, body, change_summary, author_id) VALUES
  ('00000000-0000-4000-8000-000000000193', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000191', 1, 'Network topology — Northwind HQ', 'v1 body', 'Initial import', '00000000-0000-4000-8000-000000000022'),
  ('00000000-0000-4000-8000-000000000194', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000191', 2, 'Network topology — Northwind HQ', 'v2 body with new VLANs', 'Added VLAN 40 refresh', '00000000-0000-4000-8000-000000000022'),
  ('00000000-0000-4000-8000-000000000195', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000192', 1, 'Escalation policy', 'Escalation ladder document', 'Initial version', '00000000-0000-4000-8000-000000000021');

-- ====================================================== contracts ========
INSERT INTO contracts (id, org_id, client_id, number, name, contract_type, status, currency, effective_from, effective_until, billing_frequency, billing_anchor, amount_minor, hourly_rate_minor, included_minutes, overage_rate_minor, notes) VALUES
  ('00000000-0000-4000-8000-000000000201', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000051', 'CTR-2026-001', 'Northwind managed services', 'recurring', 'active', 'USD', '2026-01-01', '2026-12-31', 'monthly', '2026-02-01', 150000, NULL, 600, 15000, 'Auto-renews annually'),
  ('00000000-0000-4000-8000-000000000203', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000052', 'CTR-2026-002', 'Contoso retainer', 'retainer', 'active', 'USD', '2026-03-01', NULL, NULL, NULL, 120000, NULL, NULL, NULL, NULL);

INSERT INTO contract_rates (id, org_id, contract_id, work_type, rate_minor) VALUES
  ('00000000-0000-4000-8000-000000000204', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000201', 'remote', 15000),
  ('00000000-0000-4000-8000-000000000205', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000201', 'on_site', 19000);

INSERT INTO contract_fixed_fees (id, org_id, contract_id, label, amount_minor, due_on, notes) VALUES
  ('00000000-0000-4000-8000-000000000206', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000203', 'SOC rollout milestone 1', 250000, '2026-10-01', 'On phase completion');

-- ======================================================= projects ========
INSERT INTO projects (id, org_id, client_id, name, description, status, billing_method, budget_minor, currency, start_date, due_date) VALUES
  ('00000000-0000-4000-8000-000000000211', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000051', 'Northwind M365 migration', 'Tenant-to-tenant migration', 'active', 'time_and_materials', 800000, 'USD', '2026-07-01', '2026-11-30'),
  ('00000000-0000-4000-8000-000000000212', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000052', 'Contoso SOC rollout', 'Security operations rollout', 'planning', 'milestone', 1500000, 'USD', '2026-10-01', NULL);

INSERT INTO project_phases (id, org_id, project_id, name, description, sort_order, status, is_milestone, milestone_amount_minor, milestone_currency) VALUES
  ('00000000-0000-4000-8000-000000000213', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000211', 'Assessment',      'Discovery + licensing', 0, 'completed', false, NULL, NULL),
  ('00000000-0000-4000-8000-000000000214', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000211', 'Migration waves', 'Mailbox + file waves',  1, 'active',    false, NULL, NULL),
  ('00000000-0000-4000-8000-000000000215', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000212', 'SOC go-live',     'Cutover milestone',     0, 'pending',   true,  500000, 'USD');

INSERT INTO project_tasks (id, org_id, project_id, phase_id, title, description, status, assignee_id, sort_order, due_at, completed_at) VALUES
  ('00000000-0000-4000-8000-000000000216', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000211', '00000000-0000-4000-8000-000000000213', 'License inventory',      'Count + reconcile',     'completed',   '00000000-0000-4000-8000-000000000024', 0, '2026-07-15T00:00:00Z', '2026-07-12T00:00:00Z'),
  ('00000000-0000-4000-8000-000000000217', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000211', '00000000-0000-4000-8000-000000000214', 'Wave 1 — IT pilots',     '20 pilot mailboxes',    'in_progress', '00000000-0000-4000-8000-000000000023', 1, '2026-09-15T00:00:00Z', NULL),
  ('00000000-0000-4000-8000-000000000218', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000211', '00000000-0000-4000-8000-000000000214', 'Wave 2 — Sales org',     'Bulk move',             'pending',     '00000000-0000-4000-8000-000000000023', 2, '2026-10-15T00:00:00Z', NULL),
  ('00000000-0000-4000-8000-000000000219', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000212', NULL,                                    'Vendor selection',       'SOC platform RFP',      'blocked',     '00000000-0000-4000-8000-000000000021', 0, NULL, NULL);

INSERT INTO project_task_deps (org_id, task_id, depends_on_task_id) VALUES
  ('00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000218', '00000000-0000-4000-8000-000000000217');

INSERT INTO project_views (id, org_id, project_id, name, spec, is_default) VALUES
  ('00000000-0000-4000-8000-000000000220', '00000000-0000-4000-8000-000000000001', NULL, 'All active projects', '{"status":["active","planning"]}', true);

INSERT INTO project_templates (id, org_id, name, description, spec) VALUES
  ('00000000-0000-4000-8000-000000000221', '00000000-0000-4000-8000-000000000001', 'Client onboarding', 'Standard onboarding plan', '{"phases":[{"name":"Discovery","tasks":["Kickoff","Access audit"]},{"name":"Cutover","tasks":["Go/no-go","Cutover"]}]}');

-- ======================================================== tickets ========
INSERT INTO tickets (id, org_id, number, board_id, status_id, title, body, priority, client_id, site_id, contact_id, assignee_id, requester_id, category, due_at, resolved_at) VALUES
  ('00000000-0000-4000-8000-000000000301', '00000000-0000-4000-8000-000000000001', 'ACM-1001', '00000000-0000-4000-8000-000000000081', '00000000-0000-4000-8000-000000000092', 'Firewall flapping on NW-FW-01', 'Intermittent WAN drops since firmware update.', 'p1', '00000000-0000-4000-8000-000000000051', '00000000-0000-4000-8000-000000000054', '00000000-0000-4000-8000-000000000057', '00000000-0000-4000-8000-000000000023', '00000000-0000-4000-8000-000000000021', 'in_progress', '2026-09-02T18:00:00Z', NULL),
  ('00000000-0000-4000-8000-000000000302', '00000000-0000-4000-8000-000000000001', 'ACM-1002', '00000000-0000-4000-8000-000000000081', '00000000-0000-4000-8000-000000000094', 'Password reset — Marco Ruiz', NULL, 'p3', '00000000-0000-4000-8000-000000000052', '00000000-0000-4000-8000-000000000055', '00000000-0000-4000-8000-000000000058', '00000000-0000-4000-8000-000000000024', '00000000-0000-4000-8000-000000000021', 'closed', NULL, '2026-08-28T15:00:00Z'),
  ('00000000-0000-4000-8000-000000000303', '00000000-0000-4000-8000-000000000001', 'ACM-1003', '00000000-0000-4000-8000-000000000081', '00000000-0000-4000-8000-000000000093', 'Quote for laptop refresh', 'Awaiting client confirmation on spec.', 'p3', '00000000-0000-4000-8000-000000000051', '00000000-0000-4000-8000-000000000054', '00000000-0000-4000-8000-000000000057', NULL, '00000000-0000-4000-8000-000000000021', 'waiting', NULL, NULL),
  ('00000000-0000-4000-8000-000000000304', '00000000-0000-4000-8000-000000000001', 'ACM-1004', '00000000-0000-4000-8000-000000000081', '00000000-0000-4000-8000-000000000095', 'On-site: printer maintenance visit', 'Scheduled preventive maintenance.', 'p4', '00000000-0000-4000-8000-000000000051', '00000000-0000-4000-8000-000000000054', '00000000-0000-4000-8000-000000000057', '00000000-0000-4000-8000-000000000023', '00000000-0000-4000-8000-000000000021', 'waiting', NULL, NULL),
  ('00000000-0000-4000-8000-000000000305', '00000000-0000-4000-8000-000000000001', 'ACM-1005', '00000000-0000-4000-8000-000000000081', '00000000-0000-4000-8000-000000000091', 'New hire onboarding — 5 accounts', NULL, 'p2', '00000000-0000-4000-8000-000000000051', '00000000-0000-4000-8000-000000000054', '00000000-0000-4000-8000-000000000057', NULL, '00000000-0000-4000-8000-000000000021', 'open', NULL, NULL),
  ('00000000-0000-4000-8000-000000000306', '00000000-0000-4000-8000-000000000001', 'ACM-1006', '00000000-0000-4000-8000-000000000081', '00000000-0000-4000-8000-000000000094', 'VPN tunnel drops nightly', 'Ruled out ISP; monitored.', 'p2', '00000000-0000-4000-8000-000000000052', '00000000-0000-4000-8000-000000000055', '00000000-0000-4000-8000-000000000058', '00000000-0000-4000-8000-000000000024', '00000000-0000-4000-8000-000000000021', 'closed', NULL, '2026-08-20T22:00:00Z'),
  ('00000000-0000-4000-8000-000000000307', '00000000-0000-4000-8000-000000000001', 'ACM-1007', '00000000-0000-4000-8000-000000000082', '00000000-0000-4000-8000-000000000096', 'Migration intake — scope review', 'Intake from projects board.', 'p3', '00000000-0000-4000-8000-000000000051', NULL, NULL, '00000000-0000-4000-8000-000000000024', '00000000-0000-4000-8000-000000000021', 'open', NULL, NULL),
  ('00000000-0000-4000-8000-000000000308', '00000000-0000-4000-8000-000000000001', 'ACM-1008', '00000000-0000-4000-8000-000000000082', '00000000-0000-4000-8000-000000000098', 'SOC rollout — vendor on hold', 'Blocked on RFP responses.', 'p3', '00000000-0000-4000-8000-000000000052', NULL, NULL, '00000000-0000-4000-8000-000000000021', '00000000-0000-4000-8000-000000000021', 'waiting', NULL, NULL);

INSERT INTO ticket_comments (id, org_id, ticket_id, author_id, kind, body) VALUES
  ('00000000-0000-4000-8000-000000000311', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000301', '00000000-0000-4000-8000-000000000023', 'internal', 'Firmware rollback scheduled tonight.'),
  ('00000000-0000-4000-8000-000000000312', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000301', '00000000-0000-4000-8000-000000000021', 'client_visible', 'Acknowledged — please keep us posted.'),
  ('00000000-0000-4000-8000-000000000313', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000302', '00000000-0000-4000-8000-000000000024', 'in_house_update', 'Reset completed and verified.'),
  ('00000000-0000-4000-8000-000000000314', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000303', '00000000-0000-4000-8000-000000000021', 'internal', 'Following up with Lena on Thursday.');

INSERT INTO custom_field_values (field_id, entity_id, org_id, value) VALUES
  ('00000000-0000-4000-8000-000000000101', '00000000-0000-4000-8000-000000000301', '00000000-0000-4000-8000-000000000001', '"Production"'),
  ('00000000-0000-4000-8000-000000000102', '00000000-0000-4000-8000-000000000301', '00000000-0000-4000-8000-000000000001', '"Full branch office outage risk"'),
  ('00000000-0000-4000-8000-000000000103', '00000000-0000-4000-8000-000000000301', '00000000-0000-4000-8000-000000000001', '6.5'),
  ('00000000-0000-4000-8000-000000000101', '00000000-0000-4000-8000-000000000305', '00000000-0000-4000-8000-000000000001', '"Staging"'),
  ('00000000-0000-4000-8000-000000000107', '00000000-0000-4000-8000-000000000302', '00000000-0000-4000-8000-000000000001', '"CW-99120"'),
  ('00000000-0000-4000-8000-000000000104', '00000000-0000-4000-8000-000000000051', '00000000-0000-4000-8000-000000000001', '"Gold"'),
  ('00000000-0000-4000-8000-000000000105', '00000000-0000-4000-8000-000000000058', '00000000-0000-4000-8000-000000000001', '"Spanish"'),
  ('00000000-0000-4000-8000-000000000106', '00000000-0000-4000-8000-000000000059', '00000000-0000-4000-8000-000000000001', '"2027-03-31"');

INSERT INTO checklist_items (id, org_id, ticket_id, text, checked, sort_order, checked_by, checked_at, template_item_id) VALUES
  ('00000000-0000-4000-8000-000000000321', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000301', 'Root cause documented',        true,  0, '00000000-0000-4000-8000-000000000023', '2026-09-01T16:00:00Z', '00000000-0000-4000-8000-000000000112'),
  ('00000000-0000-4000-8000-000000000322', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000301', 'KB article linked or created', false, 1, NULL, NULL, '00000000-0000-4000-8000-000000000113'),
  ('00000000-0000-4000-8000-000000000323', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000301', 'Client notified',              false, 2, NULL, NULL, '00000000-0000-4000-8000-000000000114');

-- SLA state + a fired escalation on the P1
INSERT INTO ticket_sla_state (ticket_id, org_id, policy_id, first_response_due, resolution_due, first_responded_at, paused_business_secs, breached) VALUES
  ('00000000-0000-4000-8000-000000000301', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000162', '2026-09-01T09:30:00Z', '2026-09-02T09:30:00Z', '2026-09-01T09:05:00Z', 0, false),
  ('00000000-0000-4000-8000-000000000305', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000161', '2026-09-03T13:00:00Z', '2026-09-04T13:00:00Z', NULL, 1800, false);

INSERT INTO sla_fired_escalations (org_id, ticket_id, step_id, target, fired_at) VALUES
  ('00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000301', '00000000-0000-4000-8000-000000000166', 'first_response', '2026-09-01T09:29:00Z');

-- ================================================ time + timers ==========
INSERT INTO timer_sessions (id, org_id, user_id, ticket_id, started_at, stopped_at, segments) VALUES
  ('00000000-0000-4000-8000-000000000331', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000023', '00000000-0000-4000-8000-000000000301', '2026-09-01T13:00:00Z', '2026-09-01T16:00:00Z',
   '[{"started_at":"2026-09-01T13:00:00Z","ended_at":"2026-09-01T14:30:00Z"},{"started_at":"2026-09-01T14:45:00Z","ended_at":"2026-09-01T16:00:00Z"}]');

INSERT INTO time_entries (id, org_id, ticket_id, user_id, started_at, ended_at, minutes, billable, notes, work_type, timer_session_id) VALUES
  ('00000000-0000-4000-8000-000000000332', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000301', '00000000-0000-4000-8000-000000000023', '2026-09-01T13:00:00Z', '2026-09-01T16:00:00Z', 165, true, 'Firmware rollback + monitoring', 'remote', '00000000-0000-4000-8000-000000000331'),
  ('00000000-0000-4000-8000-000000000333', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000302', '00000000-0000-4000-8000-000000000024', '2026-08-28T14:30:00Z', '2026-08-28T15:00:00Z', 30, true, NULL, 'remote', NULL),
  ('00000000-0000-4000-8000-000000000334', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000306', '00000000-0000-4000-8000-000000000024', '2026-08-20T18:00:00Z', '2026-08-20T22:00:00Z', 240, true, 'Nightly tunnel watch', 'remote', NULL),
  ('00000000-0000-4000-8000-000000000335', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000303', '00000000-0000-4000-8000-000000000021', '2026-08-25T10:00:00Z', '2026-08-25T10:45:00Z', 45, false, 'Internal quote prep', NULL, NULL);

INSERT INTO time_gap_audits (id, org_id, user_id, day, activity, logged_minutes, expected_minutes, has_gap, nudge_sent_at) VALUES
  ('00000000-0000-4000-8000-000000000336', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000023', '2026-08-29', true, 300, 480, true, '2026-08-29T22:00:00Z');

-- ================================================ notifications ==========
INSERT INTO notification_org_defaults (org_id, kind, in_app, email, quiet_from, quiet_until, timezone) VALUES
  ('00000000-0000-4000-8000-000000000001', 'mention',    true, false, NULL, NULL, 'UTC'),
  ('00000000-0000-4000-8000-000000000001', 'sla_breach', true, true,  '22:00', '07:00', 'America/New_York');

INSERT INTO notification_preferences (user_id, kind, in_app, email, quiet_from, quiet_until, timezone) VALUES
  ('00000000-0000-4000-8000-000000000021', 'assignment', true, true, '21:00', '07:00', 'America/New_York'),
  ('00000000-0000-4000-8000-000000000023', 'mention',    true, false, NULL, NULL, 'America/Chicago');

INSERT INTO notifications (id, org_id, user_id, kind, ticket_id, title, body, read_at) VALUES
  ('00000000-0000-4000-8000-000000000341', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000022', 'sla_risk',   '00000000-0000-4000-8000-000000000301', 'P1 at risk', 'ACM-1001 consumed 75% of response budget', NULL),
  ('00000000-0000-4000-8000-000000000342', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000023', 'assignment', '00000000-0000-4000-8000-000000000301', 'Assigned to you', 'ACM-1001 auto-assigned by P1 pager rule', '2026-09-01T13:05:00Z');

-- ======================================================== email ==========
INSERT INTO email_inbound_settings (id, org_id, catchall_address, catchall_board_id, enabled) VALUES
  ('00000000-0000-4000-8000-000000000351', '00000000-0000-4000-8000-000000000001', 'support-8f2k@inbound.acme-it.example', '00000000-0000-4000-8000-000000000081', true);

INSERT INTO email_messages (id, org_id, direction, rfc_message_id, in_reply_to, email_refs, subject, from_address, from_name, to_addresses, body_text, ticket_id, comment_id, scan_status, transport) VALUES
  ('00000000-0000-4000-8000-000000000352', '00000000-0000-4000-8000-000000000001', 'inbound',  '<fw-update-1@northwind.example>', NULL, '{}', 'Re: Firewall flapping', 'lena.fischer@northwind.example', 'Lena Fischer', '{support-8f2k@inbound.acme-it.example}', 'Any update on the drops?', '00000000-0000-4000-8000-000000000301', '00000000-0000-4000-8000-000000000312', 'clean', 'smtp'),
  ('00000000-0000-4000-8000-000000000353', '00000000-0000-4000-8000-000000000001', 'outbound', '<marula-reply-001@acme-it.example>', '<fw-update-1@northwind.example>', '{<fw-update-1@northwind.example>}', 'Re: Firewall flapping', 'support@acme-it.example', 'Acme IT Support', '{lena.fischer@northwind.example}', 'Rollback scheduled tonight.', '00000000-0000-4000-8000-000000000301', '00000000-0000-4000-8000-000000000311', 'clean', 'smtp');

INSERT INTO email_outbound (id, org_id, email_message_id, status, attempts, max_attempts, next_attempt_at, provider_message_id) VALUES
  ('00000000-0000-4000-8000-000000000354', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000353', 'sent', 1, 8, '2026-09-01T13:30:00Z', 'relay-20260901-001');

-- ==================================================== invoicing ==========
INSERT INTO invoice_runs (id, org_id, number, period_from, period_to, invoice_count, total_minor, currency, due_in_days, created_by) VALUES
  ('00000000-0000-4000-8000-000000000361', '00000000-0000-4000-8000-000000000001', 'RUN-2026-08', '2026-08-01', '2026-08-31', 2, 165000, 'USD', 30, '00000000-0000-4000-8000-000000000021');

INSERT INTO invoices (id, org_id, run_id, client_id, contract_id, number, status, currency, period_from, period_to, issued_at, due_on, total_minor, total_paid_minor, dunning_state, dunning_updated_at, notes, created_by) VALUES
  ('00000000-0000-4000-8000-000000000362', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000361', '00000000-0000-4000-8000-000000000051', '00000000-0000-4000-8000-000000000201', 'ACM-INV-0001', 'issued', 'USD', '2026-08-01', '2026-08-31', '2026-09-01', '2026-10-01', 165000, 0, 'reminder_sent', '2026-09-02T09:00:00Z', NULL, '00000000-0000-4000-8000-000000000021'),
  ('00000000-0000-4000-8000-000000000363', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000361', '00000000-0000-4000-8000-000000000052', '00000000-0000-4000-8000-000000000203', 'ACM-INV-0002', 'paid',   'USD', '2026-08-01', '2026-08-31', '2026-09-01', '2026-10-01', 120000, 120000, 'none', NULL, 'Paid via ACH', '00000000-0000-4000-8000-000000000021');

INSERT INTO invoice_lines (id, org_id, invoice_id, contract_id, kind, description, period_start, period_end, quantity_minutes, unit_amount_minor, amount_minor, position) VALUES
  ('00000000-0000-4000-8000-000000000364', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000362', '00000000-0000-4000-8000-000000000201', 'recurring_base',  'Managed services — August', '2026-08-01', '2026-08-31', 0, 150000, 150000, 0),
  ('00000000-0000-4000-8000-000000000365', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000362', '00000000-0000-4000-8000-000000000201', 'time_materials',  'Remote support overage',    '2026-08-01', '2026-08-31', 165, 15000, 15000, 1),
  ('00000000-0000-4000-8000-000000000366', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000363', '00000000-0000-4000-8000-000000000203', 'retainer_base',   'Retainer — August',         '2026-08-01', '2026-08-31', 0, 120000, 120000, 0);

INSERT INTO invoice_time_entries (id, org_id, invoice_id, time_entry_id) VALUES
  ('00000000-0000-4000-8000-000000000367', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000362', '00000000-0000-4000-8000-000000000332');

INSERT INTO invoice_payments (id, org_id, invoice_id, amount_minor, currency, provider, provider_ref, notes, created_by) VALUES
  ('00000000-0000-4000-8000-000000000368', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000363', 120000, 'USD', 'none', 'ACH-20260901-7', 'Client ACH transfer', '00000000-0000-4000-8000-000000000021');

-- ======================================================= alerts ==========
INSERT INTO alert_sources (id, org_id, name, source_ref, secret_encrypted, board_id, client_id, correlation_window_secs, enabled, created_by) VALUES
  ('00000000-0000-4000-8000-000000000371', '00000000-0000-4000-8000-000000000001', 'NinjaOne production', 'ninjaone:prod-tenant', decode('616c657274736f7572636573656372657430313233343536373839616263','hex'), '00000000-0000-4000-8000-000000000081', '00000000-0000-4000-8000-000000000051', 900, true, '00000000-0000-4000-8000-000000000021');

INSERT INTO alerts (id, org_id, source_id, dedup_key, correlation_key, title, description, severity, occurred_at, attributes, incident_ticket_id) VALUES
  ('00000000-0000-4000-8000-000000000372', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000371', 'nw-fw-01-wan-20260901', 'device:nw-fw-01', 'WAN interface down', 'Interface eth0 lost link', 'critical', '2026-09-01T08:20:00Z', '{"device":"NW-FW-01","site":"HQ"}', '00000000-0000-4000-8000-000000000301'),
  ('00000000-0000-4000-8000-000000000373', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000371', 'nw-esx-02-diskwarn', 'device:nw-esx-02', 'Disk usage warning', 'Volume at 82%', 'warning', '2026-08-30T05:10:00Z', '{"device":"NW-ESX-02"}', NULL);

INSERT INTO alert_incidents (id, org_id, source_id, correlation_key, ticket_id, alert_count, first_alert_at, last_alert_at) VALUES
  ('00000000-0000-4000-8000-000000000374', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000371', 'device:nw-fw-01', '00000000-0000-4000-8000-000000000301', 3, '2026-09-01T08:20:00Z', '2026-09-01T08:50:00Z');

-- ======================================================= import ==========
INSERT INTO import_batches (id, org_id, label, source_note, entity_kind, conflict_policy, dry_run, rows_total, rows_created, rows_updated, rows_skipped, rows_failed, created_by, created_by_kind, started_at, finished_at, staged_chunks, queued_at) VALUES
  ('00000000-0000-4000-8000-000000000381', '00000000-0000-4000-8000-000000000001', 'Northwind client import', 'connectwise export', 'client', 'update', false, 42, 40, 2, 0, 0, '00000000-0000-4000-8000-000000000021', 'user', '2026-04-10T09:00:00Z', '2026-04-10T09:04:00Z', 3, '2026-04-10T09:01:00Z');

INSERT INTO external_ids (id, org_id, entity_kind, external_key, internal_id, last_seen_batch) VALUES
  ('00000000-0000-4000-8000-000000000382', '00000000-0000-4000-8000-000000000001', 'client', 'NW-001',   '00000000-0000-4000-8000-000000000051', '00000000-0000-4000-8000-000000000381'),
  ('00000000-0000-4000-8000-000000000383', '00000000-0000-4000-8000-000000000001', 'ticket', 'CW-99120', '00000000-0000-4000-8000-000000000302', NULL);

INSERT INTO import_mapping_templates (id, org_id, name, source_system, entity_kind, mapping, created_by, created_by_kind) VALUES
  ('00000000-0000-4000-8000-000000000384', '00000000-0000-4000-8000-000000000001', 'ConnectWise clients', 'connectwise', 'client', '{"name":"company_name","external_key":"company_id","notes":"note_text"}', '00000000-0000-4000-8000-000000000021', 'user');

-- ======================================================= portal ==========
INSERT INTO portal_users (id, org_id, client_id, email, display_name, password_hash, role, status) VALUES
  ('00000000-0000-4000-8000-000000000391', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000051', 'lena.fischer@northwind.example', 'Lena Fischer', '$argon2id$v=19$m=19456,t=2,p=1$fixtureseed$fixturehashplena', 'contact',    'active'),
  ('00000000-0000-4000-8000-000000000392', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000051', 'cfo@northwind.example',          'Nora Chen',    '$argon2id$v=19$m=19456,t=2,p=1$fixtureseed$fixturehashpnora',  'it_manager', 'active');

INSERT INTO portal_sessions (id, org_id, portal_user_id, refresh_token_hash, user_agent, expires_at) VALUES
  ('00000000-0000-4000-8000-000000000393', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000391', 'sha256$fixtureseed$portalsession01', 'Mozilla/5.0 fixture', '2026-09-30T00:00:00Z');

INSERT INTO board_portal_exposure (id, org_id, board_id, client_id, exposed, visible_status_ids) VALUES
  ('00000000-0000-4000-8000-000000000394', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000081', '00000000-0000-4000-8000-000000000051', true, '{00000000-0000-4000-8000-000000000091,00000000-0000-4000-8000-000000000093,00000000-0000-4000-8000-000000000094}');

INSERT INTO appointments (id, org_id, user_id, ticket_id, client_id, site_id, title, starts_at, ends_at, kind, status, travel_minutes, notes) VALUES
  ('00000000-0000-4000-8000-000000000395', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000023', '00000000-0000-4000-8000-000000000304', '00000000-0000-4000-8000-000000000051', '00000000-0000-4000-8000-000000000054', 'Printer maintenance visit', '2026-09-10T17:00:00Z', '2026-09-10T19:00:00Z', 'on_site', 'scheduled', 45, 'Parts in van stock');

INSERT INTO availability_windows (id, org_id, user_id, weekday, start_local, end_local, timezone) VALUES
  ('00000000-0000-4000-8000-000000000396', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000023', 1, '09:00', '17:00', 'America/Chicago'),
  ('00000000-0000-4000-8000-000000000397', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000023', 3, '09:00', '17:00', 'America/Chicago');

INSERT INTO on_call_blocks (id, org_id, user_id, starts_at, ends_at, category, notes) VALUES
  ('00000000-0000-4000-8000-000000000398', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000022', '2026-09-05T00:00:00Z', '2026-09-12T00:00:00Z', 'primary', 'Week 37 rotation');

INSERT INTO calendar_exclusions (id, org_id, user_id, starts_at, ends_at, reason) VALUES
  ('00000000-0000-4000-8000-000000000399', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000023', '2026-09-21T08:00:00Z', '2026-09-25T08:00:00Z', 'PTO');

INSERT INTO portal_appointment_requests (id, org_id, client_id, portal_user_id, user_id, title, starts_at, ends_at, kind, notes, status, decision_note, decided_by, decided_at, appointment_id) VALUES
  ('00000000-0000-4000-8000-000000000400', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000051', '00000000-0000-4000-8000-000000000391', '00000000-0000-4000-8000-000000000023', 'Quarterly review call', '2026-09-18T15:00:00Z', '2026-09-18T16:00:00Z', 'remote', 'Agenda to follow', 'pending', NULL, NULL, NULL, NULL),
  ('00000000-0000-4000-8000-000000000401', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000051', '00000000-0000-4000-8000-000000000391', '00000000-0000-4000-8000-000000000023', 'Firewall walkthrough', '2026-09-09T16:00:00Z', '2026-09-09T17:00:00Z', 'remote', NULL, 'approved', 'Approved — invites sent', '00000000-0000-4000-8000-000000000021', '2026-09-01T14:00:00Z', NULL);

-- =================================================== dispatch (0045) =====
INSERT INTO dispatch_queue_entries (id, org_id, ticket_id, rank, note, created_by) VALUES
  ('00000000-0000-4000-8000-000000000411', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000304', 0, 'Morning run — bring toner', '00000000-0000-4000-8000-000000000021');

INSERT INTO dispatch_status_updates (id, org_id, appointment_id, status, note, actor_id, source) VALUES
  ('00000000-0000-4000-8000-000000000412', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000395', 'en_route', 'On the way', '00000000-0000-4000-8000-000000000023', 'connector');

-- ============================================ agent platform (0040) ======
INSERT INTO mcp_capability_grants (id, org_id, capability, mode, granted_by) VALUES
  ('00000000-0000-4000-8000-000000000421', '00000000-0000-4000-8000-000000000001', 'tickets:read',  'proposed',   '00000000-0000-4000-8000-000000000021'),
  ('00000000-0000-4000-8000-000000000422', '00000000-0000-4000-8000-000000000001', 'tickets:write', 'autonomous', '00000000-0000-4000-8000-000000000021');

INSERT INTO agent_skills (id, org_id, slug, version, manifest, created_by) VALUES
  ('00000000-0000-4000-8000-000000000423', '00000000-0000-4000-8000-000000000001', 'ticket-triage', 1, '{"name":"ticket-triage","api_version":1,"description":"Suggest triage for inbound tickets","tools":["tickets:read","tickets:write"]}', '00000000-0000-4000-8000-000000000021');

INSERT INTO agent_proposals (id, org_id, entity, entity_id, op, before_value, proposed_value, classification, capability, dial_mode, base_version, status, agent_kind, agent_ref, skill_slug, skill_version, rule_set_hash, session_id, tool, for_user, rationale) VALUES
  ('00000000-0000-4000-8000-000000000424', '00000000-0000-4000-8000-000000000001', 'ticket', '00000000-0000-4000-8000-000000000305', 'update', '{"priority":"p3"}', '{"priority":"p2"}', 'internal', 'tickets:write', 'proposed', 3, 'proposed', 'agent', 'triage-bot@marula', 'ticket-triage', 1, 'rules-2026-08', 'sess-fixture-001', 'set_priority', '00000000-0000-4000-8000-000000000021', '5 accounts affected, onboarding deadline');

INSERT INTO user_skills (id, org_id, user_id, skill, expires_at) VALUES
  ('00000000-0000-4000-8000-000000000425', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000024', 'networking', '2027-06-30T00:00:00Z');

-- =================================================== provenance (0043) ===
INSERT INTO entity_write_seqs (org_id, entity, entity_id, seq) VALUES
  ('00000000-0000-4000-8000-000000000001', 'ticket', '00000000-0000-4000-8000-000000000301', 3);

INSERT INTO entity_writes (id, org_id, entity, entity_id, seq, op, actor_id, actor_kind, before, after, session_id, skill, ruleset_hash, is_undo) VALUES
  ('00000000-0000-4000-8000-000000000431', '00000000-0000-4000-8000-000000000001', 'ticket', '00000000-0000-4000-8000-000000000301', 1, 'update', '00000000-0000-4000-8000-000000000023', 'human', '{"status":"open"}', '{"status":"in_progress"}', NULL, NULL, NULL, false),
  ('00000000-0000-4000-8000-000000000432', '00000000-0000-4000-8000-000000000001', 'ticket', '00000000-0000-4000-8000-000000000301', 2, 'update', '00000000-0000-4000-8000-000000000022', 'agent',  '{"priority":"p2"}', '{"priority":"p1"}', '00000000-0000-4000-8000-000000000434', 'ticket-triage', 'rules-2026-08', false);

-- provenance_events is deliberately NOT seeded by hand: the
-- `provenance_project` trigger (0043) projects every entity_writes row into
-- field-level provenance exactly as production does, so the fixture carries
-- trigger-generated rows for write 431 (human) and 432 (agent).

INSERT INTO idempotency_keys (id, org_id, key, entity, entity_id, request_hash, response, actor_id) VALUES
  ('00000000-0000-4000-8000-000000000433', '00000000-0000-4000-8000-000000000001', 'idem-fixture-0001', 'ticket', '00000000-0000-4000-8000-000000000305', 'sha256$fixtureseed$req01', '{"id":"00000000-0000-4000-8000-000000000305"}', '00000000-0000-4000-8000-000000000021');

-- =============================================== audit + outbox ==========
INSERT INTO audit_events (id, org_id, actor_kind, actor_id, action, entity_type, entity_id, detail, ip, occurred_at) VALUES
  ('00000000-0000-4000-8000-000000000441', '00000000-0000-4000-8000-000000000001', 'human',   '00000000-0000-4000-8000-000000000021', 'auth.login',        'user', '00000000-0000-4000-8000-000000000021', '{"mfa":false}', '203.0.113.10', '2026-09-01T12:55:00Z'),
  ('00000000-0000-4000-8000-000000000442', '00000000-0000-4000-8000-000000000001', 'agent',   'triage-bot@marula',                  'ticket.update',     'ticket', '00000000-0000-4000-8000-000000000301', '{"field":"priority"}', NULL, '2026-09-01T13:02:00Z'),
  ('00000000-0000-4000-8000-000000000443', '00000000-0000-4000-8000-000000000001', 'import',  '00000000-0000-4000-8000-000000000381', 'import.batch',      'import_batch', '00000000-0000-4000-8000-000000000381', '{"rows":42}', NULL, '2026-04-10T09:04:00Z'),
  ('00000000-0000-4000-8000-000000000444', '00000000-0000-4000-8000-000000000001', 'system',  NULL,                                   'sla.escalation',    'ticket', '00000000-0000-4000-8000-000000000301', '{"step":0,"target":"first_response"}', NULL, '2026-09-01T09:29:00Z'),
  ('00000000-0000-4000-8000-000000000445', '00000000-0000-4000-8000-000000000001', 'portal_contact', '00000000-0000-4000-8000-000000000391', 'portal.login', 'portal_user', '00000000-0000-4000-8000-000000000391', '{}', '198.51.100.7', '2026-09-02T08:00:00Z');

INSERT INTO outbox_events (id, org_id, event_type, entity, entity_id, payload, published_at) VALUES
  ('00000000-0000-4000-8000-000000000451', '00000000-0000-4000-8000-000000000001', 'ticket.created',        'ticket', '00000000-0000-4000-8000-000000000301', '{"ticket":{"number":"ACM-1001","title":"Firewall flapping on NW-FW-01","priority":"p1"}}', '2026-09-01T09:00:10Z'),
  ('00000000-0000-4000-8000-000000000452', '00000000-0000-4000-8000-000000000001', 'ticket.status_changed', 'ticket', '00000000-0000-4000-8000-000000000301', '{"from":"open","to":"in_progress"}', NULL),
  ('00000000-0000-4000-8000-000000000453', '00000000-0000-4000-8000-000000000001', 'invoice.issued',        'invoice', '00000000-0000-4000-8000-000000000362', '{"number":"ACM-INV-0001","total_minor":165000}', '2026-09-01T09:00:20Z');

-- ========================================= connectors + BYOK + jobs ======
INSERT INTO connector_installations (id, org_id, connector_type, config_encrypted, capability_scopes, consented_by) VALUES
  ('00000000-0000-4000-8000-000000000461', '00000000-0000-4000-8000-000000000001', 'qbo-export-sync', decode('776f726b666c6f77736563726574313233343536373839616263646566','hex'), '{billing:export,clients:read}', '00000000-0000-4000-8000-000000000021'),
  ('00000000-0000-4000-8000-000000000462', '00000000-0000-4000-8000-000000000001', 'immybot',         decode('696d6d79626f74','hex'), '{alerts:ingest}', '00000000-0000-4000-8000-000000000021');

INSERT INTO byok_secrets (id, org_id, provider, secret_encrypted, created_by) VALUES
  ('00000000-0000-4000-8000-000000000463', '00000000-0000-4000-8000-000000000001', 'openai', decode('62796f6b736563726574666978747572653031323334353637383930','hex'), '00000000-0000-4000-8000-000000000021');

INSERT INTO jobs (id, org_id, kind, payload, status, run_after, attempts, max_attempts) VALUES
  ('00000000-0000-4000-8000-000000000464', '00000000-0000-4000-8000-000000000001', 'webhook.deliver', '{"webhook_id":"00000000-0000-4000-8000-000000000131","event_id":"00000000-0000-4000-8000-000000000452"}', 'pending', '2026-09-03T03:00:00Z', 0, 5);

INSERT INTO webhook_deliveries (id, org_id, webhook_id, event_id, event_type, attempt, response_status, delivered_at, last_error) VALUES
  ('00000000-0000-4000-8000-000000000465', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000131', '00000000-0000-4000-8000-000000000451', 'ticket.created', 1, 200, '2026-09-01T09:00:12Z', NULL),
  ('00000000-0000-4000-8000-000000000466', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000131', '00000000-0000-4000-8000-000000000452', 'ticket.status_changed', 2, NULL, NULL, 'connect timeout');

-- =================================================== search index ========
INSERT INTO search_index (id, org_id, entity_kind, entity_id, parent_id, title, body, indexed_at) VALUES
  ('00000000-0000-4000-8000-000000000471', '00000000-0000-4000-8000-000000000001', 'ticket',     '00000000-0000-4000-8000-000000000301', NULL, 'Firewall flapping on NW-FW-01', 'Intermittent WAN drops since firmware update.', '2026-09-01T09:00:00Z'),
  ('00000000-0000-4000-8000-000000000472', '00000000-0000-4000-8000-000000000001', 'kb_article', '00000000-0000-4000-8000-000000000183', NULL, 'Welcome to the portal', 'How to raise your first request.', '2026-09-01T09:00:00Z'),
  ('00000000-0000-4000-8000-000000000473', '00000000-0000-4000-8000-000000000001', 'client',     '00000000-0000-4000-8000-000000000051', NULL, 'Northwind Traders', 'Flagship MSP account', '2026-09-01T09:00:00Z');

-- =================================================== attachments =========
INSERT INTO attachments (id, org_id, kind, ticket_id, filename, content_type, size_bytes, sha256, storage_key, uploaded_by_user_id) VALUES
  ('00000000-0000-4000-8000-000000000481', '00000000-0000-4000-8000-000000000001', 'ticket', '00000000-0000-4000-8000-000000000301', 'wan-graph.png', 'image/png', 48213, 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855', 'fixtures/v0.3.0/wan-graph.png', '00000000-0000-4000-8000-000000000023');

-- ================================================= travel draft (0045) ===
INSERT INTO travel_entry_drafts (id, org_id, user_id, ticket_id, started_at, ended_at, distance, distance_unit, vehicle, billable, notes, status, created_by) VALUES
  ('00000000-0000-4000-8000-000000000491', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000023', '00000000-0000-4000-8000-000000000304', '2026-09-10T16:15:00Z', '2026-09-10T17:00:00Z', 22.5, 'mi', 'Van 2', true, 'Plant visit outbound leg', 'draft', '00000000-0000-4000-8000-000000000023');

-- ============================================ plugin runtime (0156/0154) ==
-- One curated catalog entry, one enabled install pinned to its module digest,
-- the two surfaces the manifest declares, a clean health row, and KV data with
-- the usage counter the write path maintains. A v0.3.0 deployment that runs a
-- plugin looks like this, so the next release's migrations are proven against it.
INSERT INTO plugin_catalog (id, slug, name, description, publisher, latest_version, interface_version, origin_org_id) VALUES
  ('00000000-0000-4000-8000-000000000901', 'warranty-watch', 'Warranty Watch',
   'Shows what is still under warranty on a ticket, and writes the vendor note.',
   'Marula', '1.2.0', 1, NULL);

INSERT INTO plugin_versions (id, catalog_id, version, manifest, module_digest, manifest_authenticated) VALUES
  ('00000000-0000-4000-8000-000000000902', '00000000-0000-4000-8000-000000000901', '1.2.0',
   '{"name":"Warranty Watch","version":"1.2.0","interface_version":1,"description":"Shows what is still under warranty on a ticket, and writes the vendor note.","capabilities":["ui.widgets","automation.actions"],"surface_profile":{"widgets":[{"id":"warranty_card","title":"Warranty","slot":"ticket_pane","kind":"list"}],"actions":[{"id":"log_warranty_note","label":"Log warranty note","effect":"add_internal_comment"}]}}'::jsonb,
   repeat('a1b2c3d4', 8), true);

INSERT INTO plugin_installs (id, org_id, catalog_id, version, status, granted_capabilities, installed_by, installed_at, updated_at, module_digest) VALUES
  ('00000000-0000-4000-8000-000000000903', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000901', '1.2.0', 'enabled',
   '["ui.widgets","automation.actions"]'::jsonb, '00000000-0000-4000-8000-000000000021',
   '2026-09-20T10:00:00Z', '2026-09-20T10:00:00Z', repeat('a1b2c3d4', 8));

INSERT INTO plugin_surface_entries (id, org_id, install_id, surface, entry_key, label, definition, enabled, output_schema, idempotent, emits, audience, agent_description) VALUES
  ('00000000-0000-4000-8000-000000000904', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000903', 'widget',
   'plugin.warranty-watch.warranty_card', 'Warranty',
   '{"kind":"list","slot":"ticket_pane","props":{"title":"Warranty","source":"asset.by_client"}}'::jsonb,
   true, NULL, true, '{}', 'human', NULL),
  ('00000000-0000-4000-8000-000000000905', '00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000903', 'automation_action',
   'plugin.warranty-watch.log_warranty_note', 'Log warranty note',
   '{"effect":"add_internal_comment","params_schema":{"type":"object","properties":{"note":{"type":"string"}},"required":["note"]}}'::jsonb,
   true, NULL, false, '{}', 'both',
   'Writes an internal note carrying the vendor warranty answer for this ticket.');

INSERT INTO plugin_install_health (install_id, org_id, consecutive_failures) VALUES
  ('00000000-0000-4000-8000-000000000903', '00000000-0000-4000-8000-000000000001', 0);

INSERT INTO plugin_kv_entries (org_id, install_id, namespace, key, value, expires_at) VALUES
  ('00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000903', 'cache', 'last-seen',
   decode('323032362d30392d32305431303a30303a30305a', 'hex'), '2026-12-31T00:00:00Z'),
  ('00000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000903', 'docs', 'warranty-terms',
   decode('34382d6d6f6e746873', 'hex'), NULL);

INSERT INTO plugin_kv_usage (install_id, org_id, bytes, keys) VALUES
  ('00000000-0000-4000-8000-000000000903', '00000000-0000-4000-8000-000000000001', 29, 2);

COMMIT;
