-- Marula rebrand string-data (Wave 41 lane 1, `p41-marula-rebrand`; migration
-- 0114 is this lane's exclusive window; head was 0113).
--
-- Purpose: carry the Marula name into DATA and live schema defaults that
-- migrations 0001–0113 shipped under the previous name. Old migration files
-- are history and stay byte-identical — every correction lands HERE, once.
--
-- In scope (string data + live defaults only — no table or column renames,
-- no relationship changes):
-- 1. `marula_events` NOTIFY channel: migration 0068 created the outbox
--    trigger against `meridian_events`; core-api now listens on
--    `marula_events`. Drop and recreate the function + trigger so the
--    realtime fan-out speaks the current channel name.
-- 2. App role `marula_app`: migration 0001 provisioned `meridian_app` and
--    later migrations granted it EXECUTE/SELECT. The API now connects as
--    `marula_app`. Provision the new role and give it the SAME privilege
--    surface the history gave the old one, by NAME over pg_proc (signature-
--    exact; the history's supersede chain keeps the newest signature live).
-- 3. Ticket-prefix default 'MER' → 'MAR' on live schema (orgs.ticket_prefix
--    DEFAULT, shipped by 0001; rendered in-product as MER-2841-style ids).
-- 4. Seeded rows that render the previous name: nothing ships in migrations
--    0001–0113 (verified — the demo org and template bodies are inserted
--    from code paths that already say Marula). Recorded as the no-op it is.

-- 1. The outbox NOTIFY channel --------------------------------------------
DROP TRIGGER IF EXISTS trg_outbox_notify_meridian_events ON outbox_events;
DROP FUNCTION IF EXISTS outbox_notify_meridian_events();

-- Exact body of migration 0068's trigger with only the channel name moved
-- (channels are string data on the wire); SECURITY DEFINER preserved.
CREATE FUNCTION outbox_notify_marula_events() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER AS $$
DECLARE
  v_entity_id uuid;
BEGIN
  -- outbox_events.entity_id is text; only uuid-keyed entities have a
  -- ledger counter to join against. A non-uuid id degrades to seq: null.
  IF NEW.entity_id IS NOT NULL THEN
    BEGIN
      v_entity_id := NEW.entity_id::uuid;
    EXCEPTION WHEN invalid_text_representation THEN
      v_entity_id := NULL;
    END;
  END IF;

  PERFORM pg_notify(
    'marula_events',
    json_build_object(
      'id',        NEW.id,
      'org_id',    NEW.org_id,
      'type',      NEW.event_type,
      'entity',    NEW.entity,
      'entity_id', NEW.entity_id,
      'seq',       (SELECT seq FROM entity_write_seqs
                     WHERE org_id = NEW.org_id
                       AND entity = NEW.entity
                       AND entity_id = v_entity_id),
      'at',        NEW.occurred_at
    )::text
  );
  RETURN NEW;
END;
$$;

CREATE TRIGGER trg_outbox_notify_marula_events
AFTER INSERT ON outbox_events
FOR EACH ROW EXECUTE FUNCTION outbox_notify_marula_events();

-- 2. The application role ---------------------------------------------------
DO $$
BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'marula_app') THEN
    EXECUTE 'CREATE ROLE marula_app LOGIN PASSWORD ''marula_app''';
  END IF;
END
$$;

-- Table/sequence/schema surface (0001 + later familias).
GRANT USAGE ON SCHEMA public TO marula_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO marula_app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO marula_app;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO marula_app;

-- EXECUTE over the app-resolver functions, by NAME with their LIVE
-- signatures (the last CREATE OR REPLACE wins in pg_proc), exactly the
-- surface 0010/0013/0014/0020/0021/0037/0046/0047/0055/0064/0072/0096
-- granted to the previous role name.
DO $fn$
DECLARE
  fn_name text;
BEGIN
  FOREACH fn_name IN ARRAY ARRAY[
    'app_resolve_login',
    'app_resolve_session',
    'app_resolve_refresh',
    'app_list_enabled_oidc_providers',
    'app_resolve_oidc_provider',
    'app_resolve_sso_code',
    'app_resolve_user_invite',
    'app_resolve_edge_source',
    'app_create_session',
    'app_create_portal_session',
    'resolve_inbound_email_route',
    'claim_due_email_outbound',
    'apply_email_send_result',
    'api_key_auth_lookup',
    'api_key_touch_last_used',
    'portal_resolve_login',
    'portal_resolve_session',
    'scheduling_link_resolve',
    'scheduling_link_mark_viewed',
    'scheduling_link_mark_scheduled',
    'alert_source_auth_lookup',
    'resolve_secret_share_link',
    'app_auth_gate_check',
    'app_auth_gate_fail',
    'app_auth_gate_clear',
    'app_resolve_auth_email',
    'app_consume_one_use_token'
  ] LOOP
    EXECUTE format(
      'GRANT EXECUTE ON FUNCTION %s TO marula_app;',
      (SELECT p.oid::regprocedure
         FROM pg_proc p
         JOIN pg_namespace n ON n.oid = p.pronamespace
        WHERE n.nspname = 'public'
          AND p.proname = fn_name
        ORDER BY p.oid DESC
        LIMIT 1)
    );
  END LOOP;
END
$fn$;

-- ops read-model surface (0026/0041/0059/0081/0091).
GRANT USAGE ON SCHEMA ops TO marula_app;
GRANT SELECT ON ALL TABLES IN SCHEMA ops TO marula_app;
ALTER DEFAULT PRIVILEGES IN SCHEMA ops GRANT SELECT ON TABLES TO marula_app;

-- 3. Ticket-prefix default --------------------------------------------------
ALTER TABLE orgs ALTER COLUMN ticket_prefix SET DEFAULT 'MAR';

-- 4. Verified no-op: migrations 0001–0113 seed no org/template/notification
-- rows carrying the previous brand name (the demo org and template bodies
-- are inserted from code, which ships Marula). Nothing more to update here.
